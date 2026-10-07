//! Prompts held back while a thread's turn runs. Each completed turn sends one,
//! with the model and modes it was queued with. In memory only.
use std::collections::HashMap;

use serde_json::Value;

use crate::commands;

#[derive(Clone, Debug, PartialEq)]
pub struct QueuedMessage {
    /// Assigned by `Queues::push`.
    pub id: u64,
    pub text: String,
    pub attachments: Vec<Value>,
    pub model_selection: Value,
    pub runtime_mode: String,
    pub interaction_mode: String,
}

impl QueuedMessage {
    /// The `thread.turn.start` this message sends.
    pub fn start_command(&self, thread_id: &str) -> Result<Value, &'static str> {
        commands::start_turn(
            thread_id,
            &self.text,
            &self.attachments,
            self.model_selection.clone(),
            &self.runtime_mode,
            &self.interaction_mode,
        )
    }
}

#[derive(Default)]
struct ThreadQueue {
    messages: Vec<QueuedMessage>,
    /// The head is on the wire.
    dispatching: bool,
    /// A send was accepted but its turn has not been seen running, so an idle
    /// session still describes the turn before it.
    awaiting_turn: bool,
    /// The last send was rejected; the head waits for the next completed turn.
    held: bool,
}

#[derive(Default)]
pub struct Queues {
    threads: HashMap<String, ThreadQueue>,
    next_id: u64,
}

impl Queues {
    /// Appends `message`, or puts it ahead of everything not already on the
    /// wire when `front` is set. Returns its id.
    pub fn push(&mut self, thread_id: &str, mut message: QueuedMessage, front: bool) -> u64 {
        self.next_id += 1;
        message.id = self.next_id;
        let queue = self.threads.entry(thread_id.to_owned()).or_default();
        if front {
            queue
                .messages
                .insert(usize::from(queue.dispatching), message);
        } else {
            queue.messages.push(message);
        }
        self.next_id
    }

    pub fn messages(&self, thread_id: &str) -> &[QueuedMessage] {
        self.threads
            .get(thread_id)
            .map_or(&[], |queue| queue.messages.as_slice())
    }

    pub fn has_queued(&self, thread_id: &str) -> bool {
        self.threads.contains_key(thread_id)
    }

    /// True while the head's send awaits its receipt.
    pub fn dispatching(&self, thread_id: &str) -> bool {
        self.threads
            .get(thread_id)
            .is_some_and(|queue| queue.dispatching)
    }

    pub fn thread_ids(&self) -> Vec<String> {
        self.threads.keys().cloned().collect()
    }

    /// Takes a message back out. A message already on the wire stays.
    pub fn remove(&mut self, thread_id: &str, id: u64) -> Option<QueuedMessage> {
        let queue = self.threads.get_mut(thread_id)?;
        let index = queue.messages.iter().position(|message| message.id == id)?;
        if index == 0 && queue.dispatching {
            return None;
        }
        let message = queue.messages.remove(index);
        self.forget_if_empty(thread_id);
        Some(message)
    }

    /// Records the thread's session state. Returns the message to send now:
    /// the head, once the turn before it has completed.
    pub fn observe(&mut self, thread_id: &str, running: bool) -> Option<QueuedMessage> {
        let queue = self.threads.get_mut(thread_id)?;
        if running {
            queue.awaiting_turn = false;
            queue.held = false;
            return None;
        }
        if queue.dispatching || queue.awaiting_turn || queue.held {
            return None;
        }
        queue.dispatching = true;
        queue.messages.first().cloned()
    }

    /// Sends `id` now, whatever the session state, unless another send is on
    /// the wire. Used by steering and Send now.
    pub fn take(&mut self, thread_id: &str, id: u64) -> Option<QueuedMessage> {
        let queue = self.threads.get_mut(thread_id)?;
        if queue.dispatching {
            return None;
        }
        let index = queue.messages.iter().position(|message| message.id == id)?;
        let message = queue.messages.remove(index);
        queue.messages.insert(0, message.clone());
        queue.dispatching = true;
        queue.held = false;
        Some(message)
    }

