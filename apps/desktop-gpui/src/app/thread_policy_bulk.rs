//! Modifier selection and receipt-ordered, capability-checked bulk lifecycle.
use crate::NativeApp;
use gpui_kit::{Context, Modifiers, Window};
use serde_json::Value;
use t3code_desktop_gpui::{commands, connection, sidebar};

impl NativeApp {
    pub(crate) fn reveal_sidebar_selection(&mut self) {
        if let Some(thread) = self
            .shell
            .threads
            .iter()
            .chain(&self.archived_shell.threads)
            .find(|thread| Some(&thread.id) == self.selected_thread.as_ref())
        {
            self.collapsed_projects
                .retain(|id| id != &thread.project_id);
            if self
                .sidebar_state
                .project_scope
                .as_ref()
                .is_some_and(|scope| scope != &thread.project_id)
            {
                self.sidebar_state.project_scope = None;
            }
            let group = if sidebar::snoozed(thread, chrono::Utc::now()) {
                "snoozed"
            } else if thread.settled_override.as_deref() == Some("settled") {
                "settled"
            } else if thread.pinned_at.is_some() {
                "pinned"
            } else {
                "active"
            };
            let project = if self.sidebar_state.priority && thread.archived_at.is_none() {
                ""
            } else {
                &thread.project_id
            };
            self.collapsed_groups
                .retain(|key| key != &sidebar::group_key(project, group));
            if thread.archived_at.is_some() {
                self.sidebar_view = sidebar::View::Archived;
            } else if !self.sidebar_state.priority {
                self.sidebar_view = if group == "snoozed" {
                    sidebar::View::Snoozed
                } else if group == "settled" {
                    sidebar::View::Settled
                } else {
                    sidebar::View::Active
                };
            } else {
                self.sidebar_view = sidebar::View::Active;
            }
        }
    }

    pub(crate) fn sidebar_selectable_order(&self) -> Vec<String> {
        let shell = if self.sidebar_view == sidebar::View::Archived {
            &self.archived_shell
        } else {
            &self.shell
        };
        let now = chrono::Utc::now();
        if self.sidebar_state.priority && self.sidebar_view != sidebar::View::Archived {
            return self
                .sidebar_state
                .priority_groups(&shell.threads, shell.revision, now, &self.environment_scope)
                .into_iter()
                .filter(|(group, _, _)| {
                    !self
                        .collapsed_groups
                        .contains(&sidebar::group_key("", group))
                })
                .flat_map(|(_, _, rows)| rows.into_iter().map(|thread| thread.id.clone()))
                .collect();
        }
        let mut order = Vec::new();
        for project in &self.shell.projects {
            if self.collapsed_projects.contains(&project.id)
                || self
                    .sidebar_state
                    .project_scope
                    .as_ref()
                    .is_some_and(|scope| scope != &project.id)
            {
                continue;
            }
            for (group, _, rows) in self.sidebar_state.grouped(
                &shell.threads,
                Some(shell.revision),
                &project.id,
                self.sidebar_view,
                now,
                &self.environment_scope,
            ) {
                if !self
                    .collapsed_groups
                    .contains(&sidebar::group_key(&project.id, group))
                {
                    order.extend(rows.into_iter().map(|thread| thread.id.clone()));
                }
            }
        }
        order
    }

    pub(crate) fn reconcile_sidebar_selection(&mut self) {
        let order = self.sidebar_selectable_order();
        self.sidebar_state
            .reconcile_selection(&self.environment_scope, &order);
    }

