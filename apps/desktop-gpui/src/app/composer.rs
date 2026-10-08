//! Prompt drafts and stash, sending and queueing, attachments, and replies to agent requests.

use super::attachments::{AttachmentInput, AttachmentScope, upload_target};
use crate::{NativeApp, attachment_mime};
use gpui_kit::{App, Context, Focusable as _, KeyDownEvent, Window};
use serde_json::json;
use t3code_desktop_gpui::{commands, connection, files, mentions, queue};

impl NativeApp {
    pub(crate) fn send_shortcut(&self) -> t3code_desktop_gpui::drafts::SendShortcut {
        self.draft_store
            .as_ref()
            .map(|store| store.send_shortcut())
            .unwrap_or(self.composer_state.send_shortcut)
    }

    pub(crate) fn set_send_shortcut(
        &mut self,
        value: t3code_desktop_gpui::drafts::SendShortcut,
        cx: &mut Context<Self>,
    ) {
        self.composer_state.send_shortcut = value;
        if let Some(store) = &mut self.draft_store {
            store.set_send_shortcut(value);
            self.schedule_draft_save(cx);
        } else {
            self.set_status(
                "Send preference is session-only because native draft storage is unavailable",
                cx,
            );
        }
        cx.notify();
    }

    pub(crate) fn composer_action_key(
        &mut self,
        key: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Ok(keystroke) = gpui_kit::Keystroke::parse(key) {
            self.composer_key(
                &KeyDownEvent {
                    keystroke,
                    is_held: false,
                    prefer_character_input: false,
                },
                window,
                cx,
            );
        }
    }

    /// Capture submission before Kit's PressEnter event; menu actions keep priority.
    pub(crate) fn composer_key(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if event.prefer_character_input
            || !self.composer.read(cx).focus_handle(cx).is_focused(window)
            || self.catalog_open
            || self.mention_open(cx)
            || self.command_trigger(cx).is_some()
        {
            return;
        }
        let key = &event.keystroke.key;
        let modifiers = event.keystroke.modifiers;
        if key == "enter" {
            let requires_modifier = self
                .send_shortcut()
                .requires_modifier(&self.composer.read(cx).value());
            let modifier = modifiers.platform || modifiers.control;
            let send = (!requires_modifier || modifier)
                && (!modifiers.shift || (requires_modifier && modifier && self.turn_running()));
            if send {
                if self.turn_running() && modifier && (!requires_modifier || modifiers.shift) {
                    self.steer_prompt(window, cx);
                } else {
                    self.send_prompt(window, cx);
                }
            } else {
                self.composer
                    .update(cx, |input, cx| input.replace("\n", window, cx));
                self.remember_composer_draft(cx);
            }
            cx.stop_propagation();
            return;
        }
        if !matches!(key.as_str(), "up" | "down")
            || modifiers.shift
            || modifiers.alt
            || modifiers.platform
            || modifiers.control
            || !self.attachments.is_empty()
        {
            return;
        }
        let target = self
            .selected_thread
            .as_ref()
            .map(|thread| (self.environment_scope.clone(), thread.clone()));
        if target != self.composer_state.history_target {
            self.composer_state.history_target = target;
            self.composer_state.history_id = None;
        }
        let value = self.composer.read(cx).value().to_string();
        if value != self.composer_state.history_recalled {
            self.composer_state.history_id = None;
        }
        if self
            .selected_thread
            .as_ref()
            .and_then(|id| self.queue_observation(id))
            .is_some_and(|state| state.blocked)
        {
            return;
        }
        let cursor = self.composer.read(cx).cursor();
        if (key == "up" && value[..cursor].contains('\n'))
            || (key == "down" && value[cursor..].contains('\n'))
        {
            return;
        }
        let mut entries: Vec<(String, String)> = Vec::new();
        for message in self
            .detail
            .thread
            .as_ref()
            .and_then(|thread| thread["messages"].as_array())
            .into_iter()
            .flatten()
        {
            if message["role"] != "user" {
                continue;
            }
            let text = message["text"].as_str().unwrap_or_default().trim();
            // Never recall dangling context references. Legacy appended blocks
            // remain out of text-only recall until their full parser is ported.
            if text.is_empty()
                || text == "/compact"
                || text.contains("t3-context://")
                || text.contains("<terminal_context>")
                || text.contains("<review_comment")
                || text.contains("<element_context>")
                || text.contains("<preview_annotation>")
            {
                continue;
            }
            let Some(id) = message["id"].as_str() else {
                continue;
            };
            if entries.last().is_some_and(|(_, previous)| previous == text) {
                entries.pop();
            }
            entries.push((id.into(), text.into()));
        }
        let active = self.composer_state.history_id.as_ref().and_then(|id| {
            entries
                .iter()
                .position(|(entry, _)| entry == id)
                .or_else(|| entries.iter().rposition(|(_, text)| text == &value))
        });
        let next = if key == "up" {
            if active.is_none() && !value.is_empty() {
                return;
            }
            match active {
                Some(index) => index.checked_sub(1),
                None => entries.len().checked_sub(1),
            }
        } else {
            let Some(index) = active else {
                return;
            };
            (index + 1 < entries.len()).then_some(index + 1)
        };
        if next.is_none() && key == "up" {
            return;
        }
        if active.is_none() {
            self.composer_state.history_draft = value;
        }
        let prompt = if let Some(index) = next {
            self.composer_state.history_id = Some(entries[index].0.clone());
            entries[index].1.clone()
        } else {
            self.composer_state.history_id = None;
            self.composer_state.history_draft.clone()
        };
        self.composer_state.history_recalled = prompt.clone();
        let end = prompt.len();
        self.composer.update(cx, |input, cx| {
            input.set_value(prompt, window, cx);
            input.set_selected_range(end..end, cx);
        });
        self.remember_composer_draft(cx);
        cx.stop_propagation();
        cx.notify();
    }

