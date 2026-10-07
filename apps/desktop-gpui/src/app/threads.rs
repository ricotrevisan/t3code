//! Thread selection, unsent thread drafts, and thread lifecycle commands.

use crate::NativeApp;
use gpui_kit::{Context, Window};
use serde_json::{Value, json};
use t3code_desktop_gpui::{commands, connection, drafts, project_actions, sidebar, thread};

impl NativeApp {
    /// Applies a thread subscription item. The conversation list keeps itself on
    /// the newest content unless the user has scrolled up to read history.
    pub(crate) fn apply_thread_update(
        &mut self,
        thread_id: &str,
        item: Value,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        self.detail.apply(thread_id, item)?;
        self.flush_queues(cx);
        Ok(())
    }

    pub(crate) fn turn_running(&self) -> bool {
        self.detail.thread.as_ref().is_some_and(|thread| {
            matches!(
                thread["session"]["status"].as_str(),
                Some("starting" | "running")
            )
        })
    }

    /// Selects the previous or next thread in the order the sidebar shows them.
    pub(crate) fn step_thread(
        &mut self,
        forward: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let now = chrono::Utc::now();
        let threads = if self.sidebar_view == sidebar::View::Archived {
            &self.archived_shell.threads
        } else {
            &self.shell.threads
        };
        let order: Vec<String> = self
            .shell
            .projects
            .iter()
            .filter(|project| !self.collapsed_projects.contains(&project.id))
            .flat_map(|project| {
                sidebar::grouped(threads, &project.id, self.sidebar_view, now)
                    .into_iter()
                    .filter(|(group, _, _)| {
                        !self
                            .collapsed_groups
                            .contains(&sidebar::group_key(&project.id, group))
                    })
                    .flat_map(|(_, _, rows)| rows.into_iter().map(|thread| thread.id.clone()))
                    .collect::<Vec<_>>()
            })
            .collect();
        if let Some(id) = sidebar::neighbor(&order, self.selected_thread.as_deref(), forward) {
            self.select_thread(&id, window, cx);
        }
    }

    pub(crate) fn new_thread(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(project) = self
            .shell
            .projects
            .iter()
            .find(|project| Some(&project.id) == self.selected_project.as_ref())
        else {
            self.set_status("Select a project first", cx);
            cx.notify();
            return;
        };
        let project_id = project.id.clone();
        let cwd = project.workspace_root.clone();
        let effective = project_actions::model_default(
            self.project_settings.as_ref(),
            &project.id,
            project.default_model_selection.as_ref(),
        );
        let previous = &effective;
        let instance = self.provider.read(cx).value();
        let model = self.model.read(cx).value();
        let selection = if instance.trim().is_empty() && model.trim().is_empty() {
            previous.clone()
        } else {
            match self.model_selection(previous, cx) {
                Ok(selection) => selection,
                Err(error) => {
                    self.set_status(error, cx);
                    cx.notify();
                    return;
                }
            }
        };
        let title = self.title.read(cx).value().to_string();
        let mut command = match commands::create_thread(&project_id, &title, selection) {
            Ok(value) => value,
            Err(error) => {
                self.set_status(error, cx);
                cx.notify();
                return;
            }
        };
        if let Some((id, branch, path)) = &self.new_thread_checkout
            && id == &project_id
        {
            command["branch"] = json!(branch);
            command["worktreePath"] = json!(path);
        }
        let prepare = if self.prepare_new_worktree {
            let base = self.worktree_ref.read(cx).value().trim().to_owned();
            if base.is_empty() {
                self.set_status("Select a base branch for the new worktree", cx);
                cx.notify();
                return;
            }
            let mut value = json!({"projectCwd":cwd,"baseBranch":base,"startFromOrigin":self.start_from_origin});
            let branch = self.worktree_branch.read(cx).value().trim().to_owned();
            if !branch.is_empty() {
                value["branch"] = json!(branch);
            }
            Some(value)
        } else {
            None
        };
        command["id"] = command["threadId"].clone();
        let draft = drafts::PendingThread {
            thread: serde_json::from_value(command.clone())
                .expect("native create command has shell fields"),
            created_at: command["createdAt"].as_str().unwrap().into(),
            prepare_worktree: prepare,
            run_setup_script: self.run_setup_script,
        };
        let id = draft.thread.id.clone();
        if let Some(store) = &mut self.draft_store {
            store.put_pending(&self.environment_scope, draft.clone());
        }
        self.pending_threads.push(draft);
        self.schedule_draft_save(cx);
        self.select_thread(&id, window, cx);
        self.set_status(
            "New thread draft · sent to the server with the first message",
            cx,
        );
        cx.notify();
    }

    pub(crate) fn toggle_thread_property(&mut self, pin: bool, cx: &mut Context<Self>) {
        let selected = self.selected_thread.as_ref().and_then(|id| {
            self.shell
                .threads
                .iter()
                .chain(&self.archived_shell.threads)
                .find(|thread| &thread.id == id)
        });
        let kind = if pin {
            if selected.is_some_and(|thread| thread.pinned_at.is_some()) {
                "thread.unpin"
            } else {
                "thread.pin"
            }
        } else if selected
            .is_some_and(|thread| thread.settled_override.as_deref() == Some("settled"))
        {
            "thread.unsettle"
        } else {
            "thread.settle"
        };
        self.lifecycle(kind, cx);
    }

