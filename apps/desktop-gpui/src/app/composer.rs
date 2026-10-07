//! Prompt drafts and stash, sending and queueing, attachments, and replies to agent requests.

use crate::{NativeApp, attachment_mime};
use gpui_kit::{App, Context, Window};
use serde_json::json;
use t3code_desktop_gpui::{commands, connection, files, mentions, palette, queue};

impl NativeApp {
    /// Whether the "@" file menu shows: its token is still the one before the cursor.
    pub(crate) fn mention_open(&self, cx: &App) -> bool {
        let Some((range, query)) = &self.mention else {
            return false;
        };
        let input = self.composer.read(cx);
        let value = input.value();
        mentions::path_trigger(&value, input.cursor())
            .is_some_and(|(at, typed)| &at == range && typed == query)
    }

    /// Follows the composer's "@" token; a new query searches the selected
    /// thread's workspace and only the latest request's reply is shown.
    pub(crate) fn update_mention(&mut self, cx: &mut Context<Self>) {
        let input = self.composer.read(cx);
        let value = input.value();
        let trigger = palette::slash_matches(&value, self.selected_thread.is_some())
            .is_none()
            .then(|| mentions::path_trigger(&value, input.cursor()))
            .flatten()
            .map(|(range, query)| (range, query.to_owned()));
        let Some((range, query)) = trigger else {
            self.close_mention(cx);
            return;
        };
        let unchanged = self
            .mention
            .as_ref()
            .is_some_and(|(_, current)| current == &query);
        self.mention = Some((range, query.clone()));
        if unchanged {
            return;
        }
        self.mention_epoch += 1;
        self.mention_entries.clear();
        self.mention_index = 0;
        self.mention_loading = false;
        cx.notify();
        let (Some(sender), Some(cwd)) = (self.commands.clone(), self.selected_workspace()) else {
            return;
        };
        self.mention_loading = true;
        let epoch = self.mention_epoch;
        let generation = self.generation;
        let payload = json!({"cwd": cwd, "query": query, "limit": 100});
        self.mention_task = Some(cx.spawn(async move |entity, cx| {
            let result = connection::call(&sender, "projects.searchEntries", payload).await;
            let _ = entity.update(cx, |app, cx| {
                if app.generation != generation || app.mention_epoch != epoch {
                    return;
                }
                app.mention_loading = false;
                app.mention_entries = result
                    .ok()
                    .and_then(|value| serde_json::from_value::<files::Listing>(value).ok())
                    .map(|listing| listing.entries)
                    .unwrap_or_default();
                cx.notify();
            });
        }));
    }

    /// Closes the "@" menu and drops its pending search; the text stays as typed.
    pub(crate) fn close_mention(&mut self, cx: &mut Context<Self>) {
        if self.mention.take().is_some() {
            self.mention_epoch += 1;
            self.mention_entries.clear();
            self.mention_loading = false;
            self.mention_task = None;
            cx.notify();
        }
    }

    /// Replaces the "@" token with row `index`'s file link, as the web composer does.
    pub(crate) fn pick_mention(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (Some((range, query)), Some(entry)) =
            (self.mention.clone(), self.mention_entries.get(index))
        else {
            return;
        };
        let link = format!("{} ", mentions::file_link(&entry.path));
        let value = self.composer.read(cx).value().to_string();
        if value.get(range.clone()) != Some(&format!("@{query}")) {
            return;
        }
        // A space already after the token is reused rather than doubled.
        let end = range.end + usize::from(value[range.end..].starts_with(' '));
        self.close_mention(cx);
        self.composer.update(cx, |input, cx| {
            input.set_selected_range(range.start..end, cx);
            input.replace(link, window, cx);
        });
        self.remember_composer_draft(cx);
        cx.notify();
    }

