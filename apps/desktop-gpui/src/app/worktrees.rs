//! Checkouts for new threads, worktree operations, and the git action.

use crate::NativeApp;
use gpui_kit::{Context, Window};
use serde_json::json;
use t3code_desktop_gpui::{commands, connection, thread, workspaces};

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
        if self.selected_thread.is_none()
            && self.new_thread_checkout.as_ref()
                == Some(&(project.clone(), branch.clone(), path.clone()))
        {
            return;
        }
        if !self.can_change_checkout(cx) {
            return;
        }
        if let Some(previous) = &self.selected_thread {
            self.drafts
                .insert(previous.clone(), self.composer.read(cx).value().to_string());
        }
        self.reset_checkout_context(window, cx);
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

    /// Applies a branch picker choice to the selection: an unsent draft takes the
    /// checkout, a materialised thread moves to the ref's worktree or switches its
    /// own checkout to the ref, and without a thread it is the next new thread's.
    pub(crate) fn pick_checkout(
        &mut self,
        branch: String,
        path: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.close_branch_picker(window, cx);
        if self.selected_thread.as_ref().is_some_and(|id| {
            self.archived_shell
                .threads
                .iter()
                .any(|thread| &thread.id == id)
                && !self.shell.threads.iter().any(|thread| &thread.id == id)
        }) {
            self.set_status(
                "Restore the archived thread before changing its checkout",
                cx,
            );
            cx.notify();
            return;
        }
        let Some(thread) = self.selected_thread.as_ref().and_then(|id| {
            self.shell
                .threads
                .iter()
                .chain(self.pending_threads.iter().map(|draft| &draft.thread))
                .find(|thread| &thread.id == id)
        }) else {
            self.choose_checkout(branch, path, window, cx);
            return;
        };
        if self.worktree_busy {
            self.set_status("Wait for the worktree operation to finish", cx);
            return;
        }
        let (id, project_id) = (thread.id.clone(), thread.project_id.clone());
        let has_session = thread.session.is_some()
            || self.detail.thread.as_ref().is_some_and(|detail| {
                detail["id"] == id
                    && detail
                        .get("session")
                        .is_some_and(|session| !session.is_null())
            });
        let Some(root) = self
            .shell
            .projects
            .iter()
            .find(|p| p.id == project_id)
            .map(|p| p.workspace_root.clone())
        else {
            return;
        };
        let workspace = self.selected_workspace();
        let row = self
            .refs
            .iter()
            .find(|row| row.name == branch)
            .cloned()
            .unwrap_or(workspaces::Ref {
                name: branch.clone(),
                current: false,
                is_default: false,
                worktree_path: path,
            });
        let (cwd, path, reuse) = workspaces::checkout_target(
            &root,
            workspace.as_deref().filter(|cwd| *cwd != root),
            &row,
        );
        let cwd = if std::path::Path::new(&cwd) == std::path::Path::new(&root) {
            root.clone()
        } else {
            cwd
        };
        let path = path.filter(|path| std::path::Path::new(path) != std::path::Path::new(&root));
        if thread.branch.as_deref() == Some(branch.as_str())
            && workspace.as_deref() == Some(cwd.as_str())
        {
            return;
        }
        if !self.can_change_checkout(cx) {
            return;
        }
        if !self.shell.threads.iter().any(|thread| thread.id == id) {
            self.prepare_new_worktree = false;
            self.worktree_path.update(cx, |input, cx| {
                input.set_value(path.clone().unwrap_or_default(), window, cx)
            });
            self.new_thread_checkout = Some((project_id, branch, path));
            self.update_draft_checkout(cx);
            self.reset_checkout_context(window, cx);
            return;
        }
        let Some(sender) = self.commands.clone() else {
            self.set_status("Not connected", cx);
            return;
        };
        let context = self.checkout_context();
        let generation = self.generation;
        self.worktree_epoch += 1;
        let epoch = self.worktree_epoch;
        self.worktree_busy = true;
        let stop = has_session && workspace.as_deref() != Some(cwd.as_str());
        let handle = window.window_handle();
        self.worktree_task = Some(cx.spawn(async move |entity, cx| {
            // Receipts, not dispatch order, serialize stop -> switch -> metadata.
            let result = async {
                if stop {
                    connection::call(
                        &sender,
                        "orchestration.dispatchCommand",
                        commands::command("thread.session.stop", &id),
                    )
                    .await?;
                }
                let current = entity
                    .update(cx, |app, _| {
                        app.generation == generation
                            && app.worktree_epoch == epoch
                            && app.checkout_context() == context
                    })
                    .unwrap_or(false);
                if !current {
                    return Err("Checkout selection changed".to_owned());
                }
                let actual_branch = if !reuse {
                    let switched = connection::call(
                        &sender,
                        "vcs.switchRef",
                        json!({"cwd":cwd,"refName":branch}),
                    )
                    .await?;
                    // Null denotes a detached checkout; never restore the requested ref.
                    switched["refName"]
                        .as_str()
                        .filter(|name| !name.is_empty())
                        .map(str::to_owned)
                } else {
                    Some(branch.clone())
                };
                let current = entity
                    .update(cx, |app, _| {
                        app.generation == generation
                            && app.worktree_epoch == epoch
                            && app.checkout_context() == context
                    })
                    .unwrap_or(false);
                if !current {
                    return Err("Checkout selection changed".to_owned());
                }
                let mut payload = commands::command("thread.meta.update", &id);
                payload["branch"] = json!(actual_branch);
                payload["worktreePath"] = json!(path);
                connection::call(&sender, "orchestration.dispatchCommand", payload).await?;
                Ok(actual_branch)
            }
            .await;
            let _ = handle.update(cx, |_, window, cx| {
                entity.update(cx, |app, cx| {
                    if app.generation != generation
                        || app.worktree_epoch != epoch
                        || app.selected_project != context.0
                        || app.selected_thread != context.1
                        || (app.checkout_context() != context
                            && app.selected_workspace().as_deref() != Some(cwd.as_str()))
                    {
                        return;
                    }
                    app.worktree_busy = false;
                    match result {
                        Ok(actual_branch) => {
                            for thread in app
                                .shell
                                .threads
                                .iter_mut()
                                .chain(app.archived_shell.threads.iter_mut())
                            {
                                if thread.id == id {
                                    thread.branch = actual_branch.clone();
                                    thread.worktree_path = path.clone();
                                }
                            }
                            if let Some(detail) =
                                app.detail.thread.as_mut().filter(|t| t["id"] == id)
                            {
                                detail["branch"] = json!(actual_branch);
                                detail["worktreePath"] = json!(path);
                            }
                            app.reset_checkout_context(window, cx);
                            app.set_status("Thread checkout updated; message retained", cx);
                        }
                        Err(error) => app.set_status(error, cx),
                    }
                    cx.notify();
                })
            });
        }));
        cx.notify();
    }

    pub(crate) fn update_draft_checkout(&mut self, cx: &mut Context<Self>) {
        if !self.can_change_checkout(cx) {
            return;
        }
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
        self.invalidate_checkout_context(cx);
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
        let context = self.checkout_context();
        let base = self.worktree_ref.read(cx).value().trim().to_owned();
        let path = self.worktree_path.read(cx).value().trim().to_owned();
        let payload = match tag {
            "vcs.listRefs" => {
                let mut payload = json!({"cwd":self.selected_workspace().unwrap_or(cwd),"refresh":!more,"limit":100});
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
        if tag == "vcs.listRefs" {
            self.refs_error = None;
        }
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
                    || app.checkout_context() != context
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
                                    if !rows.is_repo {
                                        app.refs_error =
                                            Some("This directory is not a repository".into());
                                    }
                                    app.set_status(
                                        if rows.is_repo {
                                            "Branches loaded"
                                        } else {
                                            "This directory is not a repository"
                                        },
                                        cx,
                                    );
                                }
                                Err(_) => {
                                    app.refs_error = Some("Invalid branch listing".into());
                                    app.set_status("Invalid branch listing", cx)
                                }
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
                    Err(error) => {
                        if tag == "vcs.listRefs" {
                            app.refs_error = Some(error.clone());
                        }
                        app.set_status(error, cx)
                    }
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
        if !self
            .shell
            .threads
            .iter()
            .chain(self.archived_shell.threads.iter())
            .chain(self.pending_threads.iter().map(|draft| &draft.thread))
            .any(|thread| {
                thread.id == id && Some(&thread.project_id) == self.selected_project.as_ref()
            })
        {
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
        let Some(sender) = self.commands.clone() else {
            self.set_status("Not connected", cx);
            cx.notify();
            return;
        };
        let generation = self.generation;
        let context = self.checkout_context();
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
                if app.generation != generation || app.checkout_context() != context {
                    cx.notify();
                    return;
                }
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