    pub(crate) fn sidebar_row_click(
        &mut self,
        id: &str,
        modifiers: Modifiers,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.reconcile_sidebar_selection();
        if self.sidebar_state.bulk_busy && self.sidebar_state.bulk_generation == self.generation {
            return;
        }
        let order = self.sidebar_selectable_order();
        if !order.iter().any(|entry| entry == id) {
            return;
        }
        if modifiers.shift {
            let anchor = self
                .sidebar_state
                .selection_anchor
                .as_deref()
                .or(self.selected_thread.as_deref());
            if let Some(start) = anchor.and_then(|id| order.iter().position(|entry| entry == id))
                && let Some(end) = order.iter().position(|entry| entry == id)
            {
                self.sidebar_state.selection = order[start.min(end)..=start.max(end)].to_vec();
            } else {
                self.sidebar_state.selection = vec![id.into()];
                self.sidebar_state.selection_anchor = Some(id.into());
            }
        } else if modifiers.platform || modifiers.control {
            if let Some(index) = self
                .sidebar_state
                .selection
                .iter()
                .position(|entry| entry == id)
            {
                self.sidebar_state.selection.remove(index);
            } else {
                if self.sidebar_state.selection.is_empty()
                    && let Some(selected) = &self.selected_thread
                    && order.contains(selected)
                {
                    self.sidebar_state.selection.push(selected.clone());
                }
                if !self.sidebar_state.selection.iter().any(|entry| entry == id) {
                    self.sidebar_state.selection.push(id.into());
                }
            }
            self.sidebar_state.selection_anchor = Some(id.into());
        } else {
            self.select_thread(id, window, cx);
            if self.selected_thread.as_deref() == Some(id) {
                self.sidebar_state.selection = vec![id.into()];
                self.sidebar_state.selection_anchor = Some(id.into());
            }
        }
        cx.notify();
    }

    pub(crate) fn bulk_thread_lifecycle(
        &mut self,
        action: &str,
        kind: &str,
        cx: &mut Context<Self>,
    ) {
        self.bulk_thread_lifecycle_until(action, kind, None, cx);
    }

