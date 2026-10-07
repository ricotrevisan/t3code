//! Checkouts for new threads, worktree operations, and the git action.

use crate::NativeApp;
use gpui_kit::{Context, Window};
use serde_json::json;
use t3code_desktop_gpui::{connection, thread, workspaces};

impl NativeApp {
    pub(crate) fn selected_setup(&self) -> Option<workspaces::SetupProgress> {
        self.setup_progress.clone().or_else(|| {
            self.detail
                .thread
                .as_ref()
                .and_then(|thread| thread["activities"].as_array())
                .and_then(|activities| {
                    activities
                        .iter()
                        .rev()
                        .find(|activity| activity["kind"] == "worktree-setup")
                })
                .and_then(|activity| serde_json::from_value(activity["payload"].clone()).ok())
        })
    }

    pub(crate) fn choose_checkout(
        &mut self,
        branch: String,
        path: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(project) = self.selected_project.clone() else {
            return;
        };
        if let Some(previous) = &self.selected_thread {
            self.drafts
                .insert(previous.clone(), self.composer.read(cx).value().to_string());
        }
        self.selected_thread = None;
        self.detail = thread::ThreadDetail::default();
        self.worktree_path.update(cx, |input, cx| {
            input.set_value(path.clone().unwrap_or_default(), window, cx)
        });
        self.new_thread_checkout = Some((project, branch, path));
        self.composer
            .update(cx, |input, cx| input.set_value("", window, cx));
        if let Some(sender) = &self.commands {
            let _ = sender.try_send(connection::Command::SelectThread(None));
        }
        self.set_status("Checkout selected for the next new thread", cx);
        cx.notify();
    }

    pub(crate) fn update_draft_checkout(&mut self, cx: &mut Context<Self>) {
        if self.pending_actions > 0 {
            self.set_status("Wait for the pending server receipt", cx);
            cx.notify();
            return;
        }
        let Some(draft) = self.pending_threads.iter_mut().find(|draft| {
            Some(&draft.thread.id) == self.selected_thread.as_ref()
                && !self
                    .shell
                    .threads
                    .iter()
                    .any(|thread| thread.id == draft.thread.id)
        }) else {
            self.set_status("Select an unsent draft to change its checkout", cx);
            cx.notify();
            return;
        };
        let Some(project) = self
            .shell
            .projects
            .iter()
            .find(|project| project.id == draft.thread.project_id)
        else {
            return;
        };
        let base = self.worktree_ref.read(cx).value().trim().to_owned();
        if self.prepare_new_worktree && base.is_empty() {
            self.set_status("Select a base branch for the new worktree", cx);
            cx.notify();
            return;
        }
        draft.prepare_worktree = self.prepare_new_worktree.then(|| {
            let mut recipe = json!({"projectCwd":project.workspace_root,"baseBranch":base,"startFromOrigin":self.start_from_origin});
            let branch = self.worktree_branch.read(cx).value().trim().to_owned();
            if !branch.is_empty() { recipe["branch"] = json!(branch); }
            recipe
        });
        draft.run_setup_script = self.run_setup_script;
        draft.thread.branch = None;
        draft.thread.worktree_path = None;
        if !self.prepare_new_worktree
            && let Some((project_id, branch, path)) = &self.new_thread_checkout
            && project_id == &project.id
        {
            draft.thread.branch = Some(branch.clone());
            draft.thread.worktree_path = path.clone();
        }
        if let Some(store) = &mut self.draft_store {
            store.put_pending(&self.environment_scope, draft.clone());
        }
        self.setup_progress = None;
        self.schedule_draft_save(cx);
        self.set_status("Draft checkout updated; message retained", cx);
        cx.notify();
    }