    pub(crate) fn select_thread(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(previous) = &self.selected_thread {
            self.drafts
                .insert(previous.clone(), self.composer.read(cx).value().to_string());
        }
        self.model_override = None;
        self.selected_thread = Some(id.into());
        self.pending_history_attached = false;
        self.close_thread_menu();
        self.detail = thread::ThreadDetail::default();
        self.conversation_rows = Default::default();
        self.conversation_scroll.reset(0);
        self.conversation_scroll
            .set_follow_mode(gpui_kit::FollowMode::Tail);
        let text = self.drafts.get(id).cloned().unwrap_or_else(|| {
            self.draft_store
                .as_ref()
                .map(|store| store.get(&self.environment_scope, id).to_owned())
                .unwrap_or_default()
        });
        self.composer
            .update(cx, |input, cx| input.set_value(text, window, cx));
        if let Some(thread) = self
            .shell
            .threads
            .iter()
            .chain(&self.archived_shell.threads)
            .chain(self.pending_threads.iter().map(|draft| &draft.thread))
            .find(|t| t.id == id)
        {
            if self.selected_project.as_ref() != Some(&thread.project_id) {
                self.action_task = None;
                self.action_busy = false;
                self.action_edit_id = None;
                self.confirm_action_delete = None;
            }
            self.selected_project = Some(thread.project_id.clone());
            self.title.update(cx, |input, cx| {
                input.set_value(thread.title.clone(), window, cx)
            });
            self.provider.update(cx, |input, cx| {
                input.set_value(
                    thread.model_selection["instanceId"]
                        .as_str()
                        .unwrap_or("")
                        .to_owned(),
                    window,
                    cx,
                )
            });
            self.model.update(cx, |input, cx| {
                input.set_value(
                    thread.model_selection["model"]
                        .as_str()
                        .unwrap_or("")
                        .to_owned(),
                    window,
                    cx,
                )
            });
        }
        let pending = self
            .pending_threads
            .iter()
            .any(|draft| draft.thread.id == id)
            && !self.shell.threads.iter().any(|thread| thread.id == id);
        self.pending_history_attached = !pending;
        if pending
            && let Some(draft) = self
                .pending_threads
                .iter()
                .find(|draft| draft.thread.id == id)
        {
            self.prepare_new_worktree = draft.prepare_worktree.is_some();
            self.run_setup_script = draft.run_setup_script;
            if let Some(recipe) = &draft.prepare_worktree {
                self.start_from_origin = recipe["startFromOrigin"] == true;
                self.worktree_ref.update(cx, |input, cx| {
                    input.set_value(
                        recipe["baseBranch"].as_str().unwrap_or_default().to_owned(),
                        window,
                        cx,
                    )
                });
                self.worktree_branch.update(cx, |input, cx| {
                    input.set_value(
                        recipe["branch"].as_str().unwrap_or_default().to_owned(),
                        window,
                        cx,
                    )
                });
            }
        }
        self.setup_progress = None;
        if let Some(sender) = &self.commands {
            let _ = sender.try_send(connection::Command::Subscribe {
                key: "worktree-setup".into(),
                tag: "subscribeWorktreeSetup".into(),
                payload: json!({"threadId":id}),
            });
        }
        if let Some(sender) = &self.commands
            && sender
                .try_send(connection::Command::SelectThread(if pending {
                    None
                } else {
                    Some(id.into())
                }))
                .is_err()
        {
            self.set_status("Connection busy or closed; retry selection", cx);
        }
        cx.notify();
    }

    pub(crate) fn attach_created_pending(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.selected_thread.as_ref() else {
            return;
        };
        if !self
            .pending_threads
            .iter()
            .any(|draft| &draft.thread.id == id)
        {
            return;
        }
        let materialized = self.shell.threads.iter().any(|thread| &thread.id == id);
        if self.pending_history_attached && !materialized {
            // Bootstrap cancellation deletes the server thread but retains the local draft.
            // Its old starting session must not prevent sending that draft again.
            self.detail = thread::ThreadDetail::default();
            self.pending_history_attached = false;
            if let Some(sender) = &self.commands {
                let _ = sender.try_send(connection::Command::SelectThread(None));
            }
            cx.notify();
            return;
        }
        if self.pending_history_attached || !materialized {
            return;
        }
        if let Some(sender) = &self.commands
            && sender
                .try_send(connection::Command::SelectThread(Some(id.clone())))
                .is_ok()
        {
            self.pending_history_attached = true;
            cx.notify();
        }
    }

