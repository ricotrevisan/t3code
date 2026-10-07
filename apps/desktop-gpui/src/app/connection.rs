//! Connecting, pairing, saved credentials, and server requests.

use crate::{InspectorSection, NativeApp};
use gpui_kit::{Context, Window};
use serde_json::{Value, json};
use t3code_desktop_gpui::{
    auth, connection, credentials, files, providers, shell, sidebar, thread, workspaces,
};

impl NativeApp {
    pub(crate) fn disconnect(&mut self, cx: &mut Context<Self>) {
        self.diffs.invalidate();
        self.connection_task = None;
        self.pairing_task = None;
        self.credential_task = None;
        self.commands = None;
        self.action_task = None;
        self.action_busy = false;
        self.project_settings = None;
        self.action_edit_id = None;
        self.confirm_action_delete = None;
        self.shell = shell::Shell::default();
        self.archived_shell = shell::Shell::default();
        self.detail = thread::ThreadDetail::default();
        self.selected_thread = None;
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
        self.set_status("Pairing…", cx);
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
                        Ok(_) => app.set_status("Server changed while pairing; pair again", cx),
                        Err(message) => app.set_status(message, cx),
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
                self.set_status(error, cx);
                cx.notify();
                return;
            }
        };
        let receiver = match self.vault.request(&server, operation) {
            Ok(receiver) => receiver,
            Err(error) => {
                self.set_status(error, cx);
                cx.notify();
                return;
            }
        };
        let generation = self.generation;
        self.set_status("Updating Keychain…", cx);
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
                                Err(error) => app.set_status(error, cx),
                            }
                        }
                        Ok(None) if loading => app.set_status(
                            "No remembered credential for this server. Pair to connect.",
                            cx,
                        ),
                        Ok(_) => app.set_status("Keychain updated", cx),
                        Err(error) => app.set_status(error, cx),
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
                        self.drafts.clear();
                        self.stash_memory.clear();
                        self.search_results.clear();
                    }
                    self.pending_threads = self
                        .draft_store
                        .as_ref()
                        .map(|store| store.pending_threads(&scope))
                        .unwrap_or_default();
                    self.environment_scope = scope;
                    let session = connection::start_session(options);
                    self.commands = Some(session.commands);
                    self.connection_task = Some(cx.spawn(async move |entity, cx| {
                        while let Ok(update) = session.updates.recv().await {
                            let keep_running = entity.update(cx, |app, cx| {
                                let result = match update {
                                    connection::Update::Resynchronizing => {
                                        app.shell = shell::Shell::default();
                                        app.archived_shell = shell::Shell::default();
                                        app.action_task = None;
                                        app.action_busy = false;
                                        app.project_settings = None;
                                        app.action_edit_id = None;
                                        app.confirm_action_delete = None;
                                        app.detail = thread::ThreadDetail::default();
                                        app.snooze_wake = None;
                                        app.clone_progress.clear();
                                        app.setup_progress = None;
                                        app.pending_history_attached = false;
                                        app.set_status(
                                            "Reconnecting — history is being synchronized",
                                            cx,
                                        );
                                        Ok(())
                                    }
                                    connection::Update::Status(status) => {
                                        app.set_status(status, cx);
                                        Ok(())
                                    }
                                    connection::Update::Shell(item) => {
                                        let was_synchronized = app.shell.synchronized;
                                        let result =
                                            app.apply_shell_item(*item, cx).map_err(str::to_owned);
                                        app.flush_queues(cx);
                                        app.attach_created_pending(cx);
                                        app.schedule_snooze_wake(cx);
                                        if app.shell.synchronized && !was_synchronized {
                                            app.perform("server.getConfig", json!({}), None, cx);
                                            if let Some(sender) = &app.commands {
                                                let _ = sender.try_send(
                                                    connection::Command::Subscribe {
                                                        key: "server-config".into(),
                                                        tag: "subscribeServerConfig".into(),
                                                        payload: json!({}),
                                                    },
                                                );
                                            }
                                            if app.inspector_section
                                                == Some(InspectorSection::Clone)
                                                && let Some(sender) = &app.commands
                                            {
                                                let _ = sender.try_send(
                                                    connection::Command::Subscribe {
                                                        key: "project-clones".into(),
                                                        tag: "subscribeProjectClones".into(),
                                                        payload: json!({}),
                                                    },
                                                );
                                            }
                                        }
                                        if app.shell.synchronized {
                                            app.set_status("Connected · synchronized", cx);
                                        }
                                        result
                                    }
                                    connection::Update::Subscription { key, item } => {
                                        if key == "server-config" {
                                            match item {
                                                Ok(value) => {
                                                    if value["type"] == "snapshot" {
                                                        app.project_settings = Some(
                                                            value["config"]["settings"].clone(),
                                                        );
                                                    } else if value["type"] == "settingsUpdated" {
                                                        app.project_settings = Some(
                                                            value["payload"]["settings"].clone(),
                                                        );
                                                    }
                                                    Ok(())
                                                }
                                                Err(error) => {
                                                    app.project_settings = None;
                                                    app.set_status(error, cx);
                                                    Ok(())
                                                }
                                            }
                                        } else if key == "project-clones"
                                            && app.inspector_section
                                                == Some(InspectorSection::Clone)
                                        {
                                            match item {
                                                Ok(value) => {
                                                    let running: Vec<String> = app
                                                        .clone_progress
                                                        .iter()
                                                        .filter(|clone| clone.phase == "running")
                                                        .map(|clone| clone.project_id.clone())
                                                        .collect();
                                                    let result = workspaces::apply_clones(
                                                        &mut app.clone_progress,
                                                        value,
                                                    );
                                                    if app.clone_progress.iter().any(|clone| {
                                                        clone.phase == "done"
                                                            && running.contains(&clone.project_id)
                                                    }) {
                                                        app.report_success("Repository cloned", cx);
                                                    }
                                                    result
                                                }
                                                Err(error) => {
                                                    app.set_status(error, cx);
                                                    app.clone_progress.clear();
                                                    Ok(())
                                                }
                                            }
                                        } else if key == "worktree-setup" {
                                            match item {
                                                Ok(value) => {
                                                    if let Some(id) = &app.selected_thread {
                                                        workspaces::apply_setup(
                                                            &mut app.setup_progress,
                                                            id,
                                                            value,
                                                        )
                                                    } else {
                                                        Ok(())
                                                    }
                                                }
                                                Err(error) => {
                                                    app.set_status(error, cx);
                                                    Ok(())
                                                }
                                            }
                                        } else {
                                            Ok(())
                                        }
                                    }
                                    connection::Update::Thread { thread_id, item } => {
                                        if app.selected_thread.as_ref() == Some(&thread_id) {
                                            app.apply_thread_update(&thread_id, item, cx)
                                        } else {
                                            Ok(())
                                        }
                                    }
                                };
                                if let Err(error) = result {
                                    app.set_status(
                                        format!("Projection failed — reconnect required: {error}"),
                                        cx,
                                    );
                                    app.commands = None;
                                    cx.notify();
                                    return false;
                                }
                                cx.notify();
                                true
                            });
                            if !matches!(keep_running, Ok(true)) {
                                break;
                            }
                        }
                    }));
                }
                Err(message) => self.set_status(message, cx),
            }
        }
        cx.notify();
    }

    pub(crate) fn perform(
        &mut self,
        tag: &str,
        payload: Value,
        sent_prompt: Option<(String, gpui_kit::AnyWindowHandle)>,
        cx: &mut Context<Self>,
    ) {
        let Some(sender) = self.commands.clone() else {
            self.set_status("Not connected", cx);
            cx.notify();
            return;
        };
        let generation = self.generation;
        let thread_id = self.selected_thread.clone();
        let tag = tag.to_owned();
        let reverting = payload["type"] == "thread.checkpoint.revert";
        self.pending_actions += 1;
        self.set_status("Waiting for server receipt…", cx);
        cx.spawn(async move |entity, cx| {
            let result = connection::call(&sender, &tag, payload).await;
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
                            app.attachments.clear();
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
                            if app.selected_thread.as_ref() == Some(id)
                                && let Some(sender) = &app.commands
                            {
                                let _ = sender
                                    .try_send(connection::Command::SelectThread(Some(id.clone())));
                            }
                        }
                        if tag == "projectClone.start" {
                            app.selected_project = value["projectId"].as_str().map(str::to_owned);
                        }
                        if tag == "server.getSettings" {
                            app.project_settings = Some(value.clone());
                        }
                        if tag == "server.getConfig" {
                            match serde_json::from_value::<providers::Config>(value.clone()) {
                                Ok(config) => app.provider_catalog = config.providers,
                                Err(_) => {
                                    app.set_status("Invalid provider catalog response", cx);
                                    cx.notify();
                                    return;
                                }
                            }
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
                        if tag == "provider.auth.start" {
                            app.auth_state = Some(value.clone());
                            if let Some(url) = value["authorizationUrl"].as_str() {
                                cx.open_url(url);
                            }
                        }
                        if tag == "provider.auth.start" {
                            app.auth_state = Some(value.clone());
                            if let Some(url) = value["authorizationUrl"].as_str() {
                                cx.open_url(url);
                            }
                        }
                        if tag == "orchestration.getArchivedShellSnapshot" {
                            match serde_json::from_value::<shell::Snapshot>(value.clone()) {
                                Ok(snapshot) => {
                                    let _ = app
                                        .archived_shell
                                        .apply(shell::ShellItem::Snapshot { snapshot });
                                }
                                Err(_) => {
                                    app.set_status("Invalid archive snapshot", cx);
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
                    Err(error) => app.set_status(error, cx),
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
