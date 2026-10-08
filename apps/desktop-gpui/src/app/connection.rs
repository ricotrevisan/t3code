//! Connecting, pairing, saved credentials, and server requests.

use crate::{InspectorSection, NativeApp};
use gpui_kit::{Context, Window};
use serde_json::{Value, json};
use t3code_desktop_gpui::{
    auth, connection, credentials, files, shell, sidebar, thread, workspaces,
};

impl NativeApp {
    /// Retain a cancellable send when the bounded queue is full. Replacing or
    /// unsubscribing the same key retires its old send; disconnect retires all.
    pub(crate) fn enqueue_subscription(
        &mut self,
        command: connection::Command,
        cx: &mut Context<Self>,
    ) {
        let key = match &command {
            connection::Command::Subscribe { key, .. } | connection::Command::Unsubscribe(key) => {
                key.clone()
            }
            _ => unreachable!("subscription control only"),
        };
        self.subscription_enqueues.remove(&key);
        let Some(sender) = self.commands.clone() else {
            self.report_error("Subscription not requested: not connected", cx);
            return;
        };
        match sender.try_send(command) {
            Ok(()) => {}
            Err(async_channel::TrySendError::Closed(_)) => {
                self.report_error("Subscription not requested: connection closed", cx)
            }
            Err(async_channel::TrySendError::Full(command)) => {
                let generation = self.generation;
                let scope = self.environment_scope.clone();
                let task = cx.spawn(async move |entity, cx| {
                    let result = sender.send(command).await;
                    let _ = entity.update(cx, |app, cx| {
                        if app.generation != generation || app.environment_scope != scope {
                            return;
                        }
                        if result.is_err() {
                            app.report_error("Subscription not requested: connection closed", cx);
                        }
                        cx.notify();
                    });
                });
                self.subscription_enqueues.insert(key, task);
            }
        }
    }

    pub(crate) fn disconnect(&mut self, cx: &mut Context<Self>) {
        self.subscription_enqueues.clear();
        self.auth_state = None;
        self.clear_checkout_stops();
        self.clear_queue_subscriptions();
        self.diffs.invalidate();
        self.action_runs_connection_lost();
        self.clear_notifications(cx);
        self.connection_task = None;
        self.environment_identity_verified = false;
        if let Some(id) = self.sessions.selected_id().map(str::to_owned) {
            self.sessions.remove(&id);
        }
        self.pairing_task = None;
        self.credential_task = None;
        self.commands = None;
        self.connection_phase = connection::Phase::Offline;
        self.environment_name = None;
        self.action_task = None;
        self.action_busy = false;
        self.project_settings = None;
        self.action_edit_id = None;
        self.confirm_action_delete = None;
        self.shell = shell::Shell::default();
        self.archived_shell = shell::Shell::default();
        self.detail = thread::ThreadDetail::default();
        self.stash_attachment_draft();
        self.selected_thread = None;
        self.restore_attachment_draft();
        self.pending_history_attached = false;
        self.selected_project = None;
        self.confirm_project_removal = None;
        self.pending_actions = 0;
        self.provider_catalog.clear();
        self.clone_progress.clear();
        self.file_epoch += 1;
        self.file_busy = false;
        self.file_listing_task = None;
        self.file_entries.clear();
        self.content_results = None;
        self.content_index = 0;
        self.file_listing_epoch += 1;
        if !self.file_editor.dirty(&self.file_contents.read(cx).value()) {
            self.file_editor = files::Editor::default();
        }
        self.snooze_wake = None;
        self.worktree_task = None;
        self.worktree_busy = false;
        self.worktree_epoch += 1;
        self.refs.clear();
        self.refs_cursor = None;
        self.worktree_outcome = None;
        self.new_thread_checkout = None;
        self.generation += 1;
        self.model_override = None;
        self.set_status("Disconnected", cx);
        cx.notify();
    }