    pub(crate) fn bulk_thread_lifecycle_until(
        &mut self,
        action: &str,
        kind: &str,
        until: Option<String>,
        cx: &mut Context<Self>,
    ) {
        if !self.connection_ready()
            || (self.sidebar_state.bulk_busy
                && self.sidebar_state.bulk_generation == self.generation)
        {
            return;
        }
        self.reconcile_sidebar_selection();
        let Some(sender) = self.commands.clone() else {
            self.report_error("Not connected", cx);
            return;
        };
        if matches!(action, "archive" | "delete")
            && self
                .selected_thread
                .as_ref()
                .is_some_and(|id| self.sidebar_state.selection.contains(id))
            && !self.can_change_checkout(cx)
        {
            return;
        }
        let now = chrono::Utc::now();
        let legacy =
            serde_json::json!({"threadSettlement":true,"threadPinning":true,"threadSnooze":true});
        let capabilities = if self.sidebar_state.capabilities.is_null() {
            &legacy
        } else {
            &self.sidebar_state.capabilities
        };
        let review = (
            self.generation,
            self.environment_scope.clone(),
            self.sidebar_state.selection.clone(),
        );
        if action == "delete" && self.sidebar_state.bulk_delete_review.as_ref() != Some(&review) {
            self.sidebar_state.bulk_delete_review = Some(review);
            self.report_error("Review selected targets; click Delete again to permanently clear their conversation history", cx);
            return;
        }
        self.sidebar_state.bulk_delete_review = None;
        if action == "snooze"
            && !self.sidebar_state.selection.iter().all(|id| {
                self.shell
                    .threads
                    .iter()
                    .find(|thread| &thread.id == id)
                    .is_some_and(|thread| {
                        sidebar::thread_action_policy(Some(thread), action, now, capabilities)
                            && !self.queues.has_queued(id)
                    })
            })
        {
            self.report_error("Every selected thread must be eligible for snooze", cx);
            return;
        }
        let mut calls: Vec<(String, Value, Vec<Value>)> = Vec::new();
        for id in &self.sidebar_state.selection {
            if let Some(thread) = self
                .shell
                .threads
                .iter()
                .chain(&self.archived_shell.threads)
                .find(|thread| &thread.id == id)
                && sidebar::thread_action_policy(Some(thread), action, now, capabilities)
                && !(matches!(action, "settle" | "snooze") && self.queues.has_queued(id))
            {
                let mut command = commands::command(kind, id);
                if matches!(kind, "thread.unsettle" | "thread.unsnooze") {
                    command["reason"] = serde_json::json!("user");
                }
                if kind == "thread.snooze" {
                    command["snoozedUntil"] =
                        serde_json::json!(until.clone().unwrap_or_else(|| {
                            (now + chrono::Duration::hours(1))
                                .to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
                        }));
                }
                if kind == "thread.meta.update" {
                    command["regenerateTitle"] = serde_json::json!(true);
                }
                calls.push((
                    id.clone(),
                    command,
                    super::thread_undo::navigation_inverses(thread, kind),
                ));
            }
        }
        if calls.is_empty() {
            self.report_error("No selected threads can perform this action", cx);
            return;
        }
        let action = action.to_owned();
        let capabilities = capabilities.clone();
        let generation = self.generation;
        let environment = self.environment_scope.clone();
        self.thread_undo.serial += 1;
        let undo_serial = self.thread_undo.serial;
        self.thread_undo.record = None;
        self.thread_undo.busy = false;
        self.sidebar_state.bulk_busy = true;
        self.sidebar_state.bulk_generation = generation;
        cx.spawn(async move |entity, cx| {
            let mut succeeded = Vec::new();
            let mut failures = Vec::new();
            let mut inverses = Vec::new();
            for (id, payload, inverse) in calls {
                let current = entity
                    .read_with(cx, |app, _| {
                        app.generation == generation
                            && app.environment_scope == environment
                            && app.sidebar_selectable_order().contains(&id)
                            && app
                                .shell
                                .threads
                                .iter()
                                .chain(&app.archived_shell.threads)
                                .find(|thread| thread.id == id)
                                .is_some_and(|thread| {
                                    sidebar::thread_action_policy(
                                        Some(thread),
                                        &action,
                                        chrono::Utc::now(),
                                        &capabilities,
                                    ) && !(matches!(action.as_str(), "settle" | "snooze")
                                        && app.queues.has_queued(&id))
                                })
                    })
                    .unwrap_or(false);
                if !current {
                    failures.push(format!("{id}: no longer visible or applicable"));
                    continue;
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
                        .unwrap_or_else(|_| Err("Lifecycle reply lost".into()))
                };
                match result {
                    Ok(_) => {
                        succeeded.push(id);
                        inverses.extend(inverse);
                    }
                    Err(error) => failures.push(format!("{id}: {error}")),
                }
            }
            let _ = entity.update(cx, |app, cx| {
                if app.generation != generation || app.environment_scope != environment {
                    return;
                }
                app.sidebar_state.bulk_busy = false;
                if app.thread_undo.serial == undo_serial && !inverses.is_empty() {
                    app.thread_undo.record = Some(super::thread_undo::UndoRecord {
                        environment: environment.clone(),
                        generation,
                        title: format!("{} threads", succeeded.len()),
                        batch: true,
                        inverses,
                    });
                }
                app.sidebar_state
                    .selection
                    .retain(|id| !succeeded.contains(id));
                if failures.is_empty() {
                    app.report_outcome(
                        t3code_desktop_gpui::notifications::Outcome::Success,
                        format!(
                            "Updated {} threads; inapplicable selections retained",
                            succeeded.len()
                        ),
                        cx,
                    );
                } else {
                    app.report_error(
                        format!(
                            "Updated {} threads; failures retained: {}",
                            succeeded.len(),
                            failures.join("; ")
                        ),
                        cx,
                    );
                }
                cx.notify();
            });
        })
        .detach();
        self.close_thread_menu();
        cx.notify();
    }
}