    pub(crate) fn command_trigger(
        &self,
        cx: &App,
    ) -> Option<(std::ops::Range<usize>, char, String)> {
        let input = self.composer.read(cx);
        let value = input.value();
        let prefix = value.get(..input.cursor())?;
        let start = prefix
            .char_indices()
            .rfind(|(_, ch)| ch.is_whitespace())
            .map_or(0, |(index, ch)| index + ch.len_utf8());
        let marker = prefix[start..].chars().next()?;
        if marker != '$' && !(marker == '/' && start == 0) {
            return None;
        }
        Some((
            start..input.cursor(),
            marker,
            prefix[start + 1..].to_owned(),
        ))
    }

    pub(crate) fn refresh_composer_catalog(&mut self, force: bool, cx: &mut Context<Self>) {
        let Some(provider) = self.selected_provider(cx) else {
            return;
        };
        if !provider.selectable() {
            return;
        }
        let instance = provider.instance_id.clone();
        let Some(cwd) = self.selected_workspace() else {
            return;
        };
        let key = (
            self.generation,
            self.environment_scope.clone(),
            instance.clone(),
            cwd.clone(),
        );
        if !force && self.composer_state.command_scope.as_ref() == Some(&key) {
            return;
        }
        let Some(sender) = self.commands.clone() else {
            return;
        };
        self.composer_state.command_scope = Some(key.clone());
        self.composer_state.command_loading = true;
        self.composer_state.command_error = None;
        self.composer_state.command_task = Some(cx.spawn(async move |entity, cx| {
            let result = connection::call(
                &sender,
                "server.refreshProviders",
                json!({"instanceId":instance,"cwd":cwd}),
            )
            .await;
            let _ =
                entity.update(cx, |app, cx| {
                    if app.composer_state.command_scope.as_ref() == Some(&key) {
                        app.composer_state.command_loading = false;
                        if app.generation != key.0
                            || app.environment_scope != key.1
                            || app.provider.read(cx).value().as_str() != key.2
                            || app.selected_workspace().as_deref() != Some(&key.3)
                        {
                            app.composer_state.command_scope = None;
                        }
                    }
                    if app.generation != key.0
                        || app.environment_scope != key.1
                        || app.composer_state.command_scope.as_ref() != Some(&key)
                        || app.provider.read(cx).value().as_str() != key.2
                        || app.selected_workspace().as_deref() != Some(&key.3)
                    {
                        return;
                    }
                    app.composer_state.command_loading = false;
                    match result.and_then(|value| {
                        serde_json::from_value::<t3code_desktop_gpui::providers::Config>(value)
                            .map_err(|_| "Invalid provider catalog".to_owned())
                    }) {
                        Ok(config) => {
                            for refreshed in config.providers {
                                if let Some(provider) = app
                                    .provider_catalog
                                    .iter_mut()
                                    .find(|provider| provider.instance_id == refreshed.instance_id)
                                {
                                    *provider = refreshed;
                                }
                            }
                        }
                        Err(_) => app.composer_state.command_error = Some(
                            "Could not load workspace commands and skills. Retry catalog refresh."
                                .into(),
                        ),
                    }
                    cx.notify();
                });
        }));
        cx.notify();
    }

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
        if self.command_trigger(cx).is_some() {
            self.refresh_composer_catalog(false, cx);
        }
        let input = self.composer.read(cx);
        let value = input.value();
        let trigger = self
            .command_trigger(cx)
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
        let path = entry.path.clone();
        let source = mentions::file_link(&path);
        let label = path.rsplit(['/', '\\']).next().unwrap_or(&path).to_owned();
        let value = self.composer.read(cx).value().to_string();
        if value.get(range.clone()) != Some(&format!("@{query}")) {
            return;
        }
        // A space already after the token is reused rather than doubled.
        let end = range.end + usize::from(value[range.end..].starts_with(' '));
        self.close_mention(cx);
        self.composer.update(cx, |input, cx| {
            let token = gpui_kit::component::input::InlineToken::new(
                format!("mention:{path}"),
                source.clone(),
            )
            .with_label(format!("@{label}"));
            if input
                .replace_range_with_token(range.start..end, token, window, cx)
                .is_ok()
            {
                input.replace(" ", window, cx);
            } else {
                input.set_selected_range(range.start..end, cx);
                input.replace(format!("{source} "), window, cx);
            }
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
        // The global shortcut decides restore from text alone. Attachment-only
        // drafts are nonempty work and must be stashed, not restored over.
        if restore && text.trim().is_empty() && !self.attachments.is_empty() {
            self.stash_prompt(false, window, cx);
            return;
        }
        if restore {
            if self.composer_stashes().len() > 1 {
                self.composer_state.stash_picker = !self.composer_state.stash_picker;
                self.composer_state.stash_index = 0;
                let focus = self
                    .composer_state
                    .stash_focus
                    .get_or_insert_with(|| cx.focus_handle());
                focus.focus(window, cx);
                cx.notify();
                return;
            }
            if let Some(entry) = self.composer_stashes().last() {
                let id = entry.id.clone();
                self.restore_stash(&id, window, cx);
            } else if let Some(text) = self.stash_memory.pop() {
                self.composer
                    .update(cx, |input, cx| input.set_value(text, window, cx));
            } else {
                self.set_status("Nothing stashed yet", cx);
            }
            return;
        }
        if self
            .composer_state
            .pending_paste
            .as_ref()
            .is_some_and(|(scope, _)| {
                scope.origin == self.environment_scope
                    && self.selected_thread.as_deref() == Some(&scope.thread)
            })
            || self.pending_actions > 0
            || self.attachment_drafts.uploading
            || self.attachment_drafts.stash_deleting
            || self.attachment_drafts.stash_writing
        {
            self.report_progress("Wait for pending sends and uploads before stashing", cx);
            return;
        }
        if text.trim().is_empty() && self.attachments.is_empty() {
            return;
        }
        let record = t3code_desktop_gpui::drafts::StashEntry::new(text, self.attachments.clone());
        if let Some(store) = &mut self.draft_store {
            let origin = self.environment_scope.clone();
            let thread = self.selected_thread.clone();
            store.stash_record(&origin, record.clone());
            let write = match store.write_snapshot() {
                Ok(write) => write,
                Err(_) => {
                    store.take_stash(&origin, &record.id);
                    self.report_error("Could not prepare stash write; draft retained", cx);
                    return;
                }
            };
            let handle = window.window_handle();
            self.attachment_drafts.stash_writing = true;
            cx.spawn(async move |entity, cx| {
                let result = cx
                    .background_executor()
                    .spawn(async move { write.persist() })
                    .await;
                let _ = entity.update(cx, |app, cx| {
                    app.attachment_drafts.stash_writing = false;
                    if result.is_err() {
                        if let Some(store) = &mut app.draft_store {
                            store.take_stash(&origin, &record.id);
                        }
                        app.storage_error =
                            Some("Could not persist stash; original draft retained".into());
                        cx.notify();
                        return;
                    }
                    let keep = |item: &serde_json::Value| {
                        !record
                            .attachments
                            .iter()
                            .any(|saved| saved["id"] == item["id"])
                    };
                    if let Some(store) = &mut app.draft_store
                        && let Some(thread) = &thread
                    {
                        store.consume_resources(&origin, thread, &record.attachments);
                    }
                    if app.environment_scope == origin && app.selected_thread == thread {
                        app.attachments.retain(keep);
                    }
                    if let Some(thread) = &thread
                        && let Some(shelf) = app
                            .attachment_drafts
                            .shelves
                            .get_mut(&(origin.clone(), thread.clone()))
                    {
                        shelf.retain(keep);
                    }
                    cx.notify();
                });
                if result.is_ok() {
                    let _ = handle.update(cx, |_, window, cx| {
                        entity.update(cx, |app, cx| {
                            if app.environment_scope == origin
                                && app.selected_thread == thread
                                && app.composer.read(cx).value().as_str() == record.prompt
                            {
                                app.composer
                                    .update(cx, |input, cx| input.set_value("", window, cx));
                                app.remember_composer_draft(cx);
                            } else if let Some(thread) = &thread
                                && app.environment_scope == origin
                                && app.drafts.get(thread) == Some(&record.prompt)
                            {
                                app.drafts.remove(thread);
                                if let Some(store) = &mut app.draft_store {
                                    store.set(&origin, thread, String::new());
                                }
                                app.schedule_draft_save(cx);
                            }
                            cx.notify();
                        })
                    });
                }
            })
            .detach();
        } else {
            self.attachment_drafts
                .stashes
                .entry(self.environment_scope.clone())
                .or_default()
                .push(record);
            self.attachments.clear();
            self.composer
                .update(cx, |input, cx| input.set_value("", window, cx));
            self.remember_composer_draft(cx);
            self.set_status(
                "Stashed in memory only; native draft storage is unavailable",
                cx,
            );
        }
        cx.notify();
    }

    pub(crate) fn composer_stashes(&self) -> &[t3code_desktop_gpui::drafts::StashEntry] {
        if let Some(store) = &self.draft_store {
            store.stashes(&self.environment_scope)
        } else {
            self.attachment_drafts
                .stashes
                .get(&self.environment_scope)
                .map(Vec::as_slice)
                .unwrap_or_default()
        }
    }

    fn take_composer_stash(&mut self, id: &str) -> Option<t3code_desktop_gpui::drafts::StashEntry> {
        if let Some(store) = &mut self.draft_store {
            store.take_stash(&self.environment_scope, id)
        } else {
            let entries = self
                .attachment_drafts
                .stashes
                .get_mut(&self.environment_scope)?;
            let index = entries.iter().position(|entry| entry.id == id)?;
            Some(entries.remove(index))
        }
    }

    pub(crate) fn restore_stash(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        if !self.composer.read(cx).value().trim().is_empty()
            || !self.attachments.is_empty()
            || self
                .composer_state
                .pending_paste
                .as_ref()
                .is_some_and(|(scope, _)| {
                    scope.origin == self.environment_scope
                        && self.selected_thread.as_deref() == Some(&scope.thread)
                })
        {
            self.report_error(
                "Stash or clear the current prompt and attachments before restoring",
                cx,
            );
            return;
        }
        if self.selected_thread.is_none()
            || self.pending_actions > 0
            || self.attachment_drafts.uploading
            || self.attachment_drafts.stash_deleting
            || self.attachment_drafts.stash_writing
        {
            self.report_error(
                "Select a thread and wait for pending operations before restoring",
                cx,
            );
            return;
        }
        let Some(entry) = self
            .composer_stashes()
            .iter()
            .find(|entry| entry.id == id)
            .cloned()
        else {
            return;
        };
        if !entry.attachments.is_empty() {
            let Some(scope) = self.attachment_scope() else {
                self.report_error("Reconnect to verify stashed resources before restoring", cx);
                return;
            };
            self.attachment_drafts.stash_writing = true;
            let handle = window.window_handle();
            cx.spawn(async move |entity, cx| {
                let mut verified = true;
                for attachment in &entry.attachments {
                    if connection::call(&scope.sender, "assets.createUrl", json!({"resource":{"_tag":"attachment","attachmentId":attachment["id"]}})).await.is_err() {
                        verified = false;
                        break;
                    }
                }
                let _ = handle.update(cx, |_, window, cx| {
                    entity.update(cx, |app, cx| {
                        app.attachment_drafts.stash_writing = false;
                        if !verified {
                            app.report_error("Stashed resource is missing or could not be verified. Stash retained; reconnect and retry, or delete it and attach the source again.", cx);
                        } else if app.attachment_selected(&scope) && app.attachment_connection_current(&scope)
                            && app.composer.read(cx).value().trim().is_empty() && app.attachments.is_empty() {
                            app.install_stash(&entry.id, window, cx);
                        }
                        cx.notify();
                    })
                });
            }).detach();
            return;
        }
        self.install_stash(id, window, cx);
    }

    fn install_stash(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.composer_state.stash_picker = false;
        let record = if let Some(store) = &mut self.draft_store {
            store.restore_record(
                &self.environment_scope,
                self.selected_thread.as_deref().unwrap(),
                id,
            )
        } else {
            self.take_composer_stash(id)
        };
        let Some(record) = record else {
            return;
        };
        self.attachments = record.attachments;
        self.composer.update(cx, |input, cx| {
            input.set_value(
                crate::composer_state::prompt_content(&record.prompt),
                window,
                cx,
            );
            input.focus(window, cx);
        });
        self.remember_composer_draft(cx);
        self.schedule_draft_save(cx);
        cx.notify();
    }

    /// Delete owned uploads on the captured environment. Failed deletions retain
    /// the stash for retry; an environment switch never removes another stash.
    pub(crate) fn delete_stash(&mut self, id: &str, cx: &mut Context<Self>) {
        if self.attachment_drafts.stash_deleting || self.attachment_drafts.stash_writing {
            return;
        }
        let Some(record) = self
            .composer_stashes()
            .iter()
            .find(|entry| entry.id == id)
            .cloned()
        else {
            return;
        };
        if record.attachments.is_empty() {
            self.take_composer_stash(id);
            self.schedule_draft_save(cx);
            cx.notify();
            return;
        }
        let Some(sender) = self.commands.clone() else {
            self.report_error("Reconnect to delete stashed attachment resources", cx);
            return;
        };
        let origin = self.environment_scope.clone();
        self.attachment_drafts.stash_deleting = true;
        cx.spawn(async move |entity, cx| {
            let mut failed = false;
            for attachment in &record.attachments {
                if connection::call(
                    &sender,
                    "attachments.delete",
                    json!({"attachmentId":attachment["id"]}),
                )
                .await
                .is_err()
                {
                    failed = true;
                    break;
                }
            }
            let _ = entity.update(cx, |app, cx| {
                app.attachment_drafts.stash_deleting = false;
                if !failed {
                    for attachment in &record.attachments {
                        if let Some(id) = attachment["id"].as_str() {
                            app.composer_state
                                .retire_preview(&(origin.clone(), id.to_owned()));
                        }
                    }
                    if let Some(store) = &mut app.draft_store {
                        store.take_stash(&origin, &record.id);
                    } else if let Some(entries) = app.attachment_drafts.stashes.get_mut(&origin) {
                        entries.retain(|entry| entry.id != record.id);
                    }
                    app.schedule_draft_save(cx);
                }
                if app.environment_scope == origin && failed {
                    app.report_error("Stash resource deletion failed; retained for retry", cx);
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// The composer's prompt as the selected thread would send it now, with
    /// the model and modes it goes out with.
    pub(crate) fn composer_message(
        &self,
        cx: &App,
    ) -> Result<(String, queue::QueuedMessage), &'static str> {
        if self.attachment_drafts.uploading
            || self.attachment_drafts.stash_writing
            || self
                .composer_state
                .pending_paste
                .as_ref()
                .is_some_and(|(scope, _)| {
                    scope.origin == self.environment_scope
                        && self.selected_thread.as_deref() == Some(&scope.thread)
                })
        {
            return Err("Finish pending uploads or pasted text before sending");
        }
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
        if text.trim().is_empty() && self.attachments.is_empty() {
            return Err("A prompt or attachment is required");
        }
        if text.encode_utf16().count() > 120_000 {
            return Err(
                "Prompt exceeds the 120,000-character limit; shorten or split it before sending",
            );
        }
        if self.attachments.len() > 100 {
            return Err("You can attach up to 100 files per message");
        }
        let selection = self.model_selection(&thread.model_selection, cx)?;
        if !self.provider_catalog.is_empty() && self.unavailable_model_options(cx) {
            return Err(
                "Saved model traits are unavailable; reset them in the model picker before sending",
            );
        }
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
        let provider = self
            .provider_catalog
            .iter()
            .find(|provider| provider.instance_id == selection["instanceId"]);
        if provider.is_some_and(|provider| !provider.selectable()) {
            return Err(
                "Selected provider is unavailable; choose another instance or check Providers settings",
            );
        }
        let (mut runtime_mode, mut interaction_mode) = self.selected_modes();
        if let Some(provider) = provider {
            let modes = provider.runtime_modes();
            if !modes.contains(&runtime_mode.as_str()) {
                runtime_mode = provider
                    .default_runtime_mode
                    .as_deref()
                    .filter(|mode| modes.contains(mode))
                    .or_else(|| modes.first().copied())
                    .ok_or("Provider has no supported runtime mode")?
                    .to_owned();
            }
            if provider.show_interaction_mode_toggle == Some(false) {
                interaction_mode = "default".into();
            }
        }
        if let Some(mode) = self.confirmed_plan_followup_mode(&text).map_err(
            |_| "Plan follow-up mode is pending or rejected; prepare it again after the receipt",
        )? {
            interaction_mode = mode;
        }
        let message = queue::QueuedMessage {
            id: 0,
            text,
            attachments: self.attachments.clone(),
            model_selection: selection,
            runtime_mode,
            interaction_mode,
        };
        Ok((id, message))
    }

    pub(crate) fn compact_disabled_reason(&self, cx: &App) -> Option<&'static str> {
        if self.commands.is_none() {
            return Some("Reconnect before compacting");
        }
        let Some(provider) = self.selected_provider(cx) else {
            return Some("Load the provider catalog before compacting");
        };
        if !provider.selectable() {
            return Some("Provider unavailable");
        }
        if !provider
            .commands_for_cwd(self.selected_workspace().as_deref())
            .iter()
            .any(|command| command["name"] == "compact")
        {
            return Some("This provider does not advertise manual compaction");
        }
        if self
            .selected_thread
            .as_ref()
            .is_none_or(|id| !self.shell.threads.iter().any(|thread| &thread.id == id))
        {
            return Some("Send the first message before compacting");
        }
        if self
            .selected_thread
            .as_ref()
            .is_some_and(|id| self.queues.has_queued(id))
        {
            return Some("Finish or remove queued follow-ups before compacting");
        }
        if self.checkout_transition_pending() || self.pending_actions > 0 || self.turn_running() {
            return Some("Wait for the current turn or operation to finish");
        }
        if self
            .queue_observation(&self.selected_thread.clone().unwrap_or_default())
            .is_some_and(|state| state.blocked || state.connecting)
        {
            return Some("Resolve the pending agent request before compacting");
        }
        None
    }

    /// Standalone /compact never consumes or replaces the user's live draft.
    pub(crate) fn compact_context(&mut self, cx: &mut Context<Self>) {
        if self.selected_provider(cx).is_none() {
            self.report_error("Load the provider catalog before compacting", cx);
            return;
        }
        if let Some(reason) = self.compact_disabled_reason(cx) {
            self.report_error(reason, cx);
            cx.notify();
            return;
        }
        let Some(id) = self.selected_thread.clone() else {
            return;
        };
        let Some(selection) = self.current_model_selection(cx) else {
            return;
        };
        let (runtime, interaction) = self.selected_modes();
        match commands::start_turn(&id, "/compact", &[], selection, &runtime, &interaction) {
            Ok(command) => self.perform("orchestration.dispatchCommand", command, None, cx),
            Err(error) => self.report_error(error, cx),
        }
    }

    /// Enter follows the preference; the alternate submission reverses it.
    pub(crate) fn send_prompt(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.submit_follow_up(false, window, cx);
    }

    fn submit_follow_up(&mut self, alternate: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.checkout_transition_pending() {
            self.report_error(
                "Finish checkout reconciliation before sending; message retained",
                cx,
            );
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
            self.report_progress("Wait for the pending server receipt", cx);
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
            self.report_error("Resolve pending requests and reconnect before sending", cx);
            cx.notify();
            return;
        }
        let (id, message) = match self.composer_message(cx) {
            Ok(prepared) => prepared,
            Err(error) => {
                self.report_composer_validation(error, cx);
                cx.notify();
                return;
            }
        };
        match message.start_command(&id) {
            Ok(mut payload) => {
                if let Some(source) = self.plan_followup_source(&message.text) {
                    payload["sourceProposedPlan"] = source;
                }
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
                    Ok(None) => {}
                    Err(error) => {
                        self.report_error(error, cx);
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
                self.report_error(message, cx);
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
            self.report_error(
                "Finish checkout reconciliation before sending; message retained",
                cx,
            );
            cx.notify();
            return;
        }
        let (thread_id, message) = match self.composer_message(cx) {
            Ok(prepared) => prepared,
            Err(error) => {
                self.report_composer_validation(error, cx);
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
            self.report_error("Resolve the pending agent request before steering", cx);
            cx.notify();
            return;
        }
        let source = self.plan_followup_source(&message.text);
        let id = self.queues.push_at(
            &thread_id,
            message,
            steer,
            state.as_ref().and_then(|state| state.tool.clone()),
        );
        if let Some(source) = source {
            self.queues.set_source_plan(&thread_id, id, source);
        }
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
        self.report_progress(
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
            self.report_error(
                "Finish checkout reconciliation before sending; queued message retained",
                cx,
            );
            cx.notify();
            return;
        }
        let state = self.queue_observation(thread_id);
        if state
            .as_ref()
            .is_none_or(|state| state.blocked || state.connecting)
        {
            self.report_error("Resolve pending requests and reconnect before sending", cx);
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
            self.report_error("Not connected", cx);
            cx.notify();
            return;
        };
        let mut payload = match message.start_command(&thread_id) {
            Ok(payload) => payload,
            Err(error) => {
                self.queues.settle(&thread_id, message.id, false);
                self.report_error(error, cx);
                cx.notify();
                return;
            }
        };
        if let Some(source) = self.queues.source_plan(&thread_id, message.id) {
            payload["sourceProposedPlan"] = source;
        }
        match self.materialized_checkout_bootstrap(&thread_id) {
            Ok(Some(bootstrap)) => payload["bootstrap"] = bootstrap,
            Ok(None) => {}
            Err(error) => {
                self.queues.settle(&thread_id, message.id, false);
                self.report_error(error, cx);
                cx.notify();
                return;
            }
        }
        let generation = self.generation;
        let scope = self.environment_scope.clone();
        self.report_progress("Sending queued message…", cx);
        cx.notify();
        cx.spawn(async move |entity, cx| {
            let ready = entity
                .update(cx, |app, cx| {
                    let ready = app.generation == generation
                        && app.environment_scope == scope
                        && !app.checkout_transition_pending();
                    if !ready && app.environment_scope == scope {
                        app.queues.settle(&thread_id, message.id, false);
                        cx.notify();
                    }
                    ready
                })
                .unwrap_or(false);
            if !ready {
                return;
            }
            let result = connection::call(&sender, "orchestration.dispatchCommand", payload).await;
            let _ = entity.update(cx, |app, cx| {
                if app.generation != generation {
                    if app.environment_scope == scope {
                        app.queues.settle(&thread_id, message.id, false);
                        cx.notify();
                    }
                    return;
                }
                if result.is_ok() {
                    app.consume_sent_attachments(&scope, &thread_id, &message.attachments);
                    app.schedule_draft_save(cx);
                }
                app.queues.settle(&thread_id, message.id, result.is_ok());
                app.flush_queues(cx);
                match result {
                    Ok(_) => app.report_success("Queued message sent", cx),
                    Err(error) => app.report_error(format!("Queued message not sent: {error}"), cx),
                };
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
                self.report_error(error, cx);
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
        if let Some(store) = &self.draft_store {
            let thread = self.selected_thread.as_deref().unwrap_or_default();
            for reference in store.resources(&self.environment_scope, thread) {
                if !self
                    .attachments
                    .iter()
                    .any(|live| live["id"] == reference["id"])
                    && !self.queues.messages(thread).iter().any(|message| {
                        message
                            .attachments
                            .iter()
                            .any(|queued| queued["id"] == reference["id"])
                    })
                {
                    self.attachments.push(reference);
                }
            }
        }
    }

    /// Consume only refs actually transmitted by an accepted send. Failed sends
    /// never call this; a late success must not erase attachments added meanwhile.
    pub(crate) fn consume_sent_attachments(
        &mut self,
        origin: &str,
        thread: &str,
        sent: &[serde_json::Value],
    ) {
        if let Some(store) = &mut self.draft_store {
            store.consume_resources(origin, thread, sent);
        }
        for attachment in sent {
            if let Some(id) = attachment["id"].as_str() {
                self.composer_state
                    .retire_preview(&(origin.to_owned(), id.to_owned()));
            }
        }
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
        outcome: t3code_desktop_gpui::notifications::Outcome,
        text: impl Into<String>,
        cx: &mut Context<Self>,
    ) {
        if self.attachment_selected(scope) {
            self.report_outcome(outcome, text, cx);
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
                    app.schedule_draft_save(cx);
                    cx.notify();
                } else {
                    app.attachment_notice(
                        &scope,
                        t3code_desktop_gpui::notifications::Outcome::Error,
                        "Attachment delete failed; attachment retained",
                        cx,
                    );
                }
            });
        })
        .detach();
    }

    pub(crate) fn paste_composer(
        &mut self,
        clipboard: &gpui_kit::ClipboardItem,
        cx: &mut Context<Self>,
    ) -> bool {
        let mut inputs = Vec::new();
        for entry in &clipboard.entries {
            match entry {
                gpui_kit::ClipboardEntry::ExternalPaths(paths) => {
                    inputs.extend(paths.0.iter().cloned().map(AttachmentInput::Path))
                }
                gpui_kit::ClipboardEntry::Image(image) => {
                    if !matches!(
                        image.format,
                        gpui_kit::ImageFormat::Png
                            | gpui_kit::ImageFormat::Jpeg
                            | gpui_kit::ImageFormat::Webp
                            | gpui_kit::ImageFormat::Gif
                    ) {
                        self.report_error(
                            "Clipboard image format unsupported; export PNG, JPEG, WebP or GIF",
                            cx,
                        );
                        cx.notify();
                        return true;
                    }
                    inputs.push(AttachmentInput::Bytes {
                        name: format!(
                            "clipboard-{}.{}",
                            uuid::Uuid::new_v4(),
                            image.format.extension()
                        ),
                        bytes: image.bytes.clone(),
                        source: None,
                    });
                }
                _ => {}
            }
        }
        if !inputs.is_empty() {
            if let Some(scope) = self.attachment_scope() {
                self.attach_inputs(scope, inputs, cx);
            } else {
                self.report_error(
                    "Select a connected thread before pasting files or images",
                    cx,
                );
                cx.notify();
            }
            return true;
        }
        let Some(text) = clipboard.text() else {
            return false;
        };
        if text.len() < 32 * 1024
            && self.composer.read(cx).value().encode_utf16().count() + text.encode_utf16().count()
                <= 120_000
        {
            return false;
        }
        let Some(scope) = self.attachment_scope() else {
            return false;
        };
        if self.composer_state.pending_paste.is_some() {
            self.report_progress("Finish or cancel the previous large paste first", cx);
        } else {
            self.composer_state.pending_paste = Some((scope, text));
        }
        cx.notify();
        true
    }

    pub(crate) fn finish_large_paste(
        &mut self,
        as_text: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((mut scope, text)) = self.composer_state.pending_paste.take() else {
            return;
        };
        if self.environment_scope != scope.origin
            || self.selected_thread.as_deref() != Some(&scope.thread)
        {
            self.composer_state.pending_paste = Some((scope, text));
            self.report_error(
                "Return to the original connected draft to finish this paste",
                cx,
            );
            return;
        }
        if !as_text && !self.attachment_connection_current(&scope) {
            if let Some(current) = self.attachment_scope() {
                scope = current;
            } else {
                self.composer_state.pending_paste = Some((scope, text));
                self.set_status(
                    "Reconnect to attach pasted text, or choose Paste as text",
                    cx,
                );
                return;
            }
        }
        if as_text {
            self.composer
                .update(cx, |input, cx| input.replace(text, window, cx));
            self.remember_composer_draft(cx);
        } else if self.attachment_drafts.uploading
            || self.attachments.len() >= 100
            || text.len() > 50 * 1024 * 1024
        {
            self.composer_state.pending_paste = Some((scope, text));
            self.set_status("Cannot attach yet: wait for uploads, free an attachment slot, or keep text below 50 MiB", cx);
        } else {
            let mut sequence = 1;
            let name = loop {
                let name = if sequence == 1 {
                    "pasted-text.txt".to_owned()
                } else {
                    format!("pasted-text-{sequence}.txt")
                };
                if !self
                    .attachments
                    .iter()
                    .any(|attachment| attachment["name"] == name)
                {
                    break name;
                }
                sequence += 1;
            };
            self.composer_state.pending_paste = Some((scope.clone(), text.clone()));
            self.attach_inputs(
                scope,
                vec![AttachmentInput::Bytes {
                    name,
                    bytes: text.into_bytes(),
                    source: Some(json!({"_tag":"pasted-text"})),
                }],
                cx,
            );
        }
        cx.notify();
    }

    /// Reject a missing target before opening the picker. Cancel has no effect
    /// on status, pending work, text or attachment context.
    pub(crate) fn attach_files(&mut self, cx: &mut Context<Self>) {
        if self.selected_thread.is_none() {
            self.report_thread_required(cx);
            cx.notify();
            return;
        }
        let Some(scope) = self.attachment_scope() else {
            self.report_error("Not connected", cx);
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
        self.attach_inputs(
            scope,
            paths.into_iter().map(AttachmentInput::Path).collect(),
            cx,
        );
    }

    pub(crate) fn attach_inputs(
        &mut self,
        scope: AttachmentScope,
        inputs: Vec<AttachmentInput>,
        cx: &mut Context<Self>,
    ) {
        if inputs.is_empty() {
            return;
        }
        if !self.attachment_connection_current(&scope) {
            return;
        }
        if self.attachment_drafts.uploading {
            self.attachment_notice(
                &scope,
                t3code_desktop_gpui::notifications::Outcome::Progress,
                "Wait for the current attachment upload before adding more files",
                cx,
            );
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
        if inputs.len().saturating_add(count) > 100 {
            self.attachment_notice(
                &scope,
                t3code_desktop_gpui::notifications::Outcome::Error,
                "You can attach up to 100 files per message",
                cx,
            );
            return;
        }
        self.attachment_drafts.uploading = true;
        self.pending_actions += 1;
        self.attachment_notice(
            &scope,
            t3code_desktop_gpui::notifications::Outcome::Progress,
            "Uploading attachments…",
            cx,
        );
        cx.spawn(async move |entity, cx| {
            for input in inputs {
                let name = input.name();
                let source = input.source();
                if name.trim().is_empty() || name.encode_utf16().count() > 255 {
                    let _ = entity.update(cx, |app, cx| app.attachment_notice(&scope, t3code_desktop_gpui::notifications::Outcome::Error, "Invalid attachment filename", cx));
                    continue;
                }
                let mime = attachment_mime(&name).to_owned();
                let image = mime.starts_with("image/");
                let bytes = cx.background_executor().spawn(async move { input.read(image) }).await;
                let bytes = match bytes {
                    Ok(bytes) => bytes,
                    Err(error) => {
                        let _ = entity.update(cx, |app, cx| app.attachment_notice(&scope, t3code_desktop_gpui::notifications::Outcome::Error, error, cx));
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
                        let _ = entity.update(cx, |app, cx| app.attachment_notice(&scope, t3code_desktop_gpui::notifications::Outcome::Error, "Could not prepare attachment upload", cx));
                        continue;
                    }
                };
                let ready = entity.update(cx, |app, _| app.attachment_connection_current(&scope)).unwrap_or(false);
                if !ready { break; }
                let (bytes, preview) = cx.background_executor().spawn(async move {
                    let preview = image.then(|| crate::composer_thumbnail::thumbnail(&bytes)).flatten();
                    (bytes, preview)
                }).await;
                let result = connection::upload(&scope.sender, url, mime.clone(), bytes).await;
                let _ = entity.update(cx, |app, cx| {
                    if result.is_ok() {
                        if let Some(preview) = preview { app.composer_state.cache_preview(scope.origin.clone(), id.clone(), preview); }
                        let mut attachment = json!({"type":if image {"image"} else {"file"}, "id":id, "name":name, "mimeType":mime, "sizeBytes":size});
                        if source.as_ref().is_some_and(|source| source["_tag"] == "pasted-text")
                            && app.composer_state.pending_paste.as_ref().is_some_and(|(owner, _)| owner.key() == scope.key()) {
                            app.composer_state.pending_paste = None;
                        }
                        if let Some(source) = source { attachment["source"] = source; }
                        if app.attachment_selected(&scope) {
                            app.attachments.push(attachment);
                        } else {
                            app.attachment_drafts.shelves.entry(scope.key()).or_default().push(attachment);
                        }
                        app.attachment_notice(&scope, t3code_desktop_gpui::notifications::Outcome::Success, format!("Attached {name}"), cx);
                        cx.notify();
                    } else {
                        app.attachment_notice(&scope, t3code_desktop_gpui::notifications::Outcome::Error, "Attachment upload failed; retry from the original draft", cx);
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
