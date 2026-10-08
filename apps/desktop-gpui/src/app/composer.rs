//! Prompt drafts and stash, sending and queueing, attachments, and replies to agent requests.

use super::attachments::{AttachmentScope, read_attachment, upload_target};
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
            } else {
                self.set_status("Nothing stashed yet", cx);
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

    /// Enter follows the preference; the alternate submission reverses it.
    pub(crate) fn send_prompt(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.submit_follow_up(false, window, cx);
    }

    fn submit_follow_up(&mut self, alternate: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.checkout_transition_pending() {
            self.set_status("Finish checkout reconciliation before sending; message retained", cx);
            cx.notify();
            return;
        }
        let thread_id = self.selected_thread.clone().unwrap_or_default();
        let active_queue = self.queues.dispatching(&thread_id)
            || (self.queues.has_queued(&thread_id) && !self.queues.held(&thread_id));
        let queue = active_queue
            || (self.turn_running()
                && ((self.follow_up_behavior == queue::FollowUpBehavior::Queue) != alternate));
        if queue {
            self.queue_prompt(false, window, cx);
            return;
        }
        if self.turn_running() {
            self.queue_prompt(true, window, cx);
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
        if self
            .queue_observation(&thread_id)
            .is_some_and(|state| state.blocked || state.connecting)
        {
            self.set_status("Resolve pending requests and reconnect before sending", cx);
            cx.notify();
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
                match self.materialized_checkout_bootstrap(&id) {
                    Ok(Some(bootstrap)) => payload["bootstrap"] = bootstrap,
                    Ok(None) => {},
                    Err(error) => {
                        self.set_status(error, cx);
                        cx.notify();
                        return;
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

    /// Moves the complete draft into the queue. Steer uses the same in-turn
    /// start contract as web; providers reject unsupported sends without losing it.
    pub(crate) fn queue_prompt(
        &mut self,
        steer: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.checkout_transition_pending() {
            self.set_status("Finish checkout reconciliation before sending; message retained", cx);
            cx.notify();
            return;
        }
        let (thread_id, message) = match self.composer_message(cx) {
            Ok(prepared) => prepared,
            Err(error) => {
                self.set_status(error, cx);
                cx.notify();
                return;
            }
        };
        let state = self.queue_observation(&thread_id);
        if steer
            && state
                .as_ref()
                .is_none_or(|state| state.blocked || state.connecting)
        {
            self.set_status("Resolve the pending agent request before steering", cx);
            cx.notify();
            return;
        }
        let id = self.queues.push_at(
            &thread_id,
            message,
            steer,
            state.as_ref().and_then(|state| state.tool.clone()),
        );
        if let Some(state) = &state {
            // Record the projection before an explicit Send now/steer receipt.
            if let Some(message) = self.queues.observe_state(&thread_id, state) {
                self.dispatch_queued(thread_id.clone(), message, cx);
            }
        }
        self.attachments.clear();
        self.composer
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.remember_composer_draft(cx);
        self.set_status(
            if steer {
                "Steering…"
            } else {
                "Queued — sends at the next tool boundary or turn end"
            },
            cx,
        );
        cx.notify();
        self.reconcile_queue_subscriptions();
        if !steer {
            return;
        }
        self.send_queued(&thread_id, id, cx);
    }

    /// ⌘⇧↩ submits with the alternate follow-up behavior.
    pub(crate) fn steer_prompt(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.submit_follow_up(true, window, cx);
    }

    pub(crate) fn queue_observation(&self, thread_id: &str) -> Option<queue::Observation> {
        self.queue_observation_for(thread_id, false)
    }

    pub(crate) fn queue_observation_for(
        &self,
        thread_id: &str,
        background: bool,
    ) -> Option<queue::Observation> {
        let shell = self
            .shell
            .threads
            .iter()
            .find(|thread| thread.id == thread_id)?;
        let cache = self.queue_details.get(thread_id);
        // Creating a queue stream must not hide the selected projection. Once
        // its baseline is valid, keep using that stream across selection changes
        // so switching views cannot introduce a tool or dispatch receipt.
        let cached =
            cache.filter(|cache| cache.detail.synchronized && cache.detail.thread.is_some());
        let selected = !background
            && self.selected_thread.as_deref() == Some(thread_id)
            && self
                .detail
                .thread
                .as_ref()
                .is_some_and(|detail| detail["id"] == thread_id)
            && cached.is_none();
        let (detail, detail_sequence) = if selected {
            (self.detail.thread.as_ref(), self.detail.sequence)
        } else {
            (
                cached.and_then(|cache| cache.detail.thread.as_ref()),
                cached.and_then(|cache| cache.detail.sequence),
            )
        };
        let activities = detail.and_then(|detail| detail["activities"].as_array());
        let blocked = shell.archived_at.is_some()
            || shell.has_pending_approvals
            || shell.has_pending_user_input
            || activities.is_some_and(|activities| {
                t3code_desktop_gpui::conversation::pending_approval(activities).is_some()
                    || t3code_desktop_gpui::conversation::pending_user_input(activities).is_some()
            })
            || self.commands.is_none();
        // Keep the shell marker independent of selection. Losing detail must
        // not look like an acknowledgment (or a completed tool).
        let identity = json!([
            shell
                .latest_turn
                .as_ref()
                .map(|turn| (&turn.turn_id, &turn.completed_at)),
            shell.latest_user_message_at,
            shell
                .session
                .as_ref()
                .map(|session| (&session.status, &session.last_error)),
        ])
        .to_string();
        let detail_identity = detail.map(|detail| {
            let user = detail["messages"].as_array().and_then(|messages| {
                messages
                    .iter()
                    .rev()
                    .find(|message| message["role"] == "user")
            });
            let user_id = user.and_then(|message| message["id"].as_str());
            let failure = activities.and_then(|activities| {
                activities.iter().rev().find(|activity| {
                    activity["kind"] == "provider.turn.start.failed"
                        && user_id.is_some()
                        && activity["payload"]["requestId"].as_str() == user_id
                })
            });
            // Match web's dispatch snapshot signals, not a clock or generic
            // thread updatedAt (which also changes for unrelated metadata).
            json!([
                user_id,
                user.map(|message| &message["createdAt"]),
                detail["latestTurn"]["turnId"],
                detail["latestTurn"]["requestedAt"],
                detail["latestTurn"]["startedAt"],
                detail["latestTurn"]["completedAt"],
                detail["session"]["status"],
                detail["session"]["updatedAt"],
                failure.map(|activity| &activity["id"]),
            ])
            .to_string()
        });
        let shell_status = shell
            .session
            .as_ref()
            .map(|session| session.status.as_str());
        let detail_status = detail.and_then(|detail| detail["session"]["status"].as_str());
        let status = if !selected && self.shell.sequence >= detail_sequence {
            shell_status.or(detail_status)
        } else {
            detail_status.or(shell_status)
        };
        let tool = activities.and_then(|activities| {
            activities
                .iter()
                .filter(|activity| activity["kind"] == "tool.completed")
                .max_by_key(|activity| {
                    (
                        activity["sequence"].as_i64().unwrap_or(-1),
                        activity["createdAt"].as_str().unwrap_or(""),
                    )
                })
                .and_then(|activity| activity["id"].as_str())
                .map(str::to_owned)
        });
        Some(queue::Observation {
            running: status == Some("running"),
            // Known completion needs no future live event. Running streams
            // still need a valid detail baseline before tools can be used.
            connecting: status == Some("starting")
                || (self.queues.has_queued(thread_id)
                    && (status.is_none()
                        || (!selected && cached.is_none() && status == Some("running")))),
            blocked,
            identity,
            detail_identity,
            tool,
            tool_available: activities.is_some(),
        })
    }

    /// Sends each queue's next message whose thread finished its turn.
    pub(crate) fn flush_queues(&mut self, cx: &mut Context<Self>) {
        if self.checkout_transition_pending() {
            return;
        }
        self.reconcile_queue_subscriptions();
        for thread_id in self.queues.thread_ids() {
            let Some(state) = self.queue_observation(&thread_id) else {
                continue;
            };
            if let Some(message) = self.queues.observe_state(&thread_id, &state) {
                self.dispatch_queued(thread_id, message, cx);
            }
        }
    }

    /// Sends queued message `id` now, ahead of the turn finishing.
    pub(crate) fn send_queued(&mut self, thread_id: &str, id: u64, cx: &mut Context<Self>) {
        if self.checkout_transition_pending() {
            self.set_status("Finish checkout reconciliation before sending; queued message retained", cx);
            cx.notify();
            return;
        }
        let state = self.queue_observation(thread_id);
        if state
            .as_ref()
            .is_none_or(|state| state.blocked || state.connecting)
        {
            self.set_status("Resolve pending requests and reconnect before sending", cx);
            cx.notify();
            return;
        }
        if let Some(state) = state {
            self.queues.record(thread_id, &state);
        }
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
        if self.checkout_transition_pending() {
            self.queues.settle(&thread_id, message.id, false);
            cx.notify();
            return;
        }
        let Some(sender) = self.commands.clone() else {
            self.queues.settle(&thread_id, message.id, false);
            self.set_status("Not connected", cx);
            cx.notify();
            return;
        };
        let mut payload = match message.start_command(&thread_id) {
            Ok(payload) => payload,
            Err(error) => {
                self.queues.settle(&thread_id, message.id, false);
                self.set_status(error, cx);
                cx.notify();
                return;
            }
        };
        match self.materialized_checkout_bootstrap(&thread_id) {
            Ok(Some(bootstrap)) => payload["bootstrap"] = bootstrap,
            Ok(None) => {},
            Err(error) => {
                self.queues.settle(&thread_id, message.id, false);
                self.set_status(error, cx);
                cx.notify();
                return;
            }
        }
        let generation = self.generation;
        let scope = self.environment_scope.clone();
        self.set_status("Sending queued message…", cx);
        cx.notify();
        cx.spawn(async move |entity, cx| {
            let ready = entity.update(cx, |app, cx| {
                let ready = app.generation == generation && app.environment_scope == scope && !app.checkout_transition_pending();
                if !ready && app.environment_scope == scope {
                    app.queues.settle(&thread_id, message.id, false);
                    cx.notify();
                }
                ready
            }).unwrap_or(false);
            if !ready { return; }
            let result = connection::call(&sender, "orchestration.dispatchCommand", payload).await;
            let _ = entity.update(cx, |app, cx| {
                if app.generation != generation {
                    if app.environment_scope == scope {
                        app.queues.settle(&thread_id, message.id, false);
                        cx.notify();
                    }
                    return;
                }
                app.queues.settle(&thread_id, message.id, result.is_ok());
                app.flush_queues(cx);
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
        self.flush_queues(cx);
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

    /// Save before changing selected thread or environment; shelves are never
    /// cleared by disconnect. Merge late receipts without replacing draft context.
    pub(crate) fn stash_attachment_draft(&mut self) {
        if let Some(thread) = &self.selected_thread {
            let shelf = self
                .attachment_drafts
                .shelves
                .entry((self.environment_scope.clone(), thread.clone()))
                .or_default();
            for attachment in std::mem::take(&mut self.attachments) {
                if !shelf.iter().any(|item| item["id"] == attachment["id"]) {
                    shelf.push(attachment);
                }
            }
        } else {
            self.attachments.clear();
        }
    }

    /// Restore immediately after installing the new environment/thread selection.
    pub(crate) fn restore_attachment_draft(&mut self) {
        self.attachments = self
            .selected_thread
            .as_ref()
            .and_then(|thread| {
                self.attachment_drafts
                    .shelves
                    .remove(&(self.environment_scope.clone(), thread.clone()))
            })
            .unwrap_or_default();
    }

    /// Consume only refs actually transmitted by an accepted send. Failed sends
    /// never call this; a late success must not erase attachments added meanwhile.
    pub(crate) fn consume_sent_attachments(
        &mut self,
        origin: &str,
        thread: &str,
        sent: &[serde_json::Value],
    ) {
        let keep = |item: &serde_json::Value| !sent.iter().any(|sent| sent["id"] == item["id"]);
        if self.environment_scope == origin && self.selected_thread.as_deref() == Some(thread) {
            self.attachments.retain(keep);
        }
        let key = (origin.to_owned(), thread.to_owned());
        if let Some(shelf) = self.attachment_drafts.shelves.get_mut(&key) {
            shelf.retain(keep);
            if shelf.is_empty() {
                self.attachment_drafts.shelves.remove(&key);
            }
        }
    }

    pub(crate) fn attachment_scope(&self) -> Option<AttachmentScope> {
        Some(AttachmentScope {
            origin: self.environment_scope.clone(),
            thread: self.selected_thread.clone()?,
            generation: self.generation,
            sender: self.commands.clone()?,
        })
    }

    fn attachment_connection_current(&self, scope: &AttachmentScope) -> bool {
        self.generation == scope.generation
            && self.environment_scope == scope.origin
            && !scope.sender.is_closed()
            && self
                .commands
                .as_ref()
                .is_some_and(|sender| sender.same_channel(&scope.sender))
    }

    fn attachment_selected(&self, scope: &AttachmentScope) -> bool {
        self.attachment_connection_current(scope)
            && self.selected_thread.as_deref() == Some(&scope.thread)
    }

    fn attachment_notice(
        &mut self,
        scope: &AttachmentScope,
        text: impl Into<String>,
        cx: &mut Context<Self>,
    ) {
        if self.attachment_selected(scope) {
            self.set_status(text.into(), cx);
            cx.notify();
        }
    }

    /// Explicit detach deletes the selected resource on its captured connection.
    /// A failed delete retains it, allowing a retry on that same draft.
    pub(crate) fn detach_attachment(
        &mut self,
        scope: AttachmentScope,
        id: String,
        cx: &mut Context<Self>,
    ) {
        let resources = if self.environment_scope == scope.origin
            && self.selected_thread.as_deref() == Some(&scope.thread)
        {
            Some(&self.attachments)
        } else {
            self.attachment_drafts.shelves.get(&scope.key())
        };
        let sent: Vec<_> = resources
            .into_iter()
            .flatten()
            .filter(|item| item["id"].as_str() == Some(&id))
            .cloned()
            .collect();
        if sent.is_empty() {
            return;
        }
        cx.spawn(async move |entity, cx| {
            let ready = entity
                .update(cx, |app, _| app.attachment_connection_current(&scope))
                .unwrap_or(false);
            if !ready {
                return;
            }
            let result = connection::call(
                &scope.sender,
                "attachments.delete",
                json!({"attachmentId":id}),
            )
            .await;
            let _ = entity.update(cx, |app, cx| {
                if result.is_ok() {
                    app.consume_sent_attachments(&scope.origin, &scope.thread, &sent);
                    cx.notify();
                } else {
                    app.attachment_notice(
                        &scope,
                        "Attachment delete failed; attachment retained",
                        cx,
                    );
                }
            });
        })
        .detach();
    }

    /// Reject a missing target before opening the picker. Cancel has no effect
    /// on status, pending work, text or attachment context.
    pub(crate) fn attach_files(&mut self, cx: &mut Context<Self>) {
        if self.selected_thread.is_none() {
            self.set_status("Select a thread first", cx);
            cx.notify();
            return;
        }
        let Some(scope) = self.attachment_scope() else {
            self.set_status("Not connected", cx);
            cx.notify();
            return;
        };
        if self.attachment_drafts.uploading {
            return;
        }
        let receiver = cx.prompt_for_paths(gpui_kit::PathPromptOptions {
            files: true,
            directories: false,
            multiple: true,
            prompt: Some("Attach files".into()),
        });
        cx.spawn(async move |entity, cx| {
            let paths = receiver.await.ok().and_then(Result::ok).flatten();
            let _ = entity.update(cx, |app, cx| app.attach_paths(scope, paths, cx));
        })
        .detach();
    }

    /// Shared picker/path seam. Reads run off the UI executor, sequentially with
    /// one bounded body alive; accepted uploads always return to their owner.
    pub(crate) fn attach_paths(
        &mut self,
        scope: AttachmentScope,
        paths: Option<Vec<std::path::PathBuf>>,
        cx: &mut Context<Self>,
    ) {
        let Some(paths) = paths.filter(|paths| !paths.is_empty()) else {
            return;
        };
        if !self.attachment_connection_current(&scope) || self.attachment_drafts.uploading {
            return;
        }
        let count = if self.attachment_selected(&scope) {
            self.attachments.len()
        } else {
            self.attachment_drafts
                .shelves
                .get(&scope.key())
                .map_or(0, Vec::len)
        };
        if paths.len().saturating_add(count) > 100 {
            self.attachment_notice(&scope, "You can attach up to 100 files per message", cx);
            return;
        }
        self.attachment_drafts.uploading = true;
        self.pending_actions += 1;
        self.attachment_notice(&scope, "Uploading attachments…", cx);
        cx.spawn(async move |entity, cx| {
            for path in paths {
                let name = path.file_name().and_then(|name| name.to_str()).unwrap_or("").to_owned();
                if name.trim().is_empty() || name.chars().count() > 255 {
                    let _ = entity.update(cx, |app, cx| app.attachment_notice(&scope, "Invalid attachment filename", cx));
                    continue;
                }
                let mime = attachment_mime(&name).to_owned();
                let image = mime.starts_with("image/");
                let bytes = cx.background_executor().spawn(async move { read_attachment(&path, image) }).await;
                let bytes = match bytes {
                    Ok(bytes) => bytes,
                    Err(error) => {
                        let _ = entity.update(cx, |app, cx| app.attachment_notice(&scope, error, cx));
                        continue;
                    }
                };
                let ready = entity.update(cx, |app, _| app.attachment_connection_current(&scope)).unwrap_or(false);
                if !ready { break; }
                let size = bytes.len() as u64;
                let input = json!({"type":if image {"image"} else {"file"}, "name":name, "mimeType":mime, "sizeBytes":size});
                let result = connection::call(&scope.sender, "attachments.createUploadUrl", input).await;
                let target = result.and_then(|value| upload_target(&scope.origin, &value));
                let (id, url) = match target {
                    Ok(target) => target,
                    Err(_) => {
                        let _ = entity.update(cx, |app, cx| app.attachment_notice(&scope, "Could not prepare attachment upload", cx));
                        continue;
                    }
                };
                let ready = entity.update(cx, |app, _| app.attachment_connection_current(&scope)).unwrap_or(false);
                if !ready { break; }
                let result = connection::upload(&scope.sender, url, mime.clone(), bytes).await;
                let _ = entity.update(cx, |app, cx| {
                    if result.is_ok() {
                        let attachment = json!({"id":id, "name":name, "mimeType":mime, "sizeBytes":size});
                        if app.attachment_selected(&scope) {
                            app.attachments.push(attachment);
                        } else {
                            app.attachment_drafts.shelves.entry(scope.key()).or_default().push(attachment);
                        }
                        app.attachment_notice(&scope, format!("Attached {name}"), cx);
                        cx.notify();
                    } else {
                        app.attachment_notice(&scope, "Attachment upload failed; retry from the original draft", cx);
                    }
                });
            }
            let _ = entity.update(cx, |app, cx| {
                app.attachment_drafts.uploading = false;
                if app.attachment_connection_current(&scope) {
                    app.pending_actions = app.pending_actions.saturating_sub(1);
                }
                cx.notify();
            });
        }).detach();
    }
}
