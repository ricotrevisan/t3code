//! Checkout mutations require provider completion, not persisted command acceptance.
use crate::NativeApp;
use gpui_kit::{AsyncApp, WeakEntity};
use serde_json::{Value, json};
use t3code_desktop_gpui::{commands, connection, thread};

#[derive(Default)]
pub(crate) struct CheckoutStops {
    active: Option<Stop>,
}

struct Stop {
    key: String,
    thread_id: String,
    command_id: String,
    stopped_sequence: Option<u64>,
    failure_sequence: Option<u64>,
    generation: u64,
    gate: connection::MutationGate,
    sender: async_channel::Sender<connection::Command>,
    detail: thread::ThreadDetail,
    ready: async_channel::Sender<Result<(), String>>,
    completion: async_channel::Sender<Result<u64, String>>,
}

/// An unsubscribe must not be silently lost when the transport queue is full.
/// At most one stop lease exists; the fallback owns no app/OS/provider state.
fn unsubscribe(sender: &async_channel::Sender<connection::Command>, key: String) {
    if let Err(async_channel::TrySendError::Full(command)) =
        sender.try_send(connection::Command::Unsubscribe(key))
    {
        let sender = sender.clone();
        std::thread::spawn(move || {
            let _ = sender.send_blocking(command);
        });
    }
}

struct Interest {
    sender: async_channel::Sender<connection::Command>,
    key: String,
    gate: connection::MutationGate,
}
impl Drop for Interest {
    fn drop(&mut self) {
        self.gate.cancel_before_send();
        unsubscribe(&self.sender, self.key.clone());
    }
}

impl Stop {
    fn fail(&self, message: &str) {
        self.gate.cancel_before_send();
        let _ = self.ready.try_send(Err(message.into()));
        let _ = self.completion.try_send(Err(message.into()));
        self.ready.close();
        self.completion.close();
        unsubscribe(&self.sender, self.key.clone());
    }
}

impl NativeApp {
    #[cfg(test)]
    pub(crate) fn test_checkout_stop(
        &mut self,
        cx: &mut gpui_kit::Context<Self>,
    ) -> async_channel::Receiver<Result<(), String>> {
        let sender = self.commands.as_ref().unwrap().clone();
        let (reply, result) = async_channel::bounded(1);
        cx.spawn(async move |entity, cx| {
            let result = stop_session(&entity, &sender, "t", cx).await;
            let _ = reply.send(result).await;
        })
        .detach();
        result
    }

    pub(crate) fn clear_checkout_stops(&mut self) {
        if let Some(stop) = self.checkout_stops.active.take() {
            stop.fail("Checkout stop observation canceled, disconnected or resynchronized");
        }
    }

    pub(crate) fn apply_checkout_stop_subscription(
        &mut self,
        key: &str,
        item: Result<Value, String>,
    ) {
        let Some(stop) = self.checkout_stops.active.as_mut() else {
            return;
        };
        if stop.key != key {
            return;
        }
        if stop.generation != self.generation || stop.completion.is_closed() {
            self.clear_checkout_stops();
            return;
        }
        let result = (|| -> Result<(), String> {
            let item = item?;
            let live = stop.detail.synchronized;
            let previous = stop.detail.sequence;
            if live && item["kind"] == "snapshot" {
                return Err("Checkout stop stream resynchronized".into());
            }
            if item["kind"] == "snapshot" && item["supportsCorrelatedSessionStops"] != true {
                return Err("Update the host server to support correlated session stops before changing checkout".into());
            }
            stop.detail.apply(&stop.thread_id, item.clone())?;
            if stop.detail.thread.is_none() {
                return Err("Checkout stop thread no longer exists".into());
            }
            if !live && stop.detail.synchronized {
                let _ = stop.ready.try_send(Ok(()));
            }
            // Snapshot/replay and duplicate events cannot prove this request
            // completed. In particular ThreadDetail's stop-request projection
            // is optimistic: only the reactor's session-set is authoritative.
            if !live || item["kind"] != "event" || stop.detail.sequence <= previous {
                return Ok(());
            }
            let event = &item["event"];
            let sequence = event["sequence"]
                .as_u64()
                .ok_or("Invalid stop event sequence")?;
            if event["type"] == "thread.session-set" {
                let session = &event["payload"]["session"];
                if session["status"] != "stopped" {
                    stop.stopped_sequence = None;
                } else if session["stopCommandId"].as_str() == Some(stop.command_id.as_str()) {
                    stop.stopped_sequence = Some(sequence);
                    let _ = stop.completion.try_send(Ok(sequence));
                }
                // Unmarked/other workers may finish while our stop is queued.
                // Their stopped state neither proves nor rejects our completion.
            } else if event["type"] == "thread.activity-appended" {
                let activity = &event["payload"]["activity"];
                if activity["kind"] == "provider.session.stop.failed"
                    && activity["payload"]["commandId"].as_str() == Some(stop.command_id.as_str())
                {
                    stop.failure_sequence = Some(sequence);
                    let _ = stop.completion.try_send(Ok(sequence));
                }
            }
            Ok(())
        })();
        if let Err(error) = result {
            stop.fail(&error);
        }
    }
}