    pub(crate) fn pair(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.disconnect(cx);
        let server = self.server.read(cx).value().to_string();
        let credential = self.pairing.read(cx).value().to_string();
        self.pairing
            .update(cx, |input, cx| input.set_value("", window, cx));
        let receiver = auth::pair(server, credential);
        let generation = self.generation;
        self.apply_connection_phase(connection::Phase::Authenticating, cx);
        self.report_progress("Pairing…", cx);
        self.pairing_task = Some(cx.spawn(async move |entity, cx| {
            if let Ok(result) = receiver.recv().await {
                let _ = entity.update(cx, |app, cx| {
                    if app.generation != generation {
                        return;
                    }
                    match result {
                        Ok(access)
                            if auth::origin(&app.server.read(cx).value()).ok().as_ref()
                                == Some(&access.origin) =>
                        {
                            app.access = Some(access);
                            app.connect(cx);
                        }
                        Ok(_) => app.apply_connection_phase(
                            connection::Phase::Error(
                                "Server changed while pairing; pair again".into(),
                            ),
                            cx,
                        ),
                        Err(message) => {
                            app.apply_connection_phase(connection::Phase::Error(message.into()), cx)
                        }
                    }
                    cx.notify();
                });
            }
        }));
        cx.notify();
    }

    pub(crate) fn credential_operation(
        &mut self,
        operation: credentials::Operation,
        cx: &mut Context<Self>,
    ) {
        let loading = matches!(operation, credentials::Operation::Load);
        if loading {
            self.disconnect(cx);
        }
        let server = self.server.read(cx).value().to_string();
        let origin = match auth::origin(&server) {
            Ok(origin) => origin,
            Err(error) => {
                self.report_error(error, cx);
                cx.notify();
                return;
            }
        };
        let receiver = match self.vault.request(&server, operation) {
            Ok(receiver) => receiver,
            Err(error) => {
                self.report_error(error, cx);
                cx.notify();
                return;
            }
        };
        let generation = self.generation;
        self.report_progress("Updating Keychain…", cx);
        self.credential_task = Some(cx.spawn(async move |entity, cx| {
            if let Ok(result) = receiver.recv().await {
                let _ = entity.update(cx, |app, cx| {
                    if generation != app.generation
                        || auth::origin(&app.server.read(cx).value()).ok().as_ref() != Some(&origin)
                    {
                        return;
                    }
                    match result {
                        Ok(Some(token)) if loading => {
                            match auth::Access::from_keychain(origin, token) {
                                Ok(access) => {
                                    app.access = Some(access);
                                    app.connect(cx);
                                }
                                Err(error) => app.report_error(error, cx),
                            }
                        }
                        Ok(None) if loading => app.set_status(
                            "No remembered credential for this server. Pair to connect.",
                            cx,
                        ),
                        Ok(_) => app.report_success("Keychain updated", cx),
                        Err(error) => app.report_error(error, cx),
                    }
                    cx.notify();
                });
            }
        }));
        cx.notify();
    }

    pub(crate) fn remember_credential(&mut self, cx: &mut Context<Self>) {
        let origin = auth::origin(&self.server.read(cx).value()).ok();
        if let Some(access) = self
            .access
            .as_ref()
            .filter(|access| Some(&access.origin) == origin.as_ref())
        {
            self.credential_operation(
                credentials::Operation::Remember(access.keychain_value()),
                cx,
            );
        } else {
            self.set_status(
                "Pair with this server before remembering its credential",
                cx,
            );
            cx.notify();
        }
    }

    pub(crate) fn forget_credential(&mut self, cx: &mut Context<Self>) {
        self.disconnect(cx);
        self.access = None;
        self.use_dev_credential = false;
        self.credential_operation(credentials::Operation::Forget, cx);
    }

