//! Priority drops never mutate the shell optimistically: receipts own lifecycle/order.
use crate::NativeApp;
use gpui_kit::Context;
use serde_json::{Value, json};
use t3code_desktop_gpui::{commands, connection, sidebar};

impl NativeApp {
    pub(crate) fn sidebar_drop_plan(
        &self,
        id: &str,
        section: &str,
        before: Option<&str>,
    ) -> Option<Vec<Value>> {
        let now = chrono::Utc::now();
        let thread = self.shell.threads.iter().find(|thread| thread.id == id)?;
        let caps = &self.sidebar_state.capabilities;
        if thread.archived_at.is_some()
            || !self
                .sidebar_selectable_order()
                .iter()
                .any(|entry| entry == id)
        {
            return None;
        }
        let allowed = |action| sidebar::thread_action_policy(Some(thread), action, now, caps);
        if section == "settled" {
            if !allowed("settle") || self.queues.has_queued(id) {
                return None;
            }
            return Some(vec![commands::command("thread.settle", id)]);
        }
        let pinned = section == "pinned";
        if !matches!(section, "pinned" | "active")
            || caps[if pinned {
                "threadPinReorder"
            } else {
                "threadActiveReorder"
            }]
            .as_bool()
                != Some(true)
        {
            return None;
        }
        let groups = self.sidebar_state.priority_groups(
            &self.shell.threads,
            self.shell.revision,
            now,
            &self.environment_scope,
        );
        let rows: Vec<_> = groups[if pinned { 0 } else { 1 }]
            .2
            .iter()
            .copied()
            .filter(|thread| thread.id != id)
            .collect();
        let index = if let Some(before) = before {
            if before == id {
                return None;
            }
            rows.iter().position(|thread| thread.id == before)?
        } else {
            rows.len()
        };
        let key = |thread: &t3code_desktop_gpui::shell::Thread| {
            if pinned {
                thread.pin_order_key.clone()
            } else {
                thread.active_order_key.clone()
            }
        };
        let previous = if index > 0 {
            Some(key(rows[index - 1])?)
        } else {
            None
        };
        let next = if index < rows.len() {
            Some(key(rows[index])?)
        } else {
            None
        };
        let order = sidebar::key_between(previous.as_deref(), next.as_deref())?;
        if pinned {
            if thread.pinned_at.is_none() && !allowed("pin") {
                return None;
            }
            let mut command = commands::command(
                if thread.pinned_at.is_some() {
                    "thread.pin.reorder"
                } else {
                    "thread.pin"
                },
                id,
            );
            command["orderKey"] = json!(order);
            let mut calls = Vec::new();
            if thread.pinned_at.is_some() && sidebar::snoozed(thread, now) {
                if !allowed("unsnooze") {
                    return None;
                }
                let mut wake = commands::command("thread.unsnooze", id);
                wake["reason"] = json!("user");
                calls.push(wake);
            }
            calls.push(command);
            return Some(calls);
        }
        let mut calls = Vec::new();
        for (needed, action, kind) in [
            (thread.pinned_at.is_some(), "unpin", "thread.unpin"),
            (
                thread.settled_override.as_deref() == Some("settled"),
                "unsettle",
                "thread.unsettle",
            ),
            (sidebar::snoozed(thread, now), "unsnooze", "thread.unsnooze"),
        ] {
            if needed {
                if !allowed(action) {
                    return None;
                }
                let mut command = commands::command(kind, id);
                if matches!(action, "unsettle" | "unsnooze") {
                    command["reason"] = json!("user");
                }
                calls.push(command);
            }
        }
        let mut command = commands::command("thread.active.reorder", id);
        command["orderKey"] = json!(order);
        calls.push(command);
        Some(calls)
    }

    pub(crate) fn drop_sidebar_thread(
        &mut self,
        drag: &sidebar::Drag,
        section: &str,
        before: Option<&str>,
        cx: &mut Context<Self>,
    ) {
        if drag.environment != self.environment_scope
            || drag.generation != self.generation
            || !self.connection_ready()
            || (self.sidebar_state.order_busy
                && self.sidebar_state.order_generation == self.generation)
        {
            return;
        }
        let Some(calls) = self.sidebar_drop_plan(&drag.id, section, before) else {
            self.report_error(
                "This destination is unavailable (capability, thread state or unkeyed neighbors)",
                cx,
            );
            return;
        };
        let Some(sender) = self.commands.clone() else {
            return;
        };
        let environment = self.environment_scope.clone();
        let generation = self.generation;
        let id = drag.id.clone();
        let scope = self.sidebar_state.project_scope.clone();
        let view = self.sidebar_view;
        let priority = self.sidebar_state.priority;
        let collapsed_groups = self.collapsed_groups.clone();
        self.sidebar_state.order_busy = true;
        self.sidebar_state.order_generation = generation;
        self.thread_undo.serial += 1;
        self.thread_undo.record = None;
        cx.spawn(async move |entity, cx| {
            let mut failure = None;
            for payload in calls {
                if !entity
                    .read_with(cx, |app, _| {
                        app.generation == generation
                            && app.environment_scope == environment
                            && app.sidebar_state.project_scope == scope
                            && app.sidebar_view == view
                            && app.sidebar_state.priority == priority
                            && app.collapsed_groups == collapsed_groups
                            && app.shell.threads.iter().any(|thread| thread.id == id)
                    })
                    .unwrap_or(false)
                {
                    failure = Some(
                        "Navigation context changed; remaining drop operations cancelled".into(),
                    );
                    break;
                }
                if let Err(error) =
                    connection::call(&sender, "orchestration.dispatchCommand", payload).await
                {
                    failure = Some(error);
                    break;
                }
            }
            let _ = entity.update(cx, |app, cx| {
                if app.generation != generation || app.environment_scope != environment {
                    return;
                }
                app.sidebar_state.order_busy = false;
                if let Some(error) = failure {
                    app.report_error(
                        format!("Drop rejected; showing server-authoritative state: {error}"),
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