    pub(crate) fn schedule_draft_save(&mut self, cx: &mut Context<Self>) {
        self.draft_save = Some(cx.spawn(async move |entity, cx| {
            cx.background_executor()
                .timer(std::time::Duration::from_millis(200))
                .await;
            let write = entity.update(cx, |app, _| {
                app.draft_store.as_ref().map(|store| store.write_snapshot())
            });
            if let Ok(Some(Ok(write))) = write {
                let result = cx
                    .background_executor()
                    .spawn(async move { write.persist() })
                    .await;
                if result.is_err() {
                    let _ = entity.update(cx, |app, cx| {
                        app.storage_error = Some("Could not persist native drafts".into());
                        cx.notify();
                    });
                }
            }
        }));
    }

    pub(crate) fn stash_prompt(
        &mut self,
        restore: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let text = self.composer.read(cx).value().to_string();
        if restore {
            if !text.is_empty() {
                self.set_status("Stash or clear the current prompt before restoring", cx);
                cx.notify();
                return;
            }
            let text = if let Some(store) = &mut self.draft_store {
                store.unstash(&self.environment_scope)
            } else {
                self.stash_memory.pop()
            };
            if let Some(text) = text {
                self.composer
                    .update(cx, |input, cx| input.set_value(text, window, cx));
            }
        } else {
            if text.trim().is_empty() {
                return;
            }
            if let Some(store) = &mut self.draft_store {
                store.stash(&self.environment_scope, text);
            } else {
                self.stash_memory.push(text);
            }
            self.composer
                .update(cx, |input, cx| input.set_value("", window, cx));
        }
        if self.draft_store.is_some() {
            self.schedule_draft_save(cx);
        }
        cx.notify();
    }

