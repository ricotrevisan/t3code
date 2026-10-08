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
        let checkout = self.checkout_context();
        let branch = self.selected_thread_branch();
        self.detail.apply(thread_id, item)?;
        if self.selected_thread.as_deref() == Some(thread_id) {
            self.sidebar_state.read_at.insert(
                (self.environment_scope.clone(), thread_id.into()),
                chrono::Utc::now(),
            );
        }
        if self.checkout_context() != checkout || self.selected_thread_branch() != branch {
            self.invalidate_checkout_context(cx);
            self.set_status(
                "Checkout changed on the server; reload files in the selected checkout",
                cx,
            );
        }
        self.flush_queues(cx);
        Ok(())
    }

    pub(crate) fn turn_running(&self) -> bool {
        // The shell already knows the phase while the selected detail loads.
        let status = if let Some(thread) = self
            .detail
            .thread
            .as_ref()
            .filter(|thread| thread["id"].as_str() == self.selected_thread.as_deref())
        {
            thread["session"]["status"].as_str()
        } else {
            self.shell
                .threads
                .iter()
                .find(|thread| Some(&thread.id) == self.selected_thread.as_ref())
                .and_then(|thread| thread.session.as_ref())
                .map(|session| session.status.as_str())
        };
        matches!(status, Some("starting" | "running"))
    }

    /// Selects the previous or next thread in the order the sidebar shows them.
    pub(crate) fn step_thread(
        &mut self,
        forward: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let order = self.sidebar_selectable_order();
        if let Some(id) = sidebar::neighbor(&order, self.selected_thread.as_deref(), forward) {
            self.select_thread(&id, window, cx);
        }
    }

    pub(crate) fn new_thread(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.can_change_checkout(cx) {
            return;
        }
        let Some(project) = self
            .shell
            .projects
            .iter()
            .find(|project| Some(&project.id) == self.selected_project.as_ref())
        else {
            self.report_error("Select a project first", cx);
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
                    self.report_error(error, cx);
                    cx.notify();
                    return;
                }
            }
        };
        let mut command = match commands::create_thread(&project_id, "New thread", selection) {
            Ok(value) => value,
            Err(error) => {
                self.report_error(error, cx);
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
                self.report_error("Select a base branch for the new worktree", cx);
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
        if self.selected_thread.as_deref() != Some(id) && !self.can_change_checkout(cx) {
            return;
        }
        if let Some(thread) = self.shell.threads.iter().find(|thread| thread.id == id) {
            self.sidebar_state.visit(&self.environment_scope, thread);
            self.collapsed_projects
                .retain(|project| project != &thread.project_id);
            self.collapsed_groups.retain(|key| {
                key != &sidebar::group_key(
                    &thread.project_id,
                    if thread.pinned_at.is_some() {
                        "pinned"
                    } else {
                        "active"
                    },
                )
            });
            if self
                .sidebar_state
                .project_scope
                .as_ref()
                .is_some_and(|scope| scope != &thread.project_id)
            {
                self.sidebar_state.project_scope = None;
            }
        }
        if let Err(error) = self.sidebar_state.save_local() {
            self.report_error(error, cx);
        }
        if self.selected_thread.as_deref() == Some(id) {
            self.clear_thread_required(cx);
            self.reveal_sidebar_selection();
            self.sidebar_state.scroll_selected(window, cx);
            cx.notify();
            return;
        }
        self.sidebar_state.message_anchor = None;
        self.schedule_sidebar_clock(cx);
        self.reset_checkout_context(window, cx);
        if let Some(previous) = &self.selected_thread {
            self.drafts
                .insert(previous.clone(), self.composer.read(cx).value().to_string());
        }
        self.model_override = None;
        self.stash_attachment_draft();
        self.selected_thread = Some(id.into());
        self.clear_thread_required(cx);
        self.reveal_sidebar_selection();
        self.sidebar_state.scroll_selected(window, cx);
        self.restore_attachment_draft();
        self.mention = None;
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
        self.composer.update(cx, |input, cx| {
            input.set_value(crate::composer_state::prompt_content(&text), window, cx)
        });
        if let Some(thread) = self
            .shell
            .threads
            .iter()
            .chain(&self.archived_shell.threads)
            .chain(self.pending_threads.iter().map(|draft| &draft.thread))
            .find(|t| t.id == id)
        {
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
        self.sync_checkout_recipe_controls(window, cx);
        self.setup_progress = None;
        self.enqueue_subscription(
            connection::Command::Subscribe {
                key: "worktree-setup".into(),
                tag: "subscribeWorktreeSetup".into(),
                payload: json!({"threadId":id}),
            },
            cx,
        );
        if self.commands.is_some()
            && self.send_thread_selection(if pending { None } else { Some(id.into()) }).is_err()
        {
            self.report_error("Connection busy or closed; retry selection", cx);
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
            let _ = self.send_thread_selection(None);
            cx.notify();
            return;
        }
        if self.pending_history_attached || !materialized {
            return;
        }
        let id = id.clone();
        if self.commands.is_some() && self.send_thread_selection(Some(id)).is_ok()
        {
            self.pending_history_attached = true;
            cx.notify();
        }
    }

    pub(crate) fn discard_pending(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.can_change_checkout(cx) {
            return;
        }
        let Some(id) = self.selected_thread.clone() else {
            return;
        };
        if self.confirm_delete.as_ref() != Some(&id) {
            self.confirm_delete = Some(id);
            self.set_status("Click Delete again to discard this unsent thread", cx);
            cx.notify();
            return;
        }
        self.reset_checkout_context(window, cx);
        self.pending_threads.retain(|draft| draft.thread.id != id);
        self.drafts.remove(&id);
        if let Some(store) = &mut self.draft_store {
            store.remove_pending(&self.environment_scope, &id);
            store.set(&self.environment_scope, &id, String::new());
        }
        self.stash_attachment_draft();
        self.selected_thread = None;
        self.restore_attachment_draft();
        self.confirm_delete = None;
        self.composer
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.setup_progress = None;
        self.enqueue_subscription(
            connection::Command::Unsubscribe("worktree-setup".into()),
            cx,
        );
        self.schedule_draft_save(cx);
        self.report_success("Unsent thread discarded", cx);
        cx.notify();
    }

    /// Runs a thread command on the selected thread.
    pub(crate) fn lifecycle(&mut self, kind: &str, cx: &mut Context<Self>) {
        if let Some(id) = self.selected_thread.clone() {
            self.thread_lifecycle(&id, kind, cx);
        }
    }

    pub(crate) fn thread_lifecycle(&mut self, id: &str, kind: &str, cx: &mut Context<Self>) {
        let materialized = self.shell.threads.iter().any(|thread| thread.id == id);
        if !materialized
            && let Some(draft) = self
                .pending_threads
                .iter_mut()
                .find(|draft| draft.thread.id == id)
        {
            if kind == "thread.meta.update" {
                let title = self.title.read(cx).value().trim().to_owned();
                if title.is_empty() {
                    self.report_error("A title is required", cx);
                    cx.notify();
                    return;
                }
                draft.thread.title = title;
                if let Some(store) = &mut self.draft_store {
                    store.put_pending(&self.environment_scope, draft.clone());
                }
                self.schedule_draft_save(cx);
                self.report_success("Draft renamed", cx);
            } else {
                self.report_error("Send the first message before changing thread status", cx);
            }
            cx.notify();
            return;
        }
        if matches!(kind, "thread.delete" | "thread.archive")
            && self.selected_thread.as_deref() == Some(id)
            && !self.can_change_checkout(cx)
        {
            return;
        }
        let mut payload = commands::command(kind, id);
        match kind {
            "thread.meta.update" => {
                let title = self.title.read(cx).value();
                if title.trim().is_empty() {
                    self.report_error("A title is required", cx);
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
        self.dispatch_navigation(payload, cx);
    }

    /// One coarse tick, only while the sidebar has visible time labels. This
    /// task is retained separately from snooze expiry and never drives frames.
    pub(crate) fn schedule_sidebar_clock(&mut self, cx: &mut Context<Self>) {
        self.sidebar_clock = Some(cx.spawn(async move |entity, cx| {
            loop {
                cx.background_executor()
                    .timer(std::time::Duration::from_secs(60))
                    .await;
                let keep = entity
                    .update(cx, |app, cx| {
                        if !app.sidebar_open {
                            return true;
                        }
                        let now = chrono::Utc::now();
                        let previous = now - chrono::Duration::minutes(1);
                        let visible: std::collections::HashSet<_> =
                            app.sidebar_selectable_order().into_iter().collect();
                        let changed = app
                            .shell
                            .threads
                            .iter()
                            .chain(&app.archived_shell.threads)
                            .any(|thread| {
                                visible.contains(&thread.id)
                                    && sidebar::row_time(thread, now)
                                        != sidebar::row_time(thread, previous)
                            });
                        if changed {
                            cx.notify();
                        }
                        true
                    })
                    .unwrap_or(false);
                if !keep {
                    break;
                }
            }
        }));
    }

    pub(crate) fn sidebar_menu_action(
        &mut self,
        target: &str,
        kind: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(until) = kind.strip_prefix("bulk-snooze-until:") {
            self.bulk_thread_lifecycle_until("snooze", "thread.snooze", Some(until.into()), cx);
            return;
        }
        if kind == "bulk-snooze-custom" {
            self.sidebar_state.bulk_snooze_review = Some(self.sidebar_state.selection.clone());
            self.sidebar_state.custom_snooze_generation = Some(self.generation);
            self.sidebar_state.custom_snooze_environment = Some(self.environment_scope.clone());
            self.sidebar_state.custom_snooze_target = Some(target.into());
            self.close_thread_menu();
            self.custom_snooze
                .update(cx, |input, cx| input.focus(window, cx));
            cx.notify();
            return;
        }
        if kind == "bulk-mark-unread" {
            self.reconcile_sidebar_selection();
            for id in self.sidebar_state.selection.clone() {
                self.sidebar_state
                    .marked_unread
                    .insert((self.environment_scope.clone(), id));
            }
            self.sidebar_state.selection.clear();
            if let Err(error) = self.sidebar_state.save_local() {
                self.report_error(error, cx);
            }
            self.close_thread_menu();
            cx.notify();
            return;
        }
        if let Some(bulk) = kind.strip_prefix("bulk:") {
            if let Some((action, kind)) = bulk.split_once(':') {
                self.bulk_thread_lifecycle(action, kind, cx);
            }
            return;
        }
        match kind {
            "thread.meta.update" if self.thread_menu_row.is_some() => {
                self.sidebar_state.project_settings_target = None;
                if let Some(thread) = self
                    .shell
                    .threads
                    .iter()
                    .chain(&self.archived_shell.threads)
                    .chain(self.pending_threads.iter().map(|draft| &draft.thread))
                    .find(|thread| thread.id == target)
                {
                    let title = thread.title.clone();
                    self.sidebar_state.rename_target =
                        Some((self.environment_scope.clone(), target.into()));
                    self.sidebar_rename.update(cx, |input, cx| {
                        input.set_value(title, window, cx);
                        input.focus(window, cx);
                    });
                }
                self.close_thread_menu();
            }
            "discard" => {
                if self.selected_thread.as_deref() == Some(target) {
                    self.discard_pending(window, cx);
                } else if self.confirm_delete.as_deref() != Some(target) {
                    self.confirm_delete = Some(target.into());
                    self.set_status("Click Discard again to remove this unsent thread", cx);
                } else {
                    self.pending_threads
                        .retain(|draft| draft.thread.id != target);
                    self.drafts.remove(target);
                    if let Some(store) = &mut self.draft_store {
                        store.remove_pending(&self.environment_scope, target);
                        store.set(&self.environment_scope, target, String::new());
                    }
                    self.confirm_delete = None;
                    self.schedule_draft_save(cx);
                    self.report_success("Unsent thread discarded", cx);
                }
                if self.confirm_delete.is_none() {
                    self.close_thread_menu();
                }
            }
            "move-up" | "move-down" => {
                self.move_sidebar_thread(target, kind == "move-down", cx);
                self.close_thread_menu();
            }
            "mark-unread" => {
                self.sidebar_state
                    .marked_unread
                    .insert((self.environment_scope.clone(), target.into()));
                if let Err(error) = self.sidebar_state.save_local() {
                    self.report_error(error, cx);
                }
                self.close_thread_menu();
            }
            "filter-project" => {
                if let Some(thread) = self
                    .shell
                    .threads
                    .iter()
                    .chain(&self.archived_shell.threads)
                    .find(|thread| thread.id == target)
                {
                    let project = thread.project_id.clone();
                    self.sidebar_state.project_scope =
                        if self.sidebar_state.project_scope.as_ref() == Some(&project) {
                            None
                        } else {
                            Some(project)
                        };
                    self.collapsed_projects
                        .retain(|id| id != &thread.project_id);
                }
                self.reconcile_sidebar_selection();
                self.close_thread_menu();
            }
            "snooze-custom" => {
                self.sidebar_state.bulk_snooze_review = None;
                self.sidebar_state.custom_snooze_generation = Some(self.generation);
                self.sidebar_state.custom_snooze_target = Some(target.into());
                self.sidebar_state.custom_snooze_environment = Some(self.environment_scope.clone());
                self.close_thread_menu();
            }
            "project-settings" => {
                if let Some(project) = self
                    .shell
                    .threads
                    .iter()
                    .chain(&self.archived_shell.threads)
                    .find(|thread| thread.id == target)
                    .and_then(|thread| {
                        self.shell
                            .projects
                            .iter()
                            .find(|project| project.id == thread.project_id)
                    })
                    .cloned()
                {
                    self.sidebar_state.rename_target = None;
                    self.sidebar_state.project_settings_target =
                        Some((self.environment_scope.clone(), project.id));
                    self.sidebar_rename
                        .update(cx, |input, cx| input.set_value(project.title, window, cx));
                }
                self.close_thread_menu();
            }
            "cleanup-review" => {
                self.sidebar_state.cleanup.target =
                    Some((self.environment_scope.clone(), target.into()));
                self.review_sidebar_cleanup(false, cx);
                self.close_thread_menu();
            }
            "auto-settle-enable" | "auto-settle-disable" => {
                if self.sidebar_state.capabilities["threadAutoSettleOptOut"].as_bool() != Some(true)
                {
                    self.report_error("This server does not support per-thread auto-settle", cx);
                    return;
                }
                let mut command = commands::command("thread.auto-settle.set", target);
                command["enabled"] = json!(kind == "auto-settle-enable");
                self.dispatch(command, cx);
                self.close_thread_menu();
            }
            "regenerate-title" => {
                let mut command = commands::command("thread.meta.update", target);
                command["regenerateTitle"] = json!(true);
                self.dispatch(command, cx);
                self.close_thread_menu();
            }
            "new-thread-on-branch" => {
                if !self.can_change_checkout(cx) {
                    return;
                }
                if let Some(thread) = self
                    .shell
                    .threads
                    .iter()
                    .chain(&self.archived_shell.threads)
                    .find(|thread| thread.id == target)
                    .cloned()
                    && let Some(branch) = thread.branch
                {
                    self.selected_project = Some(thread.project_id.clone());
                    self.new_thread_checkout =
                        Some((thread.project_id, branch, thread.worktree_path));
                    self.prepare_new_worktree = false;
                    self.new_thread(window, cx);
                }
                self.close_thread_menu();
            }
            "copy-path" | "copy-branch" | "copy-thread-id" => {
                if let Some(thread) = self
                    .shell
                    .threads
                    .iter()
                    .chain(&self.archived_shell.threads)
                    .find(|thread| thread.id == target)
                {
                    let value = match kind {
                        "copy-thread-id" => Some(thread.id.clone()),
                        "copy-branch" => thread.branch.clone(),
                        _ => thread.worktree_path.clone().or_else(|| {
                            self.shell
                                .projects
                                .iter()
                                .find(|project| project.id == thread.project_id)
                                .map(|project| project.workspace_root.clone())
                        }),
                    };
                    if let Some(value) = value {
                        cx.write_to_clipboard(gpui_kit::ClipboardItem::new_string(value));
                        self.report_success("Copied", cx);
                    }
                }
                self.close_thread_menu();
            }
            value if value.starts_with("snooze-until:") => {
                let mut command = commands::command("thread.snooze", target);
                command["snoozedUntil"] = json!(&value[13..]);
                self.dispatch_navigation(command, cx);
                self.close_thread_menu();
            }
            _ => self.run_thread_menu_action(kind, window, cx),
        }
        cx.notify();
    }

    pub(crate) fn rename_sidebar_target(&mut self, cx: &mut Context<Self>) {
        let Some((environment, id)) = self.sidebar_state.rename_target.clone() else {
            return;
        };
        if environment != self.environment_scope {
            self.sidebar_state.rename_target = None;
            return;
        }
        if self.sidebar_state.rename_busy == Some(self.generation) {
            return;
        }
        let title = self.sidebar_rename.read(cx).value().trim().to_owned();
        if title.is_empty() {
            self.report_error("A title is required", cx);
            return;
        }
        if !self.shell.threads.iter().any(|thread| thread.id == id)
            && let Some(draft) = self
                .pending_threads
                .iter_mut()
                .find(|draft| draft.thread.id == id)
        {
            draft.thread.title = title;
            if let Some(store) = &mut self.draft_store {
                store.put_pending(&self.environment_scope, draft.clone());
            }
            self.schedule_draft_save(cx);
            self.report_success("Draft renamed", cx);
            self.sidebar_state.rename_target = None;
        } else {
            let Some(sender) = self.commands.clone() else {
                self.report_error("Not connected; rename retained", cx);
                return;
            };
            let mut payload = commands::command("thread.meta.update", &id);
            payload["title"] = json!(title);
            let generation = self.generation;
            self.sidebar_state.rename_busy = Some(generation);
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
                        .unwrap_or_else(|_| Err("Rename reply lost".into()))
                };
                let _ = entity.update(cx, |app, cx| {
                    if app.generation != generation || app.environment_scope != environment {
                        return;
                    }
                    app.sidebar_state.rename_busy = None;
                    match result {
                        Ok(_) => {
                            if app
                                .sidebar_state
                                .rename_target
                                .as_ref()
                                .is_some_and(|target| target == &(environment.clone(), id.clone()))
                                && app.sidebar_rename.read(cx).value().trim() == title
                            {
                                app.sidebar_state.rename_target = None;
                            }
                            app.set_status("Thread renamed", cx);
                        }
                        Err(error) => {
                            app.report_error(format!("Rename failed; edit retained: {error}"), cx)
                        }
                    }
                    cx.notify();
                });
            })
            .detach();
        }
        cx.notify();
    }

    pub(crate) fn move_sidebar_thread(
        &mut self,
        target: &str,
        forward: bool,
        cx: &mut Context<Self>,
    ) {
        if !self.connection_ready()
            || (self.sidebar_state.order_busy
                && self.sidebar_state.order_generation == self.generation)
        {
            return;
        }
        let Some(thread) = self.shell.threads.iter().find(|thread| thread.id == target) else {
            return;
        };
        let pinned = thread.pinned_at.is_some();
        if self.sidebar_state.capabilities[if pinned {
            "threadPinReorder"
        } else {
            "threadActiveReorder"
        }]
        .as_bool()
            != Some(true)
            || !self
                .sidebar_selectable_order()
                .iter()
                .any(|id| id == target)
        {
            return;
        }
        let groups = self.sidebar_state.grouped(
            &self.shell.threads,
            Some(self.shell.revision),
            if self.sidebar_state.priority {
                self.sidebar_state.project_scope.as_deref().unwrap_or("")
            } else {
                &thread.project_id
            },
            sidebar::View::Active,
            chrono::Utc::now(),
            &self.environment_scope,
        );
        let rows = &groups[if pinned { 0 } else { 1 }].2;
        let Some(assignments) =
            sidebar::move_assignments(rows, &self.shell.threads, target, forward, pinned)
        else {
            return;
        };
        let Some(sender) = self.commands.clone() else {
            self.report_error("Not connected", cx);
            return;
        };
        let calls = assignments
            .into_iter()
            .map(|(id, key)| {
                let mut payload = commands::command(
                    if pinned {
                        "thread.pin.reorder"
                    } else {
                        "thread.active.reorder"
                    },
                    &id,
                );
                payload["orderKey"] = json!(key);
                payload
            })
            .collect::<Vec<_>>();
        let generation = self.generation;
        let environment = self.environment_scope.clone();
        self.sidebar_state.order_busy = true;
        self.sidebar_state.order_generation = generation;
        // No optimistic projection mutation: rejection leaves the displayed
        // order untouched; only authoritative shell receipts can rearrange it.
        cx.spawn(async move |entity, cx| {
            let mut result = Ok(json!({}));
            for payload in calls {
                if !entity
                    .read_with(cx, |app, _| {
                        app.generation == generation && app.environment_scope == environment
                    })
                    .unwrap_or(false)
                {
                    return;
                }
                result = connection::call(&sender, "orchestration.dispatchCommand", payload).await;
                if result.is_err() {
                    break;
                }
            }
            let _ = entity.update(cx, |app, cx| {
                if app.generation != generation || app.environment_scope != environment {
                    return;
                }
                app.sidebar_state.order_busy = false;
                if let Err(error) = result {
                    app.report_error(
                        format!(
                            "Order write rejected; showing server-authoritative state: {error}"
                        ),
                        cx,
                    );
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
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
        if self
            .sidebar_state
            .custom_snooze_generation
            .is_some_and(|generation| generation != self.generation)
        {
            self.sidebar_state.custom_snooze_target = None;
            self.sidebar_state.bulk_snooze_review = None;
            self.report_error("Connection changed; review snooze targets again", cx);
            return;
        }
        if self.sidebar_state.custom_snooze_target.is_some()
            && self.sidebar_state.custom_snooze_environment.as_deref()
                != Some(&self.environment_scope)
        {
            self.sidebar_state.custom_snooze_target = None;
            self.report_error("The snooze target belongs to a different environment", cx);
            return;
        }
        let target = self
            .sidebar_state
            .custom_snooze_target
            .clone()
            .or_else(|| self.selected_thread.clone());
        let Some(id) = target.as_deref() else {
            return;
        };
        let value = self.custom_snooze.read(cx).value();
        let until = match chrono::DateTime::parse_from_rfc3339(value.trim()) {
            Ok(until) if until > chrono::Utc::now() => until,
            _ => {
                self.report_error("Choose a future ISO date/time with timezone", cx);
                cx.notify();
                return;
            }
        };
        if let Some(review) = self.sidebar_state.bulk_snooze_review.take() {
            if review != self.sidebar_state.selection {
                self.report_error("Selection changed; review bulk snooze targets again", cx);
                self.sidebar_state.custom_snooze_target = None;
                return;
            }
            self.bulk_thread_lifecycle_until(
                "snooze",
                "thread.snooze",
                Some(
                    until
                        .with_timezone(&chrono::Utc)
                        .to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
                ),
                cx,
            );
            self.sidebar_state.custom_snooze_target = None;
            return;
        }
        let mut command = commands::command("thread.snooze", id);
        command["snoozedUntil"] = json!(
            until
                .with_timezone(&chrono::Utc)
                .to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
        );
        self.dispatch_navigation(command, cx);
        self.sidebar_state.custom_snooze_target = None;
    }

    pub(crate) fn search_threads(&mut self, cx: &mut Context<Self>) {
        let query = self.search.read(cx).value();
        if !(2..=200).contains(&query.trim().encode_utf16().count()) {
            self.report_error("Search needs 2–200 characters", cx);
            cx.notify();
            return;
        }
        let query = query.trim().to_owned();
        self.sidebar_state.search_epoch += 1;
        let epoch = self.sidebar_state.search_epoch;
        self.sidebar_state.search_query = query.clone();
        self.sidebar_state.search_environment = self.environment_scope.clone();
        self.sidebar_state.search_loading = true;
        self.sidebar_state.search_error = None;
        self.sidebar_state.search_index = 0;
        self.search_results.clear();
        let generation = self.generation;
        let environment = self.environment_scope.clone();
        let sender = self.commands.clone();
        cx.spawn(async move |entity, cx| {
            let result = if let Some(sender) = sender {
                let (reply, receipt) = async_channel::bounded(1);
                if sender
                    .send(connection::Command::Call {
                        tag: "orchestration.searchThreads".into(),
                        payload: json!({"query":query,"limit":50}),
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
                        .unwrap_or_else(|_| Err("Search reply lost".into()))
                }
            } else {
                Err("Not connected".into())
            };
            let _ = entity.update(cx, |app, cx| {
                if app.generation != generation
                    || app.environment_scope != environment
                    || app.sidebar_state.search_epoch != epoch
                {
                    return;
                }
                app.sidebar_state.search_loading = false;
                match result {
                    Ok(value) => {
                        if let Some(matches) = value["matches"].as_array() {
                            app.search_results = matches.clone();
                        } else {
                            app.sidebar_state.search_error = Some("Invalid search response".into());
                        }
                    }
                    Err(error) => app.sidebar_state.search_error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    pub(crate) fn step_search_match(&mut self, forward: bool, cx: &mut Context<Self>) {
        let len = self.search_results.len();
        if len > 0 {
            self.sidebar_state.search_index = if forward {
                (self.sidebar_state.search_index + 1) % len
            } else {
                (self.sidebar_state.search_index + len - 1) % len
            };
            cx.notify();
        }
    }

    pub(crate) fn thread_search_changed(&mut self, cx: &mut Context<Self>) {
        if self.search.read(cx).value().trim() != self.sidebar_state.search_query {
            self.sidebar_state.search_epoch += 1;
            self.sidebar_state.search_loading = false;
            self.sidebar_state.search_error = None;
            self.search_results.clear();
            cx.notify();
        }
    }

    pub(crate) fn clear_thread_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.sidebar_state.search_epoch += 1;
        self.sidebar_state.search_query.clear();
        self.sidebar_state.search_loading = false;
        self.sidebar_state.search_error = None;
        self.search_results.clear();
        self.search
            .update(cx, |input, cx| input.set_value("", window, cx));
        cx.notify();
    }

    pub(crate) fn select_search_match(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.sidebar_state.search_environment != self.environment_scope {
            return;
        }
        let Some(result) = self.search_results.get(index).cloned() else {
            return;
        };
        let Some(id) = result["threadId"].as_str() else {
            return;
        };
        self.select_thread(id, window, cx);
        if self.selected_thread.as_deref() != Some(id) {
            return;
        }
        self.sidebar_state.message_anchor =
            result["messageCreatedAt"]
                .as_str()
                .map(|at| sidebar::MessageAnchor {
                    environment: self.environment_scope.clone(),
                    thread_id: id.into(),
                    source: result["source"].as_str().unwrap_or("assistant").into(),
                    message_created_at: at.into(),
                });
        cx.notify();
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
