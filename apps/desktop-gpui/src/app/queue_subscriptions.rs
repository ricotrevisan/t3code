//! Queue-scoped, reasoning-free detail streams; history is a baseline, never a boundary.
use crate::NativeApp;
use gpui_kit::Context;
use serde_json::{Value, json};
use t3code_desktop_gpui::{connection, thread};

pub(crate) struct QueueDetail {
    pub(crate) key: String,
    generation: u64,
    pub(crate) detail: thread::ThreadDetail,
}

impl NativeApp {
    pub(crate) fn clear_queue_subscriptions(&mut self) {
        for (_, cache) in self.queue_details.drain() {
            if let Some(sender) = &self.commands {
                let _ = sender.try_send(connection::Command::Unsubscribe(cache.key));
            }
        }
    }

    pub(crate) fn reconcile_queue_subscriptions(&mut self) {
        let obsolete: Vec<_> = self
            .queue_details
            .keys()
            .filter(|id| !self.queues.has_queued(id))
            .cloned()
            .collect();
        for id in obsolete {
            if let Some(cache) = self.queue_details.remove(&id)
                && let Some(sender) = &self.commands
            {
                let _ = sender.try_send(connection::Command::Unsubscribe(cache.key));
            }
        }
        let Some(sender) = &self.commands else {
            return;
        };
        if !self.shell.synchronized {
            return;
        }
        for id in self.queues.thread_ids() {
            if !self.queues.has_queued(&id) || self.queue_details.contains_key(&id) {
                continue;
            }
            self.queue_subscription_epoch += 1;
            let key = format!(
                "queue-detail:{}:{}",
                self.generation, self.queue_subscription_epoch
            );
            if sender.try_send(connection::Command::Subscribe {
                key: key.clone(), tag: "orchestration.subscribeThread".into(),
                payload: json!({"threadId":id,"reasoningMessages":false,"requestCompletionMarker":true}),
            }).is_ok() {
                self.queue_details.insert(id, QueueDetail {
                    key, generation: self.generation, detail: thread::ThreadDetail::default(),
                });
            }
        }
    }

    /// Ignores callbacks belonging to a removed/replaced subscription.
    pub(crate) fn apply_queue_subscription(
        &mut self,
        key: &str,
        item: Result<Value, String>,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        let Some(id) = self.queue_details.iter().find_map(|(id, cache)| {
            (cache.key == key && cache.generation == self.generation).then(|| id.clone())
        }) else {
            return Ok(());
        };
        let cache = self.queue_details.get_mut(&id).unwrap();
        let item = match item {
            Ok(item) => item,
            Err(error) => {
                cache.detail = thread::ThreadDetail::default();
                self.report_error(format!("Queued thread stream failed: {error}"), cx);
                return Ok(());
            }
        };
        let was_synchronized = cache.detail.synchronized;
        cache.detail.apply(&id, item)?;
        let synchronized = cache.detail.synchronized;
        if synchronized && !was_synchronized {
            // Snapshot + replay is history. Rebase receipt markers too, so a
            // reconnect cannot acknowledge an outstanding dispatch by itself.
            if let Some(state) = self.queue_observation_for(&id, true) {
                self.queues.baseline_detail(&id, &state);
            }
        }
        if synchronized {
            self.flush_queues(cx);
        }
        Ok(())
    }
}