    pub(crate) fn connect(&mut self, cx: &mut Context<Self>) {
        self.disconnect(cx);
        self.set_status("Enter the server origin and pair to connect", cx);
        let base = self.server.read(cx).value().to_string();
        let origin = auth::origin(&base).ok();
        let token = self
            .access
            .as_ref()
            .filter(|access| Some(&access.origin) == origin.as_ref())
            .map(|access| access.token.clone())
            .or_else(|| {
                if self.use_dev_credential
                    && std::env::var("T3_NATIVE_SERVER")
                        .ok()
                        .and_then(|base| auth::origin(&base).ok())
                        == origin
                {
                    std::env::var("T3_NATIVE_ACCESS_TOKEN").ok()
                } else {
                    None
                }
            });
        if let Some(token) = token {
            match connection::ConnectionOptions::new(&base, token) {
                Ok(options) => {
                    let options = options.with_proof_key(
                        self.access
                            .as_ref()
                            .filter(|access| Some(&access.origin) == origin.as_ref())
                            .and_then(|access| access.proof_key.clone()),
                    );
                    let scope = options.origin();
                    if scope != self.environment_scope {
                        self.sidebar_state.capabilities = json!({});
                        self.sidebar_state.custom_snooze_target = None;
                        self.sidebar_state.rename_target = None;
                        self.sidebar_state.selection.clear();
                        self.queues = Default::default();
                        self.drafts.clear();
                        self.stash_memory.clear();
                        self.search_results.clear();
                    }
                    self.pending_threads = self
                        .draft_store
                        .as_ref()
                        .map(|store| store.pending_threads(&scope))
                        .unwrap_or_default();
                    self.stash_attachment_draft();
                    self.environment_scope = scope;
                    self.restore_attachment_draft();
                    let session = connection::start_session(options);
                    self.start_owned_session(session, cx);
                }
                Err(message) => {
                    self.apply_connection_phase(connection::Phase::Error(message.into()), cx)
                }
            }
        }
        cx.notify();
    }

    pub(crate) fn apply_connection_update(
        &mut self,
        update: connection::Update,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        match update {
            connection::Update::Phase(phase) => {
                self.apply_connection_phase(phase, cx);
                Ok(())
            }
            connection::Update::Resynchronizing => {
                self.connection_phase = connection::Phase::Syncing;
                self.action_runs_connection_lost();
                self.clear_checkout_stops();
                self.clear_queue_subscriptions();
                self.clear_notifications(cx);
                self.shell = shell::Shell::default();
                self.archived_shell = shell::Shell::default();
                self.action_task = None;
                self.action_busy = false;
                self.project_settings = None;
                self.action_edit_id = None;
                self.confirm_action_delete = None;
                self.detail = thread::ThreadDetail::default();
                self.snooze_wake = None;
                self.clone_progress.clear();
                self.setup_progress = None;
                self.pending_history_attached = false;
                self.report_progress("Reconnecting — history is being synchronized", cx);
                Ok(())
            }
            connection::Update::Error(error) => {
                self.report_error(error, cx);
                Ok(())
            }
            connection::Update::Status(status) => {
                self.set_status(status, cx);
                Ok(())
            }
            connection::Update::Shell(item) => {
                let was_synchronized = self.shell.synchronized;
                let result = self.apply_shell_item(*item, cx).map_err(str::to_owned);
                self.flush_queues(cx);
                self.attach_created_pending(cx);
                self.schedule_snooze_wake(cx);
                if self.shell.synchronized && !was_synchronized {
                    self.action_runs_synchronized(cx);
                    self.perform("server.getConfig", json!({}), None, cx);
                    self.enqueue_subscription(
                        connection::Command::Subscribe {
                            key: "server-config".into(),
                            tag: "subscribeServerConfig".into(),
                            payload: json!({"usageLimitSources":true}),
                        },
                        cx,
                    );
                    if self.inspector_section == Some(InspectorSection::Clone) {
                        self.enqueue_subscription(
                            connection::Command::Subscribe {
                                key: "project-clones".into(),
                                tag: "subscribeProjectClones".into(),
                                payload: json!({}),
                            },
                            cx,
                        );
                    }
                }
                if self.shell.synchronized {
                    self.set_status("Connected · synchronized", cx);
                }
                result
            }
            connection::Update::Subscription { key, item } => {
                if key.starts_with("header-vcs:") {
                    self.apply_git_status(&key, item, cx);
                    Ok(())
                } else if key.starts_with("checkout-stop-") {
                    self.apply_checkout_stop_subscription(&key, item);
                    Ok(())
                } else if key.starts_with("queue-detail:") {
                    self.apply_queue_subscription(&key, item, cx)
                } else if key.starts_with("drawer-") {
                    self.terminal_update(&key, item, cx);
                    Ok(())
                } else if key.starts_with("action-") {
                    self.action_run_update(&key, item, cx);
                    Ok(())
                } else if let Some(instance) = key.strip_prefix("auth:") {
                    if self
                        .auth_state
                        .as_ref()
                        .is_some_and(|state| state["instanceId"] == instance)
                    {
                        match item {
                            Ok(value) if value["instanceId"] == instance => {
                                self.accept_provider_auth(value, false, cx)
                            }
                            Ok(_) => self.report_error("Invalid authentication state", cx),
                            Err(error) => self.report_error(error, cx),
                        }
                    }
                    Ok(())
                } else if key == "server-config" {
                    match item {
                        Ok(value) => {
                            self.apply_server_config_event(value, cx);
                            Ok(())
                        }
                        Err(error) => {
                            self.report_error(error, cx);
                            Ok(())
                        }
                    }
                } else if key == "project-clones"
                    && self.inspector_section == Some(InspectorSection::Clone)
                {
                    match item {
                        Ok(value) => {
                            let running: Vec<String> = self
                                .clone_progress
                                .iter()
                                .filter(|clone| clone.phase == "running")
                                .map(|clone| clone.project_id.clone())
                                .collect();
                            let result = workspaces::apply_clones(&mut self.clone_progress, value);
                            if self.clone_progress.iter().any(|clone| {
                                clone.phase == "done" && running.contains(&clone.project_id)
                            }) {
                                self.report_success("Repository cloned", cx);
                            }
                            result
                        }
                        Err(error) => {
                            self.report_error(error, cx);
                            self.clone_progress.clear();
                            Ok(())
                        }
                    }
                } else if key == "worktree-setup" {
                    match item {
                        Ok(value) => {
                            if let Some(id) = &self.selected_thread {
                                workspaces::apply_setup(&mut self.setup_progress, id, value)
                            } else {
                                Ok(())
                            }
                        }
                        Err(error) => {
                            self.report_error(error, cx);
                            Ok(())
                        }
                    }
                } else {
                    Ok(())
                }
            }
            connection::Update::Thread { thread_id, item } => {
                if self.selected_thread.as_ref() == Some(&thread_id) {
                    self.apply_thread_update(&thread_id, item, cx)
                } else {
                    Ok(())
                }
            }
        }
    }