/// Register before dispatch; keep the dispatch receipt even after cancellation.
/// A capable host echoes the originating command ID only after worker success
/// (or in its failure activity). Require that exact ID and a fresh live event
/// beyond acceptance; sequence/timestamp alone cannot distinguish older workers.
pub(super) async fn stop_session(
    entity: &WeakEntity<NativeApp>,
    sender: &async_channel::Sender<connection::Command>,
    thread_id: &str,
    cx: &mut AsyncApp,
) -> Result<(), String> {
    let key = format!("checkout-stop-{}", commands::id());
    let mut payload = commands::command("thread.session.stop", thread_id);
    payload["createdAt"] = json!(commands::now());
    let command_id = payload["commandId"]
        .as_str()
        .ok_or("Invalid stop command ID")?
        .to_owned();
    let gate = connection::MutationGate::new();
    let (ready_tx, ready) = async_channel::bounded(1);
    let (completion_tx, completion) = async_channel::bounded(1);
    entity
        .update(cx, |app, _| -> Result<(), String> {
            if !app.shell.synchronized
                || app
                    .commands
                    .as_ref()
                    .is_none_or(|current| !current.same_channel(sender))
            {
                return Err("Checkout stop requires a synchronized connection".into());
            }
            if let Some(old) = &app.checkout_stops.active {
                if !old.completion.is_closed() {
                    return Err("A checkout stop is already pending".into());
                }
                app.clear_checkout_stops();
            }
            app.checkout_stops.active = Some(Stop {
                key: key.clone(),
                thread_id: thread_id.into(),
                command_id,
                stopped_sequence: None,
                failure_sequence: None,
                generation: app.generation,
                gate: gate.clone(),
                sender: sender.clone(),
                detail: Default::default(),
                ready: ready_tx,
                completion: completion_tx,
            });
            Ok(())
        })
        .map_err(|_| "Window closed")??;
    let interest = Interest {
        sender: sender.clone(),
        key: key.clone(),
        gate: gate.clone(),
    };
    let result = async {
        sender
            .send(connection::Command::Subscribe {
                key: key.clone(),
                tag: "orchestration.subscribeThread".into(),
                payload: json!({"threadId":thread_id,"reasoningMessages":false,"requestCompletionMarker":true}),
            })
            .await
            .map_err(|_| "Disconnected before stop observation")?;
        ready
            .recv()
            .await
            .map_err(|_| "Stop observation canceled")??;
        let receipt =
            connection::call_mutation(sender, "orchestration.dispatchCommand", payload, gate)
                .await?;
        let floor = receipt["sequence"]
            .as_u64()
            .ok_or("Invalid stop acceptance receipt")?;
        loop {
            // The bounded channel is only a wakeup; candidates stay in state.
            // A completion arriving while an older notification is buffered is
            // not lost, and history at/below acceptance never releases us.
            let candidate = entity.update(cx, |app, _| -> Result<Option<Result<(), String>>, String> {
                let stop = app.checkout_stops.active.as_ref().filter(|stop|
                    stop.key == key && stop.generation == app.generation && !stop.completion.is_closed())
                    .ok_or("Checkout stop interest was invalidated")?;
                if stop.failure_sequence.is_some_and(|sequence| sequence > floor) {
                    return Ok(Some(Err("Provider session stop failed; checkout was not changed".into())));
                }
                Ok(stop.stopped_sequence.filter(|sequence| *sequence > floor).map(|_| Ok(())))
            }).map_err(|_| "Window closed")??;
            if let Some(result) = candidate { return result; }
            completion.recv().await.map_err(|_| "Stop observation canceled")??;
        }
    }
    .await;
    let _ = entity.update(cx, |app, _| {
        if app
            .checkout_stops
            .active
            .as_ref()
            .is_some_and(|stop| stop.key == key)
        {
            app.checkout_stops.active.take();
        }
    });
    drop(interest);
    result
}