    /// The composer's prompt as the selected thread would send it now, with
    /// the model and modes it goes out with.
    pub(crate) fn composer_message(
        &self,
        cx: &App,
    ) -> Result<(String, queue::QueuedMessage), &'static str> {
        let Some(id) = self.selected_thread.clone() else {
            return Err("Select a thread first");
        };
        let Some(thread) = self
            .shell
            .threads
            .iter()
            .chain(self.pending_threads.iter().map(|draft| &draft.thread))
            .find(|t| t.id == id)
        else {
            return Err("Select a thread first");
        };
        let text = self.composer.read(cx).value().to_string();
        if text.trim().is_empty() {
            return Err("A prompt is required");
        }
        let selection = self.model_selection(&thread.model_selection, cx)?;
        if self.provider_catalog.iter().any(|provider| {
            provider.instance_id == selection["instanceId"].as_str().unwrap_or("")
                && provider.requires_new_thread_for_model_change
        }) && selection != thread.model_selection
            && self.detail.thread.as_ref().is_some_and(|detail| {
                detail["messages"]
                    .as_array()
                    .is_some_and(|messages| !messages.is_empty())
            })
        {
            return Err("This provider requires a new thread for a model change");
        }
        let message = queue::QueuedMessage {
            id: 0,
            text,
            attachments: self.attachments.clone(),
            model_selection: selection,
            runtime_mode: thread
                .runtime_mode
                .clone()
                .unwrap_or_else(|| "full-access".into()),
            interaction_mode: thread
                .interaction_mode
                .clone()
                .unwrap_or_else(|| "default".into()),
        };
        Ok((id, message))
    }

    /// Enter: sends the prompt, or queues it behind the running turn.
    pub(crate) fn send_prompt(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.turn_running() {
            self.queue_prompt(false, window, cx);
            return;
        }
        if self.pending_actions > 0 {
            self.set_status("Wait for the pending server receipt", cx);
            cx.notify();
            return;
        }
        if self.selected_thread.is_none() {
            return;
        }
        let (id, message) = match self.composer_message(cx) {
            Ok(prepared) => prepared,
            Err(error) => {
                self.set_status(error, cx);
                cx.notify();
                return;
            }
        };
        match message.start_command(&id) {
            Ok(mut payload) => {
                if let Some(draft) = self
                    .pending_threads
                    .iter()
                    .find(|draft| draft.thread.id == id)
                {
                    payload["bootstrap"] = json!({"createThread":{"projectId":draft.thread.project_id,"title":draft.thread.title,"modelSelection":payload["modelSelection"],"runtimeMode":payload["runtimeMode"],"interactionMode":payload["interactionMode"],"branch":draft.thread.branch,"worktreePath":draft.thread.worktree_path,"createdAt":draft.created_at},"runSetupScript":draft.run_setup_script});
                    if let Some(prepare) = &draft.prepare_worktree {
                        payload["bootstrap"]["prepareWorktree"] = prepare.clone();
                    }
                }
                let text = message.text;
                self.drafts.insert(id, text.clone());
                self.perform(
                    "orchestration.dispatchCommand",
                    payload,
                    Some((text, window.window_handle())),
                    cx,
                );
            }
            Err(message) => {
                self.set_status(message, cx);
                cx.notify();
            }
        }
    }

    /// Takes the composer's prompt into the selected thread's queue. Steering
    /// puts it first and interrupts the turn; it goes out once the interrupt
    /// is accepted.
    pub(crate) fn queue_prompt(
        &mut self,
        steer: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (thread_id, message) = match self.composer_message(cx) {
            Ok(prepared) => prepared,
            Err(error) => {
                self.set_status(error, cx);
                cx.notify();
                return;
            }
        };
        let id = self.queues.push(&thread_id, message, steer);
        self.attachments.clear();
        self.composer
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.remember_composer_draft(cx);
        self.set_status(
            if steer {
                "Interrupting to steer…"
            } else {
                "Queued — sends when the turn finishes"
            },
            cx,
        );
        cx.notify();
        if !steer {
            return;
        }
        let Some(sender) = self.commands.clone() else {
            self.set_status("Not connected", cx);
            return;
        };
        let mut payload = commands::command("thread.turn.interrupt", &thread_id);
        payload["createdAt"] = json!(commands::now());
        cx.spawn(async move |entity, cx| {
            let result = connection::call(&sender, "orchestration.dispatchCommand", payload).await;
            let _ = entity.update(cx, |app, cx| match result {
                Ok(_) => app.send_queued(&thread_id, id, cx),
                Err(error) => {
                    app.set_status(error, cx);
                    cx.notify();
                }
            });
        })
        .detach();
    }

    /// ⌘⇧↩: steers while a turn runs, otherwise sends as Enter does.
    pub(crate) fn steer_prompt(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.turn_running() {
            self.queue_prompt(true, window, cx);
        } else {
            self.send_prompt(window, cx);
        }
    }

    /// Whether `thread_id`'s turn runs, from the open detail when it is the
    /// selected thread and the shell otherwise. None when the server has not said.
    pub(crate) fn thread_running(&self, thread_id: &str) -> Option<bool> {
        let session = if self.selected_thread.as_deref() == Some(thread_id)
            && let Some(detail) = &self.detail.thread
        {
            detail["session"]["status"].as_str()
        } else {
            self.shell
                .threads
                .iter()
                .find(|thread| thread.id == thread_id)?
                .session
                .as_ref()
                .map(|session| session.status.as_str())
        };
        Some(matches!(session, Some("starting" | "running")))
    }

    /// Sends each queue's next message whose thread finished its turn.
    pub(crate) fn flush_queues(&mut self, cx: &mut Context<Self>) {
        for thread_id in self.queues.thread_ids() {
            let Some(running) = self.thread_running(&thread_id) else {
                continue;
            };
            if let Some(message) = self.queues.observe(&thread_id, running) {
                self.dispatch_queued(thread_id, message, cx);
            }
        }
    }

    /// Sends queued message `id` now, ahead of the turn finishing.
    pub(crate) fn send_queued(&mut self, thread_id: &str, id: u64, cx: &mut Context<Self>) {
        if let Some(message) = self.queues.take(thread_id, id) {
            self.dispatch_queued(thread_id.to_owned(), message, cx);
        }
    }

    pub(crate) fn dispatch_queued(
        &mut self,
        thread_id: String,
        message: queue::QueuedMessage,
        cx: &mut Context<Self>,
    ) {
        let Some(sender) = self.commands.clone() else {
            self.queues.settle(&thread_id, message.id, false);
            self.set_status("Not connected", cx);
            cx.notify();
            return;
        };
        let payload = match message.start_command(&thread_id) {
            Ok(payload) => payload,
            Err(error) => {
                self.queues.settle(&thread_id, message.id, false);
                self.set_status(error, cx);
                cx.notify();
                return;
            }
        };
        self.set_status("Sending queued message…", cx);
        cx.notify();
        cx.spawn(async move |entity, cx| {
            let result = connection::call(&sender, "orchestration.dispatchCommand", payload).await;
            let _ = entity.update(cx, |app, cx| {
                app.queues.settle(&thread_id, message.id, result.is_ok());
                app.set_status(
                    match result {
                        Ok(_) => "Queued message sent".into(),
                        Err(error) => format!("Queued message not sent: {error}"),
                    },
                    cx,
                );
                cx.notify();
            });
        })
        .detach();
    }

    /// Drops a queued message and deletes its uploaded attachments.
    pub(crate) fn remove_queued(&mut self, thread_id: &str, id: u64, cx: &mut Context<Self>) {
        let Some(message) = self.queues.remove(thread_id, id) else {
            return;
        };
        for attachment in message.attachments {
            if let Some(attachment_id) = attachment["id"].as_str() {
                self.perform(
                    "attachments.delete",
                    json!({"attachmentId": attachment_id}),
                    None,
                    cx,
                );
            }
        }
        cx.notify();
    }

    /// Answer the provider's pending approval request.
    pub(crate) fn respond_approval(
        &mut self,
        request_id: &str,
        decision: &str,
        cx: &mut Context<Self>,
    ) {
        let Some(id) = self.selected_thread.clone() else {
            return;
        };
        let payload = commands::approval_response(&id, request_id, decision);
        self.dispatch(payload, cx);
    }

    /// Choose an option of a pending question. Choosing clears the question's
    /// custom answer, which would otherwise outrank the choice.
    pub(crate) fn select_user_input_option(
        &mut self,
        question_id: &str,
        value: &str,
        multi_select: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let selected = self
            .user_input_selected
            .entry(question_id.to_owned())
            .or_default();
        if !multi_select {
            selected.clear();
            selected.push(value.to_owned());
        } else if let Some(position) = selected.iter().position(|chosen| chosen == value) {
            selected.remove(position);
        } else {
            selected.push(value.to_owned());
        }
        // The displaced custom answer moves into the composer rather than vanishing.
        if let Some(input) = self.user_input_custom.get(question_id).cloned() {
            let displaced = input.read(cx).value().trim().to_owned();
            if !displaced.is_empty() {
                input.update(cx, |input, cx| input.set_value("", window, cx));
                let prompt = self.composer.read(cx).value().to_string();
                let prompt = if prompt.trim().is_empty() {
                    displaced
                } else {
                    format!("{}\n\n{displaced}", prompt.trim_end())
                };
                self.composer
                    .update(cx, |input, cx| input.set_value(prompt, window, cx));
                self.remember_composer_draft(cx);
            }
        }
        cx.notify();
    }

    /// Typing a custom answer replaces the question's chosen options.
    pub(crate) fn type_user_input_custom(&mut self, question_id: &str, cx: &mut Context<Self>) {
        let Some(input) = self.user_input_custom.get(question_id) else {
            return;
        };
        if !input.read(cx).value().trim().is_empty() {
            self.user_input_selected.remove(question_id);
            cx.notify();
        }
    }

    /// Persist the composer text as the selected thread's draft.
    pub(crate) fn remember_composer_draft(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.selected_thread.clone() else {
            return;
        };
        let text = self.composer.read(cx).value().to_string();
        self.drafts.insert(id.clone(), text.clone());
        if let Some(store) = &mut self.draft_store {
            store.set(&self.environment_scope, &id, text);
            self.schedule_draft_save(cx);
        }
    }

    /// Answer the pending user-input request. The card stays until the server
    /// records the request as resolved.
    pub(crate) fn submit_user_input(
        &mut self,
        request: &t3code_desktop_gpui::conversation::PendingUserInput,
        cx: &mut Context<Self>,
    ) {
        let Some(id) = self.selected_thread.clone() else {
            return;
        };
        let custom = self
            .user_input_custom
            .iter()
            .map(|(question, input)| (question.clone(), input.read(cx).value().to_string()))
            .collect();
        match commands::user_input_response(
            &id,
            &request.request_id,
            &request.questions,
            &self.user_input_selected,
            &custom,
        ) {
            Ok(payload) => self.dispatch(payload, cx),
            Err(error) => {
                self.set_status(error, cx);
                cx.notify();
            }
        }
    }

    pub(crate) fn dismiss_user_input(&mut self, request_id: &str, cx: &mut Context<Self>) {
        let Some(id) = self.selected_thread.clone() else {
            return;
        };
        let payload = commands::user_input_dismiss(&id, request_id);
        self.dispatch(payload, cx);
    }

    /// Open the platform file picker, upload each selected file, and stage it as
    /// an attachment for the next turn.
    pub(crate) fn attach_files(&mut self, cx: &mut Context<Self>) {
        let Some(sender) = self.commands.clone() else {
            self.set_status("Not connected", cx);
            cx.notify();
            return;
        };
        let origin = self.environment_scope.clone();
        let receiver = cx.prompt_for_paths(gpui_kit::PathPromptOptions {
            files: true,
            directories: false,
            multiple: true,
            prompt: Some("Attach files".into()),
        });
        self.pending_actions += 1;
        self.set_status("Uploading attachments…", cx);
        cx.notify();
        cx.spawn(async move |entity, cx| {
            let paths = match receiver.await {
                Ok(Ok(Some(paths))) => paths,
                _ => Vec::new(),
            };
            for path in paths {
                let name = path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("attachment")
                    .to_owned();
                let bytes = match std::fs::read(&path) {
                    Ok(bytes) => bytes,
                    Err(error) => {
                        let _ = entity.update(cx, |app, cx| {
                            app.set_status(format!("Could not read {name}: {error}"), cx);
                            cx.notify();
                        });
                        continue;
                    }
                };
                let mime = attachment_mime(&name).to_owned();
                let size = bytes.len() as u64;
                let input = json!({
                    "type": if mime.starts_with("image/") { "image" } else { "file" },
                    "name": name,
                    "mimeType": mime,
                    "sizeBytes": size,
                });
                let attachment = match connection::call(&sender, "attachments.createUploadUrl", input).await {
                    Ok(value) => {
                        let id = value["attachmentId"].as_str().unwrap_or_default().to_owned();
                        let relative = value["relativeUrl"].as_str().unwrap_or_default().to_owned();
                        match connection::upload(&sender, format!("{origin}{relative}"), mime.clone(), bytes).await {
                            Ok(()) => Some(json!({"id": id, "name": name, "mimeType": mime, "sizeBytes": size})),
                            Err(error) => {
                                let _ = entity.update(cx, |app, cx| {
                                    app.set_status(error, cx);
                                    cx.notify();
                                });
                                None
                            }
                        }
                    }
                    Err(error) => {
                        let _ = entity.update(cx, |app, cx| {
                            app.set_status(error, cx);
                            cx.notify();
                        });
                        None
                    }
                };
                if let Some(attachment) = attachment {
                    let label = attachment["name"].as_str().unwrap_or("attachment").to_owned();
                    let _ = entity.update(cx, |app, cx| {
                        app.attachments.push(attachment);
                        app.set_status(format!("Attached {label}"), cx);
                        cx.notify();
                    });
                }
            }
            let _ = entity.update(cx, |app, cx| {
                app.pending_actions = app.pending_actions.saturating_sub(1);
                cx.notify();
            });
        })
        .detach();
    }
}
