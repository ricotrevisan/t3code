//! Workspace listing, the file editor, content search, and opening in an editor.

use crate::NativeApp;
use gpui_kit::component::input::Position;
use gpui_kit::{Context, Window};
use serde_json::json;
use t3code_desktop_gpui::{connection, files};

impl NativeApp {
    /// All selection entry points retire checkout-scoped requests and confirmations.
    pub(crate) fn reset_checkout_context(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let dirty = self
            .file_editor
            .dirty(self.file_contents.read(cx).value().as_str());
        self.invalidate_checkout_context(cx);
        if dirty {
            return;
        }
        self.file_contents
            .update(cx, |input, cx| input.set_value("", window, cx));
    }

    /// Context changes require the existing file Save/Discard flow first.
    pub(crate) fn can_change_checkout(&mut self, cx: &mut Context<Self>) -> bool {
        if self
            .file_editor
            .dirty(self.file_contents.read(cx).value().as_str())
        {
            self.report_error(
                "Unsaved file edits: Save or Discard unsaved file edits before changing selection",
                cx,
            );
            cx.notify();
            return false;
        }
        true
    }

    pub(crate) fn invalidate_checkout_context(&mut self, cx: &mut Context<Self>) {
        self.cancel_branch_workflow(cx);
        self.header_rename = Default::default();
        self.editor_choice.open = false;
        self.git_header.open = false;
        self.git_header.status = None;
        self.git_header.epoch = self.git_header.epoch.wrapping_add(1);
        self.diffs.invalidate();
        self.file_listing_epoch += 1;
        self.file_epoch += 1;
        self.worktree_epoch += 1;
        self.file_listing_task = None;
        self.worktree_task = None;
        self.action_task = None;
        self.file_busy = false;
        self.worktree_busy = false;
        self.action_busy = false;
        self.file_entries.clear();
        self.file_listing_truncated = false;
        self.content_results = None;
        self.content_index = 0;
        // An edit can arrive while a checkout RPC is awaiting its receipt.
        // Retain its original target so Save/Discard remain usable afterward.
        if !self
            .file_editor
            .dirty(self.file_contents.read(cx).value().as_str())
        {
            self.file_editor = files::Editor::default();
        }
        self.refs.clear();
        self.refs_cursor = None;
        self.refs_error = None;
        self.worktree_outcome = None;
        self.setup_progress = None;
        self.action_edit_id = None;
        self.confirm_action_delete = None;
        self.confirm_worktree_removal = None;
        self.confirm_project_removal = None;
        self.confirm_delete = None;
        cx.notify();
    }

    pub(crate) fn checkout_context(&self) -> (Option<String>, Option<String>, Option<String>) {
        (
            self.selected_project.clone(),
            self.selected_thread.clone(),
            self.selected_workspace(),
        )
    }

