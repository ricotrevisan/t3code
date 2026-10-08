//! Navigation undo is armed by accepted receipts, never by the initial click.
use crate::NativeApp;
use gpui_kit::Context;
use serde_json::{Value, json};
use t3code_desktop_gpui::{commands, connection, sidebar};

#[derive(Default)]
pub(crate) struct UndoState {
    pub(crate) serial: u64,
    pub(crate) busy: bool,
    pub(crate) generation: u64,
    pub(crate) record: Option<UndoRecord>,
}
#[derive(Clone)]
pub(crate) struct UndoRecord {
    pub(crate) environment: String,
    pub(crate) generation: u64,
    pub(crate) title: String,
    pub(crate) inverses: Vec<Value>,
}

impl NativeApp {
    pub(crate) fn dispatch_navigation(&mut self, payload: Value, cx: &mut Context<Self>) {
        let kind = payload["type"].as_str().unwrap_or("");
        if !matches!(
            kind,
            "thread.settle" | "thread.snooze" | "thread.archive" | "thread.unpin"
        ) {
            if matches!(
                kind,
                "thread.unsettle"
                    | "thread.unsnooze"
                    | "thread.unarchive"
                    | "thread.pin"
                    | "thread.delete"
            ) {
                self.thread_undo.serial += 1;
                self.thread_undo.record = None;
                self.thread_undo.busy = false;
            }
            self.dispatch(payload, cx);
            return;
        }
        let Some(id) = payload["threadId"].as_str() else {
            return;
        };
        let Some(thread) = self.shell.threads.iter().find(|thread| thread.id == id) else {
            self.dispatch(payload, cx);
            return;
        };
        let mut inverse = commands::command(
            match kind {
                "thread.settle" => "thread.unsettle",
                "thread.snooze" => "thread.unsnooze",
                "thread.archive" => "thread.unarchive",
                _ => "thread.pin",
            },
            id,
        );
        if matches!(kind, "thread.settle" | "thread.snooze") {
            inverse["reason"] = json!("user");
        }
        if kind == "thread.unpin"
            && let Some(key) = &thread.pin_order_key
        {
            inverse["orderKey"] = json!(key);
        }
        let mut inverses = vec![inverse];
        if kind == "thread.settle" && thread.pinned_at.is_some() {
            let mut pin = commands::command("thread.pin", id);
            if let Some(key) = &thread.pin_order_key {
                pin["orderKey"] = json!(key);
            }
            inverses.push(pin);
        }
        if kind == "thread.settle" {
            if let Some(key) = &thread.active_order_key {
                let mut order = commands::command("thread.active.reorder", id);
                order["orderKey"] = json!(key);
                inverses.push(order);
            }
            if let Some(until) = &thread.snoozed_until
                && chrono::DateTime::parse_from_rfc3339(until)
                    .ok()
                    .is_some_and(|at| at > chrono::Utc::now())
            {
                let mut snooze = commands::command("thread.snooze", id);
                snooze["snoozedUntil"] = json!(until);
                inverses.push(snooze);
            }
        }
        let record = UndoRecord {
            environment: self.environment_scope.clone(),
            generation: self.generation,
            title: thread.title.clone(),
            inverses,
        };
        let Some(sender) = self.commands.clone() else {
            self.report_error("Not connected", cx);
            return;
        };
        self.thread_undo.serial += 1;
        let serial = self.thread_undo.serial;
        self.thread_undo.busy = true;
        self.thread_undo.generation = self.generation;
        cx.spawn(async move |entity, cx| {
            let (reply, receipt) = async_channel::bounded(1);
            let result = if sender
                .send(connection::Command::Call {
                    tag: "orchestration.dispatchCommand".into(),
                    payload,
                    reply,
                })
                .await
                .is_err()
            {
                Err("Connection closed".to_owned())
            } else {
                receipt
                    .recv()
                    .await
                    .unwrap_or_else(|_| Err("Navigation reply lost".into()))
            };
            let _ = entity.update(cx, |app, cx| {
                if app.generation != record.generation
                    || app.environment_scope != record.environment
                    || app.thread_undo.serial != serial
                {
                    return;
                }
                app.thread_undo.busy = false;
                match result {
                    Ok(_) => {
                        app.thread_undo.record = Some(record);
                        app.report_success("Thread updated · Undo available", cx);
                    }
                    Err(error) => app.report_error(error, cx),
                }
                if app.sidebar_view == sidebar::View::Archived {
                    app.perform(
                        "orchestration.getArchivedShellSnapshot",
                        json!({}),
                        None,
                        cx,
                    );
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    pub(crate) fn undo_thread_navigation(&mut self, cx: &mut Context<Self>) {
        if self.thread_undo.busy && self.thread_undo.generation == self.generation {
            return;
        }
        let Some(mut record) = self.thread_undo.record.clone() else {
            return;
        };
        if record.generation != self.generation || record.environment != self.environment_scope {
            self.thread_undo.record = None;
            cx.notify();
            return;
        }
        let Some(sender) = self.commands.clone() else {
            self.report_error("Not connected; undo retained", cx);
            return;
        };
        self.thread_undo.serial += 1;
        let serial = self.thread_undo.serial;
        self.thread_undo.busy = true;
        self.thread_undo.generation = self.generation;
        cx.spawn(async move |entity, cx| {
            let mut failure = None;
            while let Some(payload) = record.inverses.first().cloned() {
                if payload["type"] == "thread.snooze"
                    && payload["snoozedUntil"]
                        .as_str()
                        .and_then(|at| chrono::DateTime::parse_from_rfc3339(at).ok())
                        .is_none_or(|at| at <= chrono::Utc::now())
                {
                    record.inverses.remove(0);
                    continue;
                }
                let current = entity
                    .read_with(cx, |app, _| {
                        app.generation == record.generation
                            && app.environment_scope == record.environment
                            && app.thread_undo.serial == serial
                    })
                    .unwrap_or(false);
                if !current {
                    return;
                }
                let (reply, receipt) = async_channel::bounded(1);
                let result = if sender
                    .send(connection::Command::Call {
                        tag: "orchestration.dispatchCommand".into(),
                        payload,
                        reply,
                    })
                    .await
                    .is_err()
                {
                    Err("Connection closed".to_owned())
                } else {
                    receipt
                        .recv()
                        .await
                        .unwrap_or_else(|_| Err("Undo reply lost".into()))
                };
                if let Err(error) = result {
                    failure = Some(error);
                    break;
                }
                record.inverses.remove(0);
            }
            let _ = entity.update(cx, |app, cx| {
                if app.generation != record.generation
                    || app.environment_scope != record.environment
                    || app.thread_undo.serial != serial
                {
                    return;
                }
                app.thread_undo.busy = false;
                if let Some(error) = failure {
                    app.thread_undo.record = Some(record);
                    app.report_error(format!("Undo failed; retry retained: {error}"), cx);
                } else {
                    app.thread_undo.record = None;
                    app.report_success("Thread navigation action undone", cx);
                }
                if app.sidebar_view == sidebar::View::Archived {
                    app.perform(
                        "orchestration.getArchivedShellSnapshot",
                        json!({}),
                        None,
                        cx,
                    );
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
}
