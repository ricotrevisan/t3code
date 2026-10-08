//! Prompts held for a tool boundary or turn end, with captured model and modes.
//! Rejected sends remain available for explicit retry. In memory only.
use std::collections::{HashMap, HashSet};

use serde_json::Value;

use crate::commands;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FollowUpBehavior {
    #[default]
    Queue,
    Steer,
}

/// Projection identity, not elapsed time, acknowledges an accepted dispatch.
#[derive(Clone, Debug, Default)]
pub struct Observation {
    pub running: bool,
    pub connecting: bool,
    pub blocked: bool,
    pub identity: String,
    /// Extra receipt markers available only with selected-thread detail.
    pub detail_identity: Option<String>,
    pub tool: Option<String>,
    /// False means activities were not observed, not that no tool completed.
    pub tool_available: bool,
}

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
        if self.text.trim().is_empty() && !self.attachments.is_empty() {
            let mut command = commands::command("thread.turn.start", thread_id);
            command["message"] = serde_json::json!({"messageId":commands::id(),"role":"user","text":self.text,"attachments":self.attachments});
            command["modelSelection"] = self.model_selection.clone();
            command["runtimeMode"] = serde_json::json!(self.runtime_mode);
            command["interactionMode"] = serde_json::json!(self.interaction_mode);
            command["createdAt"] = serde_json::json!(commands::now());
            return Ok(command);
        }
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
    baseline: String,
    observed: String,
    baseline_detail: Option<String>,
    observed_detail: Option<String>,
    observed_tool: Option<String>,
    tools: HashMap<u64, Option<String>>,
    source_plans: HashMap<u64, Value>,
    contexts: HashMap<u64, Value>,
    consumed_tools: HashSet<String>,
    /// Rejected messages require explicit retry or removal.
    held: HashSet<u64>,
}

#[derive(Default)]
pub struct Queues {
    threads: HashMap<String, ThreadQueue>,
    next_id: u64,
}

impl Queues {
    /// Appends `message`, or puts it ahead of everything not already on the
    /// wire when `front` is set. Returns its id.
    pub fn push(&mut self, thread_id: &str, message: QueuedMessage, front: bool) -> u64 {
        self.push_at(thread_id, message, front, None)
    }

    pub fn push_at(
        &mut self,
        thread_id: &str,
        mut message: QueuedMessage,
        front: bool,
        tool: Option<String>,
    ) -> u64 {
        self.next_id += 1;
        message.id = self.next_id;
        let queue = self.threads.entry(thread_id.to_owned()).or_default();
        if let Some(tool) = &tool {
            queue.consumed_tools.insert(tool.clone());
        }
        queue.tools.insert(message.id, tool);
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
        !self.messages(thread_id).is_empty()
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
        queue.tools.remove(&id);
        queue.source_plans.remove(&id);
        queue.contexts.remove(&id);
        queue.held.remove(&id);
        self.forget_if_empty(thread_id);
        Some(message)
    }

    pub fn set_context(&mut self, thread_id: &str, id: u64, context: Value) {
        if let Some(queue) = self.threads.get_mut(thread_id) {
            queue.contexts.insert(id, context);
        }
    }
    pub fn context(&self, thread_id: &str, id: u64) -> Option<Value> {
        self.threads.get(thread_id)?.contexts.get(&id).cloned()
    }
    pub fn set_source_plan(&mut self, thread_id: &str, id: u64, source: Value) {
        if let Some(queue) = self.threads.get_mut(thread_id) {
            queue.source_plans.insert(id, source);
        }
    }
    pub fn source_plan(&self, thread_id: &str, id: u64) -> Option<Value> {
        self.threads.get(thread_id)?.source_plans.get(&id).cloned()
    }
    pub fn held(&self, thread_id: &str) -> bool {
        self.threads.get(thread_id).is_some_and(|queue| {
            queue
                .messages
                .first()
                .is_some_and(|message| queue.held.contains(&message.id))
        })
    }

    /// Compatibility observation for projections without identity or tool events.
    pub fn observe(&mut self, thread_id: &str, running: bool) -> Option<QueuedMessage> {
        self.observe_state(
            thread_id,
            &Observation {
                running,
                ..Observation::default()
            },
        )
    }

    pub fn record(&mut self, thread_id: &str, state: &Observation) {
        if let Some(queue) = self.threads.get_mut(thread_id) {
            queue.observed = state.identity.clone();
            if let Some(identity) = &state.detail_identity {
                queue.observed_detail = Some(identity.clone());
            }
            if state.tool_available {
                queue.observed_tool = state.tool.clone();
            }
        }
    }

    /// Fresh stream history is a baseline, not a tool or dispatch receipt.
    pub fn baseline_detail(&mut self, thread_id: &str, state: &Observation) {
        self.record(thread_id, state);
        if let Some(queue) = self.threads.get_mut(thread_id) {
            if queue.awaiting_turn || queue.dispatching {
                queue.baseline_detail = state.detail_identity.clone();
            }
            for tool in queue.tools.values_mut() {
                *tool = state.tool.clone();
            }
            if let Some(tool) = &state.tool {
                queue.consumed_tools.insert(tool.clone());
            }
        }
    }