    pub(crate) fn perform(
        &mut self,
        tag: &str,
        payload: Value,
        sent_prompt: Option<(String, gpui_kit::AnyWindowHandle)>,
        cx: &mut Context<Self>,
    ) {
        // Initial configuration is part of synchronization. Every user action
        // must wait for the owned shell projection, not merely a live sender.
        if tag != "server.getConfig" && !self.connection_ready() {
            self.report_error("Action unavailable while disconnected or synchronizing", cx);
            return;
        }
        let Some(sender) = self.commands.clone() else {
            self.report_error("Not connected", cx);
            cx.notify();
            return;
        };
        let generation = self.generation;
        let thread_id = self.selected_thread.clone();
        let tag = tag.to_owned();
        let attachment_origin = self.environment_scope.clone();
        let attachment_thread = payload["threadId"].as_str().map(str::to_owned);
        let sent_attachments = if sent_prompt.is_some() {
            payload["message"]["attachments"]
                .as_array()
                .cloned()
                .unwrap_or_default()
        } else {
            Vec::new()
        };
        let sent_context = sent_prompt
            .as_ref()
            .and_then(|_| payload["message"].get("context").cloned());
        let reverting = payload["type"] == "thread.checkpoint.revert";
        self.pending_actions += 1;
        self.report_progress("Waiting for server receipt…", cx);
        cx.spawn(async move |entity, cx| {
            let result = connection::call(&sender, &tag, payload).await;
            if result.is_ok()
                && let Some(id) = &attachment_thread
            {
                let _ = entity.update(cx, |app, cx| {
                    app.consume_sent_attachments(&attachment_origin, id, &sent_attachments);
                    if let Some(context) = &sent_context {
                        app.consume_sent_context(&attachment_origin, id, context);
                    }
                    app.schedule_draft_save(cx);
                    cx.notify();
                });
            }
            if result.is_ok()
                && let Some((text, handle)) = &sent_prompt
            {
                let _ = handle.update(cx, |_, window, cx| {
                    entity.update(cx, |app, cx| {
                        if app.generation == generation
                            && app.selected_thread == thread_id
                            && app.composer.read(cx).value().as_str() == text
                        {
                            app.composer
                                .update(cx, |input, cx| input.set_value("", window, cx));
                        }
                    })
                });
            }
            let _ = entity.update(cx, |app, cx| {
                if app.generation != generation {
                    return;
                }
                app.pending_actions = app.pending_actions.saturating_sub(1);
                match result {
                    Ok(value) => {
                        if sent_prompt.is_some()
                            && let Some(id) = &thread_id
                            && app
                                .pending_threads
                                .iter()
                                .any(|draft| &draft.thread.id == id)
                        {
                            app.pending_threads.retain(|draft| &draft.thread.id != id);
                            if let Some(store) = &mut app.draft_store {
                                store.remove_pending(&app.environment_scope, id);
                            }
                            app.schedule_draft_save(cx);
                            if app.selected_thread.as_ref() == Some(id) {
                                let _ = app.send_thread_selection(Some(id.clone()));
                            }
                        }
                        if tag == "projectClone.start" {
                            app.selected_project = value["projectId"].as_str().map(str::to_owned);
                        }
                        if tag == "server.getSettings" {
                            app.project_settings = Some(value.clone());
                        }
                        if tag == "server.getConfig" {
                            app.accept_config_snapshot(value.clone(), cx);
                            app.available_editors = value["availableEditors"]
                                .as_array()
                                .map(|editors| {
                                    editors
                                        .iter()
                                        .filter_map(|editor| editor.as_str().map(str::to_owned))
                                        .collect()
                                })
                                .unwrap_or_default();
                        }
                        if matches!(
                            tag.as_str(),
                            "provider.auth.start"
                                | "provider.auth.complete"
                                | "provider.auth.cancel"
                        ) {
                            app.accept_provider_auth(
                                value.clone(),
                                tag == "provider.auth.start",
                                cx,
                            );
                        }
                        if tag == "orchestration.getArchivedShellSnapshot" {
                            match serde_json::from_value::<shell::Snapshot>(value.clone()) {
                                Ok(snapshot) => {
                                    let _ = app
                                        .archived_shell
                                        .apply(shell::ShellItem::Snapshot { snapshot });
                                }
                                Err(_) => {
                                    app.report_error("Invalid archive snapshot", cx);
                                    cx.notify();
                                    return;
                                }
                            }
                        }
                        if tag == "orchestration.dispatchCommand"
                            && app.sidebar_view == sidebar::View::Archived
                        {
                            app.perform(
                                "orchestration.getArchivedShellSnapshot",
                                json!({}),
                                None,
                                cx,
                            );
                        }
                        if tag == "orchestration.searchThreads" {
                            app.search_results =
                                value["matches"].as_array().cloned().unwrap_or_default();
                        }
                        if let Some((text, _)) = sent_prompt
                            && let Some(id) = thread_id
                            && app.drafts.get(&id).is_some_and(|draft| draft == &text)
                        {
                            app.drafts.remove(&id);
                            if let Some(store) = &mut app.draft_store {
                                store.set(&app.environment_scope, &id, String::new());
                                app.schedule_draft_save(cx);
                            }
                        }
                        app.set_status(
                            if tag == "server.getConfig" {
                                format!(
                                    "Connected · provider catalog loaded ({} instances)",
                                    app.provider_catalog.len()
                                )
                            } else {
                                format!(
                                    "Server accepted · sequence {}",
                                    value["sequence"]
                                        .as_u64()
                                        .map(|s| s.to_string())
                                        .unwrap_or_else(|| "—".into())
                                )
                            },
                            cx,
                        );
                        if reverting {
                            app.report_success("Checkpoint restored", cx);
                        }
                    }
                    Err(error) => app.report_error(error, cx),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    pub(crate) fn dispatch(&mut self, payload: Value, cx: &mut Context<Self>) {
        self.perform("orchestration.dispatchCommand", payload, None, cx);
    }
}