    pub(crate) fn list_files(&mut self, search: bool, cx: &mut Context<Self>) {
        let Some(sender) = self.commands.clone() else {
            self.report_error("Not connected", cx);
            cx.notify();
            return;
        };
        let Some(project) = self
            .selected_project
            .as_ref()
            .and_then(|id| self.shell.projects.iter().find(|project| &project.id == id))
        else {
            self.report_error("Select a project first", cx);
            cx.notify();
            return;
        };
        let cwd = self
            .selected_workspace()
            .unwrap_or_else(|| project.workspace_root.clone());
        let query = self.file_path.read(cx).value().to_string();
        let payload = if search {
            json!({"cwd":cwd,"query":query,"limit":100})
        } else {
            json!({"cwd":cwd,"directoryPath":query.trim()})
        };
        self.file_listing_epoch += 1;
        let epoch = self.file_listing_epoch;
        let generation = self.generation;
        let context = self.checkout_context();
        self.file_listing_task = Some(cx.spawn(async move |entity, cx| {
            let result = connection::call(
                &sender,
                if search {
                    "projects.searchEntries"
                } else {
                    "projects.listEntries"
                },
                payload,
            )
            .await;
            let _ = entity.update(cx, |app, cx| {
                if app.generation != generation
                    || app.file_listing_epoch != epoch
                    || app.checkout_context() != context
                {
                    return;
                }
                match result {
                    Ok(value) => match serde_json::from_value::<files::Listing>(value) {
                        Ok(list) => {
                            app.file_entries = list.entries;
                            app.file_listing_truncated = list.truncated;
                            app.set_status("Workspace entries loaded", cx);
                        }
                        Err(_) => app.report_error("Invalid workspace listing", cx),
                    },
                    Err(error) => app.report_error(error, cx),
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    pub(crate) fn search_contents(&mut self, cx: &mut Context<Self>) {
        let Some(sender) = self.commands.clone() else {
            self.report_error("Not connected", cx);
            cx.notify();
            return;
        };
        let Some(project) = self
            .selected_project
            .as_ref()
            .and_then(|id| self.shell.projects.iter().find(|project| &project.id == id))
        else {
            self.report_error("Select a project first", cx);
            cx.notify();
            return;
        };
        let payload = match self.content_options.request(
            &self
                .selected_workspace()
                .unwrap_or_else(|| project.workspace_root.clone()),
            self.content_query.read(cx).value().as_str(),
        ) {
            Ok(payload) => payload,
            Err(error) => {
                self.report_error(error, cx);
                cx.notify();
                return;
            }
        };
        self.file_listing_epoch += 1;
        let epoch = self.file_listing_epoch;
        let generation = self.generation;
        let context = self.checkout_context();
        self.content_results = None;
        self.content_request_query = self.content_query.read(cx).value().to_string();
        self.content_index = 0;
        self.file_listing_task = Some(cx.spawn(async move |entity, cx| {
            let result = connection::call(&sender, "projects.searchContents", payload).await;
            let _ = entity.update(cx, |app, cx| {
                if app.generation != generation
                    || app.file_listing_epoch != epoch
                    || app.checkout_context() != context
                {
                    return;
                }
                match result {
                    Ok(value) => match serde_json::from_value::<files::ContentResults>(value) {
                        Ok(results) => {
                            if let Some(error) = results.regex_fallback_error.clone() {
                                app.report_error(
                                    format!("Regex failed; showing literal matches: {error}"),
                                    cx,
                                );
                            } else {
                                app.set_status("Workspace content results loaded", cx);
                            };
                            app.content_results = Some(results);
                        }
                        Err(_) => app.report_error("Invalid workspace content results", cx),
                    },
                    Err(error) => app.report_error(error, cx),
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    pub(crate) fn move_content_hit(&mut self, forward: bool, cx: &mut Context<Self>) {
        let count = self
            .content_results
            .as_ref()
            .map_or(0, |results| results.matches.len());
        if count > 0 {
            self.content_index = if forward {
                (self.content_index + 1) % count
            } else {
                (self.content_index + count - 1) % count
            };
            self.content_scroll.scroll_to_item(self.content_index);
            cx.notify();
        }
    }

    pub(crate) fn open_content_hit(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(hit) = self
            .content_results
            .as_ref()
            .and_then(|results| results.matches.get(index))
            .cloned()
        else {
            return;
        };
        self.content_index = index;
        self.file_path.update(cx, |input, cx| {
            input.set_value(hit.path.clone(), window, cx)
        });
        let column = hit.character_column();
        self.file_operation_at(
            false,
            Some(Position::new(
                hit.line_number.saturating_sub(1) as u32,
                column as u32,
            )),
            window,
            cx,
        );
    }

    pub(crate) fn file_operation(
        &mut self,
        write: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.file_operation_at(write, None, window, cx);
    }

    pub(crate) fn file_operation_at(
        &mut self,
        write: bool,
        position: Option<Position>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.file_busy {
            self.report_progress("A file operation is already pending", cx);
            cx.notify();
            return;
        }
        let Some(sender) = self.commands.clone() else {
            self.report_error("Not connected", cx);
            cx.notify();
            return;
        };
        let text = self.file_contents.read(cx).value().to_string();
        let target = if write {
            if !self.file_editor.writable() {
                self.report_error(
                    "Read a complete workspace-relative text file before saving",
                    cx,
                );
                cx.notify();
                return;
            }
            if self.file_environment != self.environment_scope
                || self.file_editor.target.as_ref().is_none_or(|target| {
                    !self
                        .shell
                        .projects
                        .iter()
                        .any(|project| project.workspace_root == target.cwd)
                        && !self
                            .shell
                            .threads
                            .iter()
                            .chain(self.archived_shell.threads.iter())
                            .any(|thread| {
                                thread.worktree_path.as_deref() == Some(target.cwd.as_str())
                            })
                })
            {
                self.report_error("The file belongs to a disconnected workspace", cx);
                cx.notify();
                return;
            }
            self.file_editor.target.clone().unwrap()
        } else {
            if self.file_editor.dirty(&text) {
                self.report_error("Unsaved file edits: save before reading another file", cx);
                cx.notify();
                return;
            }
            let Some(project) = self
                .selected_project
                .as_ref()
                .and_then(|id| self.shell.projects.iter().find(|project| &project.id == id))
            else {
                self.report_error("Select a project first", cx);
                cx.notify();
                return;
            };
            let path = self.file_path.read(cx).value().trim().to_owned();
            if path.is_empty() {
                self.report_error("Enter a file path", cx);
                cx.notify();
                return;
            }
            files::Target {
                cwd: self
                    .selected_workspace()
                    .unwrap_or_else(|| project.workspace_root.clone()),
                path,
            }
        };
        self.file_busy = true;
        self.file_epoch += 1;
        let epoch = self.file_epoch;
        let generation = self.generation;
        let handle = window.window_handle();
        let context = self.checkout_context();
        let payload = if write {
            json!({"cwd":target.cwd,"relativePath":target.path,"contents":text})
        } else {
            json!({"cwd":target.cwd,"relativePath":target.path})
        };
        cx.spawn(async move |entity,cx| {
            let result=connection::call(&sender,if write { "projects.writeFile" } else { "projects.readFile" },payload).await;
            let _=handle.update(cx,|_,window,cx|entity.update(cx,|app,cx| {
                if app.generation!=generation || app.file_epoch!=epoch || app.checkout_context()!=context { return; }
                app.file_busy=false;
                match result {
                    Ok(_) if write=>{ app.file_editor.saved=text;app.report_success("File saved", cx); }
                    Ok(value)=>match serde_json::from_value::<files::Read>(value) {
                        Ok(read)=>{
                            if app.file_contents.read(cx).value().as_str()!=text {
                                app.report_error("File response arrived after an edit; edits preserved. Save or discard before retrying.", cx);cx.notify();return;
                            }
                            app.file_contents.update(cx,|input,cx| {
                                input.set_value(read.contents.clone(),window,cx);
                                if let Some(mut position) = position {
                                    let lines = read.contents.split('\n').collect::<Vec<_>>();
                                    position.line = position.line.min(lines.len().saturating_sub(1) as u32);
                                    position.character = position.character.min(lines[position.line as usize].chars().count() as u32);
                                    input.set_cursor_position(position,window,cx);
                                }
                            });
                            app.set_status(if read.truncated { "File truncated; saving disabled" } else { "File loaded" }, cx);
                            app.file_environment=app.environment_scope.clone();
                            app.file_editor.loaded(target.cwd,read);
                        }
                        Err(_)=>app.report_error("Invalid file response; existing edits preserved", cx),
                    },
                    Err(error)=>app.report_error(error, cx),
                }
                cx.notify();
            }));
        }).detach();
        cx.notify();
    }

    /// Open the selected thread's worktree (or the project root) in the first
    /// available editor, preferring Zed to match the app's design language.
    pub(crate) fn open_in_editor(&mut self, cx: &mut Context<Self>) {
        if self.selected_thread.as_ref().is_some_and(|id| {
            !self
                .shell
                .threads
                .iter()
                .chain(self.archived_shell.threads.iter())
                .chain(self.pending_threads.iter().map(|draft| &draft.thread))
                .any(|thread| {
                    &thread.id == id && Some(&thread.project_id) == self.selected_project.as_ref()
                })
        }) {
            self.report_error("Select a valid thread first", cx);
            cx.notify();
            return;
        }
        let Some(cwd) = self
            .selected_workspace()
            .filter(|path| !path.trim().is_empty())
        else {
            self.report_error("Select a project or open a worktree first", cx);
            cx.notify();
            return;
        };
        if let Some(reason) = self.editor_unavailable_reason() {
            self.report_error(reason, cx);
            return;
        }
        let Some(editor) = self.preferred_editor() else {
            return;
        };
        if self.is_remote_editor_checkout() {
            if let Some(host) = self.remote_editor_host()
                && let Some(link) = super::git_editors::remote_editor_url(&editor, host, &cwd)
            {
                cx.open_url(&link);
            } else {
                self.report_error("This editor cannot open remote checkouts", cx);
            }
            return;
        }
        self.perform(
            "shell.openInEditor",
            json!({"cwd": cwd, "editor": editor}),
            None,
            cx,
        );
    }
}