    pub(crate) fn worktree_operation(&mut self, tag: &str, more: bool, cx: &mut Context<Self>) {
        if self.worktree_busy {
            self.set_status("Wait for the worktree operation to finish", cx);
            cx.notify();
            return;
        }
        let Some(project) = self
            .shell
            .projects
            .iter()
            .find(|p| Some(&p.id) == self.selected_project.as_ref())
        else {
            self.set_status("Select a project first", cx);
            cx.notify();
            return;
        };
        let Some(sender) = self.commands.clone() else {
            self.set_status("Not connected", cx);
            cx.notify();
            return;
        };
        let cwd = project.workspace_root.clone();
        let project_id = project.id.clone();
        let base = self.worktree_ref.read(cx).value().trim().to_owned();
        let path = self.worktree_path.read(cx).value().trim().to_owned();
        let payload = match tag {
            "vcs.listRefs" => {
                let mut payload = json!({"cwd":cwd,"refresh":!more,"limit":100});
                if more {
                    if let Some(cursor) = self.refs_cursor {
                        payload["cursor"] = json!(cursor);
                    } else {
                        return;
                    }
                }
                payload
            }
            "vcs.createWorktree" => match workspaces::create_worktree(
                &cwd,
                &base,
                self.worktree_branch.read(cx).value().as_str(),
                &path,
            ) {
                Ok(value) => value,
                Err(error) => {
                    self.set_status(error, cx);
                    cx.notify();
                    return;
                }
            },
            "vcs.switchRef" if !base.is_empty() => {
                json!({"cwd":self.selected_workspace().unwrap_or(cwd),"refName":base})
            }
            "vcs.removeWorktree" if !path.is_empty() => {
                if self.confirm_worktree_removal.as_ref() != Some(&path) {
                    self.confirm_worktree_removal = Some(path.clone());
                    self.set_status(
                        "Click Remove again to confirm worktree removal; dirty files are preserved",
                        cx,
                    );
                    cx.notify();
                    return;
                }
                json!({"cwd":cwd,"path":path,"force":false})
            }
            _ => {
                self.set_status("Enter a branch or worktree path", cx);
                cx.notify();
                return;
            }
        };
        self.worktree_busy = true;
        self.worktree_epoch += 1;
        let epoch = self.worktree_epoch;
        let generation = self.generation;
        let tag = tag.to_owned();
        self.worktree_task = Some(cx.spawn(async move |entity, cx| {
            let result = connection::call(&sender, &tag, payload).await;
            let _ = entity.update(cx, |app, cx| {
                if app.generation != generation
                    || app.worktree_epoch != epoch
                    || app.selected_project.as_deref() != Some(&project_id)
                {
                    return;
                }
                app.worktree_busy = false;
                match result {
                    Ok(value) => {
                        if tag == "vcs.listRefs" {
                            match serde_json::from_value::<workspaces::Refs>(value) {
                                Ok(rows) => {
                                    if !more {
                                        app.refs.clear();
                                    }
                                    app.refs.extend(rows.refs);
                                    app.refs_cursor = rows.next_cursor;
                                    app.set_status(
                                        if rows.is_repo {
                                            "Branches loaded"
                                        } else {
                                            "This directory is not a repository"
                                        },
                                        cx,
                                    );
                                }
                                Err(_) => app.set_status("Invalid branch listing", cx),
                            }
                        } else {
                            app.confirm_worktree_removal = None;
                            match tag.as_str() {
                                "vcs.createWorktree" => app.report_success("Worktree created", cx),
                                "vcs.removeWorktree" => app.report_success("Worktree removed", cx),
                                _ => app.set_status("Worktree operation completed", cx),
                            }
                            app.worktree_outcome = Some(value);
                            app.refs.clear();
                            app.refs_cursor = None;
                        }
                    }
                    Err(error) => app.set_status(error, cx),
                };
                cx.notify();
            });
        }));
        cx.notify();
    }

    pub(crate) fn selected_workspace(&self) -> Option<String> {
        let project = self
            .shell
            .projects
            .iter()
            .find(|p| Some(&p.id) == self.selected_project.as_ref())?;
        let selected = self
            .shell
            .threads
            .iter()
            .chain(self.archived_shell.threads.iter())
            .chain(self.pending_threads.iter().map(|draft| &draft.thread))
            .find(|t| Some(&t.id) == self.selected_thread.as_ref() && t.project_id == project.id);
        if selected.is_none()
            && let Some((id, _, Some(path))) = &self.new_thread_checkout
            && id == &project.id
        {
            return Some(path.clone());
        }
        Some(
            selected
                .and_then(|t| {
                    if let Some(detail) = self.detail.thread.as_ref().filter(|detail| {
                        detail["id"] == t.id && detail.get("worktreePath").is_some()
                    }) {
                        detail["worktreePath"].as_str().map(str::to_owned)
                    } else {
                        t.worktree_path.clone()
                    }
                })
                .unwrap_or_else(|| project.workspace_root.clone()),
        )
    }

    /// Commit, push, and open a pull request for the selected thread. Streams the
    /// server's progress events and reports the server's result toast when the
    /// action finishes.
    pub(crate) fn push_and_create_pr(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.selected_thread.clone() else {
            self.set_status("Select a thread first", cx);
            cx.notify();
            return;
        };
        let cwd = self
            .shell
            .threads
            .iter()
            .chain(self.pending_threads.iter().map(|draft| &draft.thread))
            .find(|thread| thread.id == id)
            .and_then(|thread| thread.worktree_path.clone())
            .filter(|path| !path.is_empty())
            .unwrap_or_else(|| self.workspace.read(cx).value().to_string());
        if cwd.trim().is_empty() {
            self.set_status("Select a project or open a worktree first", cx);
            cx.notify();
            return;
        }
        let Some(sender) = self.commands.clone() else {
            self.set_status("Not connected", cx);
            cx.notify();
            return;
        };
        self.pending_actions += 1;
        self.set_status("Running git action\u{2026}", cx);
        cx.notify();
        cx.spawn(async move |entity, cx| {
            let result = connection::call_stream(
                &sender,
                "git.runStackedAction",
                json!({
                    "actionId": uuid::Uuid::new_v4().to_string(),
                    "cwd": cwd,
                    "action": "commit_push_pr",
                    "featureBranch": true,
                    "threadId": id,
                }),
            )
            .await;
            let _ = entity.update(cx, |app, cx| {
                app.pending_actions = app.pending_actions.saturating_sub(1);
                match result {
                    Ok(events) => match events.last() {
                        Some(event) if event["kind"] == "action_failed" => app.set_status(
                            format!(
                                "Git action failed: {}",
                                event["message"].as_str().unwrap_or("unknown error")
                            ),
                            cx,
                        ),
                        Some(event) if event["kind"] == "action_finished" => {
                            let toast = &event["result"]["toast"];
                            let title = toast["title"].as_str().unwrap_or("Git action finished");
                            let status = match toast["description"].as_str() {
                                Some(description) => format!("{title} · {description}"),
                                None => title.to_owned(),
                            };
                            app.report_success(&status, cx);
                        }
                        _ => app.set_status("Git action failed: ended without a result", cx),
                    },
                    Err(error) => app.set_status(error, cx),
                }
                cx.notify();
            });
        })
        .detach();
    }
}