    pub fn observe_state(&mut self, thread_id: &str, state: &Observation) -> Option<QueuedMessage> {
        self.record(thread_id, state);
        let queue = self.threads.get_mut(thread_id)?;
        if queue.awaiting_turn
            && (state.identity != queue.baseline
                || state
                    .detail_identity
                    .as_ref()
                    .zip(queue.baseline_detail.as_ref())
                    .is_some_and(|(observed, baseline)| observed != baseline)
                || (state.identity.is_empty() && state.running))
        {
            queue.awaiting_turn = false;
        }
        if state.blocked
            || state.connecting
            || queue.dispatching
            || queue.awaiting_turn
            || queue
                .messages
                .first()
                .is_some_and(|message| queue.held.contains(&message.id))
        {
            return None;
        }
        let message = queue.messages.first()?;
        if state.running
            && (!state.tool_available
                || state.tool.is_none()
                || state
                    .tool
                    .as_ref()
                    .is_some_and(|tool| queue.consumed_tools.contains(tool))
                || queue.tools.get(&message.id).unwrap_or(&None) == &state.tool)
        {
            return None;
        }
        for tool in queue.tools.values_mut() {
            *tool = queue.observed_tool.clone();
        }
        if let Some(tool) = &state.tool {
            queue.consumed_tools.insert(tool.clone());
        }
        queue.baseline = state.identity.clone();
        queue.baseline_detail = queue.observed_detail.clone();
        queue.dispatching = true;
        Some(message.clone())
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
        for tool in queue.tools.values_mut() {
            *tool = queue.observed_tool.clone();
        }
        queue.baseline = queue.observed.clone();
        queue.baseline_detail = queue.observed_detail.clone();
        queue.dispatching = true;
        queue.held.remove(&id);
        Some(message)
    }

    /// The receipt for the head's send. Accepted drops it; rejected keeps it
    /// at the head until explicit retry or removal.
    pub fn settle(&mut self, thread_id: &str, id: u64, accepted: bool) {
        let Some(queue) = self.threads.get_mut(thread_id) else {
            return;
        };
        queue.dispatching = false;
        if accepted {
            queue.messages.retain(|message| message.id != id);
            queue.tools.remove(&id);
            queue.source_plans.remove(&id);
            queue.contexts.remove(&id);
            queue.awaiting_turn =
                queue.observed == queue.baseline && queue.observed_detail == queue.baseline_detail;
        } else {
            queue.held.insert(id);
        }
        self.forget_if_empty(thread_id);
    }

    fn forget_if_empty(&mut self, thread_id: &str) {
        if self
            .threads
            .get(thread_id)
            .is_some_and(|queue| queue.messages.is_empty() && !queue.awaiting_turn)
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
    fn a_rejected_send_stays_at_the_head_until_explicit_retry() {
        let mut queues = Queues::default();
        let first = queues.push("t", message("first"), false);
        queues.push("t", message("second"), false);
        queues.observe("t", false).unwrap();
        queues.settle("t", first, false);
        assert_eq!(texts(&queues, "t"), ["first", "second"]);
        assert_eq!(queues.observe("t", false), None);
        queues.observe("t", true);
        assert_eq!(queues.observe("t", false), None);
        assert_eq!(queues.take("t", first).unwrap().id, first);
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
    fn unavailable_or_empty_tools_never_invent_a_completion() {
        let mut queues = Queues::default();
        let id = queues.push_at("t", message("next"), false, Some("tool-1".into()));
        let mut state = Observation {
            running: true,
            identity: "old".into(),
            tool: Some("tool-1".into()),
            tool_available: true,
            ..Observation::default()
        };
        assert_eq!(queues.observe_state("t", &state), None);
        state.tool = None;
        state.tool_available = false;
        assert_eq!(queues.observe_state("t", &state), None);
        state.tool_available = true;
        assert_eq!(queues.observe_state("t", &state), None);
        state.tool = Some("tool-1".into());
        assert_eq!(queues.observe_state("t", &state), None);
        state.tool = Some("tool-2".into());
        assert_eq!(queues.observe_state("t", &state).unwrap().id, id);
    }

    #[test]
    fn receipt_markers_acknowledge_before_or_after_acceptance_without_detail_loss_ack() {
        for before_receipt in [false, true] {
            for detail_marker in [false, true] {
                let mut queues = Queues::default();
                let first = queues.push("t", message("first"), false);
                let second = queues.push("t", message("second"), false);
                let mut state = Observation {
                    identity: "ready-session".into(),
                    detail_identity: Some("old-turn-and-session".into()),
                    ..Observation::default()
                };
                assert_eq!(queues.observe_state("t", &state).unwrap().id, first);
                // Selection loss does not acknowledge the accepted send.
                state.detail_identity = None;
                queues.record("t", &state);
                if !before_receipt {
                    queues.settle("t", first, true);
                    assert_eq!(queues.observe_state("t", &state), None);
                }
                if detail_marker {
                    state.detail_identity = Some("new-start-failure-or-session-timestamp".into());
                } else {
                    state.identity = "failed-session".into();
                }
                if before_receipt {
                    queues.record("t", &state);
                    queues.settle("t", first, true);
                }
                assert_eq!(queues.observe_state("t", &state).unwrap().id, second);
            }
        }
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
