//! Workspace listing, the file editor, content search, and opening in an editor.

use crate::NativeApp;
use gpui_kit::component::input::Position;
use gpui_kit::{Context, Window};
use serde_json::json;
use t3code_desktop_gpui::{connection, files};

impl NativeApp {
    pub(crate) fn list_files(&mut self, search: bool, cx: &mut Context<Self>) {
        let Some(sender) = self.commands.clone() else {
            self.set_status("Not connected", cx);
            cx.notify();
            return;
        };
        let Some(project) = self
            .selected_project
            .as_ref()
            .and_then(|id| self.shell.projects.iter().find(|project| &project.id == id))
        else {
            self.set_status("Select a project first", cx);
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
        let project_id = self.selected_project.clone();
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
                    || app.selected_project != project_id
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
                        Err(_) => app.set_status("Invalid workspace listing", cx),
                    },
                    Err(error) => app.set_status(error, cx),
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    pub(crate) fn search_contents(&mut self, cx: &mut Context<Self>) {
        let Some(sender) = self.commands.clone() else {
            self.set_status("Not connected", cx);
            cx.notify();
            return;
        };
        let Some(project) = self
            .selected_project
            .as_ref()
            .and_then(|id| self.shell.projects.iter().find(|project| &project.id == id))
        else {
            self.set_status("Select a project first", cx);
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
                self.set_status(error, cx);
                cx.notify();
                return;
            }
        };
        self.file_listing_epoch += 1;
        let epoch = self.file_listing_epoch;
        let generation = self.generation;
        let project_id = self.selected_project.clone();
        self.content_results = None;
        self.content_request_query = self.content_query.read(cx).value().to_string();
        self.content_index = 0;
        self.file_listing_task = Some(cx.spawn(async move |entity, cx| {
            let result = connection::call(&sender, "projects.searchContents", payload).await;
            let _ = entity.update(cx, |app, cx| {
                if app.generation != generation
                    || app.file_listing_epoch != epoch
                    || app.selected_project != project_id
                {
                    return;
                }
                match result {
                    Ok(value) => match serde_json::from_value::<files::ContentResults>(value) {
                        Ok(results) => {
                            app.set_status(
                                results
                                    .regex_fallback_error
                                    .clone()
                                    .map(|error| {
                                        format!("Regex failed; showing literal matches: {error}")
                                    })
                                    .unwrap_or_else(|| "Workspace content results loaded".into()),
                                cx,
                            );
                            app.content_results = Some(results);
                        }
                        Err(_) => app.set_status("Invalid workspace content results", cx),
                    },
                    Err(error) => app.set_status(error, cx),
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
            self.set_status("A file operation is already pending", cx);
            cx.notify();
            return;
        }
        let Some(sender) = self.commands.clone() else {
            self.set_status("Not connected", cx);
            cx.notify();
            return;
        };
        let text = self.file_contents.read(cx).value().to_string();
        let target = if write {
            if !self.file_editor.writable() {
                self.set_status(
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
                self.set_status("The file belongs to a disconnected workspace", cx);
                cx.notify();
                return;
            }
            self.file_editor.target.clone().unwrap()
        } else {
            if self.file_editor.dirty(&text) {
                self.set_status("Unsaved file edits: save before reading another file", cx);
                cx.notify();
                return;
            }
            let Some(project) = self
                .selected_project
                .as_ref()
                .and_then(|id| self.shell.projects.iter().find(|project| &project.id == id))
            else {
                self.set_status("Select a project first", cx);
                cx.notify();
                return;
            };
            let path = self.file_path.read(cx).value().trim().to_owned();
            if path.is_empty() {
                self.set_status("Enter a file path", cx);
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
        let payload = if write {
            json!({"cwd":target.cwd,"relativePath":target.path,"contents":text})
        } else {
            json!({"cwd":target.cwd,"relativePath":target.path})
        };
        cx.spawn(async move |entity,cx| {
            let result=connection::call(&sender,if write { "projects.writeFile" } else { "projects.readFile" },payload).await;
            let _=handle.update(cx,|_,window,cx|entity.update(cx,|app,cx| {
                if app.generation!=generation || app.file_epoch!=epoch { return; }
                app.file_busy=false;
                match result {
                    Ok(_) if write=>{ app.file_editor.saved=text;app.set_status("File saved", cx); }
                    Ok(value)=>match serde_json::from_value::<files::Read>(value) {
                        Ok(read)=>{
                            if app.file_contents.read(cx).value().as_str()!=text {
                                app.set_status("File response arrived after an edit; edits preserved. Save or discard before retrying.", cx);cx.notify();return;
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
                        Err(_)=>app.set_status("Invalid file response; existing edits preserved", cx),
                    },
                    Err(error)=>app.set_status(error, cx),
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
            self.set_status("Select a valid thread first", cx);
            cx.notify();
            return;
        }
        let Some(cwd) = self
            .selected_workspace()
            .filter(|path| !path.trim().is_empty())
        else {
            self.set_status("Select a project or open a worktree first", cx);
            cx.notify();
            return;
        };
        let editor = self
            .available_editors
            .iter()
            .find(|editor| editor.as_str() == "zed")
            .or_else(|| {
                self.available_editors
                    .iter()
                    .find(|editor| editor.as_str() != "file-manager")
            })
            .cloned()
            .unwrap_or_else(|| "zed".into());
        self.perform(
            "shell.openInEditor",
            json!({"cwd": cwd, "editor": editor}),
            None,
            cx,
        );
    }
}