    pub(crate) fn discard_pending(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = self.selected_thread.clone() else {
            return;
        };
        if self.confirm_delete.as_ref() != Some(&id) {
            self.confirm_delete = Some(id);
            self.set_status("Click Delete again to discard this unsent thread", cx);
            cx.notify();
            return;
        }
        self.pending_threads.retain(|draft| draft.thread.id != id);
        self.drafts.remove(&id);
        if let Some(store) = &mut self.draft_store {
            store.remove_pending(&self.environment_scope, &id);
            store.set(&self.environment_scope, &id, String::new());
        }
        self.selected_thread = None;
        self.confirm_delete = None;
        self.composer
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.setup_progress = None;
        if let Some(sender) = &self.commands {
            let _ = sender.try_send(connection::Command::Unsubscribe("worktree-setup".into()));
        }
        self.schedule_draft_save(cx);
        self.set_status("Unsent thread discarded", cx);
        cx.notify();
    }

    /// Runs a thread command on the selected thread.
    pub(crate) fn lifecycle(&mut self, kind: &str, cx: &mut Context<Self>) {
        if let Some(id) = self.selected_thread.clone() {
            self.thread_lifecycle(&id, kind, cx);
        }
    }

    pub(crate) fn thread_lifecycle(&mut self, id: &str, kind: &str, cx: &mut Context<Self>) {
        if let Some(draft) = self
            .pending_threads
            .iter_mut()
            .find(|draft| draft.thread.id == id)
        {
            if kind == "thread.meta.update" {
                let title = self.title.read(cx).value().trim().to_owned();
                if title.is_empty() {
                    self.set_status("A title is required", cx);
                    cx.notify();
                    return;
                }
                draft.thread.title = title;
                if let Some(store) = &mut self.draft_store {
                    store.put_pending(&self.environment_scope, draft.clone());
                }
                self.schedule_draft_save(cx);
                self.set_status("Draft renamed", cx);
            } else {
                self.set_status("Send the first message before changing thread status", cx);
            }
            cx.notify();
            return;
        }
        let mut payload = commands::command(kind, id);
        match kind {
            "thread.meta.update" => {
                let title = self.title.read(cx).value();
                if title.trim().is_empty() {
                    self.set_status("A title is required", cx);
                    cx.notify();
                    return;
                }
                payload["title"] = json!(title.trim());
            }
            "thread.unsnooze" | "thread.unsettle" => payload["reason"] = json!("user"),
            "thread.snooze" => {
                payload["snoozedUntil"] = json!(
                    (chrono::Utc::now() + chrono::Duration::hours(1))
                        .to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
                )
            }
            "thread.turn.interrupt" | "thread.session.stop" => {
                payload["createdAt"] = json!(commands::now())
            }
            "thread.delete" if self.confirm_delete.as_deref() != Some(id) => {
                self.confirm_delete = Some(id.to_owned());
                self.set_status(
                    "Click Delete again to confirm permanent thread deletion",
                    cx,
                );
                cx.notify();
                return;
            }
            _ => {}
        }
        self.dispatch(payload, cx);
    }

    pub(crate) fn schedule_snooze_wake(&mut self, cx: &mut Context<Self>) {
        self.snooze_wake = sidebar::expiry(&self.shell.threads, chrono::Utc::now()).map(|delay| {
            cx.spawn(async move |entity, cx| {
                cx.background_executor().timer(delay).await;
                let _ = entity.update(cx, |app, cx| {
                    app.schedule_snooze_wake(cx);
                    cx.notify();
                });
            })
        });
    }

    pub(crate) fn snooze_custom(&mut self, cx: &mut Context<Self>) {
        let Some(id) = &self.selected_thread else {
            return;
        };
        let value = self.custom_snooze.read(cx).value();
        let until = match chrono::DateTime::parse_from_rfc3339(value.trim()) {
            Ok(until) if until > chrono::Utc::now() => until,
            _ => {
                self.set_status("Choose a future ISO date/time with timezone", cx);
                cx.notify();
                return;
            }
        };
        let mut command = commands::command("thread.snooze", id);
        command["snoozedUntil"] = json!(
            until
                .with_timezone(&chrono::Utc)
                .to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
        );
        self.dispatch(command, cx);
    }

    pub(crate) fn search_threads(&mut self, cx: &mut Context<Self>) {
        let query = self.search.read(cx).value();
        if !(2..=200).contains(&query.trim().chars().count()) {
            self.set_status("Search needs 2–200 characters", cx);
            cx.notify();
            return;
        }
        self.perform(
            "orchestration.searchThreads",
            json!({"query":query.trim(),"limit":50}),
            None,
            cx,
        );
    }

    /// Revert the working tree to a completed turn's checkpoint. The first
    /// click arms the action; the second dispatches it.
    pub(crate) fn revert_checkpoint(&mut self, turn_count: u64, cx: &mut Context<Self>) {
        let Some(id) = self.selected_thread.clone() else {
            return;
        };
        if self.confirm_revert != Some(turn_count) {
            self.confirm_revert = Some(turn_count);
            self.set_status(
                "Click again to restore the working tree to this checkpoint",
                cx,
            );
            cx.notify();
            return;
        }
        self.confirm_revert = None;
        let payload = commands::revert_command(&id, turn_count);
        self.dispatch(payload, cx);
    }
}