    /// The receipt for the head's send. Accepted drops it; rejected keeps it
    /// at the head until the next completed turn.
    pub fn settle(&mut self, thread_id: &str, id: u64, accepted: bool) {
        let Some(queue) = self.threads.get_mut(thread_id) else {
            return;
        };
        queue.dispatching = false;
        if accepted {
            queue.messages.retain(|message| message.id != id);
            queue.awaiting_turn = true;
        } else {
            queue.held = true;
        }
        self.forget_if_empty(thread_id);
    }

    fn forget_if_empty(&mut self, thread_id: &str) {
        if self
            .threads
            .get(thread_id)
            .is_some_and(|queue| queue.messages.is_empty())
        {
            self.threads.remove(thread_id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn message(text: &str) -> QueuedMessage {
        QueuedMessage {
            id: 0,
            text: text.into(),
            attachments: vec![],
            model_selection: json!({"instanceId":"codex","model":"gpt-5.4"}),
            runtime_mode: "full-access".into(),
            interaction_mode: "default".into(),
        }
    }

    fn texts(queues: &Queues, thread_id: &str) -> Vec<String> {
        queues
            .messages(thread_id)
            .iter()
            .map(|message| message.text.clone())
            .collect()
    }

    #[test]
    fn messages_leave_in_order_one_per_completed_turn() {
        let mut queues = Queues::default();
        let first = queues.push("t", message("first"), false);
        let second = queues.push("t", message("second"), false);
        assert_eq!(queues.observe("t", true), None);
        let sent = queues.observe("t", false).unwrap();
        assert_eq!((sent.id, sent.text.as_str()), (first, "first"));
        // Updates while the send is on the wire, or before its turn shows up, send nothing.
        assert_eq!(queues.observe("t", false), None);
        queues.settle("t", first, true);
        assert_eq!(queues.observe("t", false), None);
        assert_eq!(queues.observe("t", true), None);
        assert_eq!(queues.observe("t", false).unwrap().id, second);
        queues.settle("t", second, true);
        assert!(!queues.has_queued("t"));
    }

    #[test]
    fn a_rejected_send_stays_at_the_head_until_the_next_completion() {
        let mut queues = Queues::default();
        let first = queues.push("t", message("first"), false);
        queues.push("t", message("second"), false);
        queues.observe("t", false).unwrap();
        queues.settle("t", first, false);
        assert_eq!(texts(&queues, "t"), ["first", "second"]);
        assert_eq!(queues.observe("t", false), None);
        queues.observe("t", true);
        assert_eq!(queues.observe("t", false).unwrap().id, first);
    }

    #[test]
    fn removal_skips_a_message_but_never_one_on_the_wire() {
        let mut queues = Queues::default();
        let first = queues.push("t", message("first"), false);
        let second = queues.push("t", message("second"), false);
        assert_eq!(queues.remove("t", second).unwrap().text, "second");
        queues.observe("t", false).unwrap();
        assert_eq!(queues.remove("t", first), None);
        queues.settle("t", first, true);
        assert!(!queues.has_queued("t"));
        assert_eq!(queues.remove("t", first), None);
    }

    #[test]
    fn steering_goes_ahead_of_the_queue_while_the_turn_runs() {
        let mut queues = Queues::default();
        queues.push("t", message("queued"), false);
        let steer = queues.push("t", message("steer"), true);
        assert_eq!(texts(&queues, "t"), ["steer", "queued"]);
        queues.observe("t", true);
        assert_eq!(queues.take("t", steer).unwrap().text, "steer");
        assert_eq!(queues.take("t", steer), None);
        assert_eq!(queues.observe("t", false), None);
    }

    #[test]
    fn threads_queue_and_complete_independently() {
        let mut queues = Queues::default();
        queues.push("a", message("for a"), false);
        queues.push("b", message("for b"), false);
        assert_eq!(queues.observe("c", false), None);
        assert_eq!(queues.observe("b", false).unwrap().text, "for b");
        assert_eq!(texts(&queues, "a"), ["for a"]);
        assert!(!queues.dispatching("a"));
        assert_eq!(queues.observe("a", false).unwrap().text, "for a");
    }
}
