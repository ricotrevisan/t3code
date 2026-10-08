//! Checkouts for new threads, worktree operations, and the git action.

use crate::NativeApp;
use gpui_kit::{Context, Window};
use serde_json::json;
use t3code_desktop_gpui::{connection, workspaces};

impl NativeApp {
    pub(crate) fn next_checkout_label(&self) -> String {
        if self.prepare_new_worktree {
            return "Next new thread: new worktree".into();
        }
        match self
            .new_thread_checkout
            .as_ref()
            .filter(|(project, _, _)| Some(project) == self.selected_project.as_ref())
        {
            Some((_, branch, Some(path))) => format!("Next new thread: {branch} · {path}"),
            Some((_, branch, None)) => format!("Next new thread: {branch} · project root"),
            None => "Next new thread: project root".into(),
        }
    }

    pub(crate) fn is_next_checkout(&self, row: &workspaces::Ref) -> bool {
        self.new_thread_checkout
            .as_ref()
            .is_some_and(|(project, branch, path)| {
                Some(project) == self.selected_project.as_ref()
                    && branch == &row.name
                    && (path.is_none() || path == &row.worktree_path)
            })
    }

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
        if self.selected_thread.is_some() && self.selected_workspace().is_none() {
            self.report_error(
                "The selected checkout is unavailable; select a valid thread",
                cx,
            );
            return;
        }
        if !self.can_change_checkout(cx) {
            return;
        }
        let Some(project) = self
            .shell
            .projects
            .iter()
            .find(|p| Some(&p.id) == self.selected_project.as_ref())
        else {
            return;
        };
        if self.prepare_new_worktree {
            self.worktree_ref
                .update(cx, |input, cx| input.set_value(branch, window, cx));
            self.set_status("Base selected for the planned new worktree", cx);
            return;
        }
        // This is a plan for New Thread, not permission to stop or update the
        // selected thread (or switch a shared physical checkout underneath it).
        if self.checkout_transition_pending() || self.worktree_busy {
            self.report_progress("Wait for the checkout operation to finish", cx);
            return;
        }
        let root = project.workspace_root.clone();
        let project_id = project.id.clone();
        let known = self.refs.iter().find(|row| row.name == branch);
        let destination = known.and_then(|row| row.worktree_path.clone());
        if let Some(cwd) = destination {
            let path = (std::path::Path::new(&cwd) != std::path::Path::new(&root)).then_some(cwd);
            self.new_thread_checkout = Some((project_id, branch, path));
            self.set_status(
                "Checkout selected for the next new thread; current thread retained",
                cx,
            );
            cx.notify();
            return;
        }
        let current_path = known
            .filter(|row| row.current)
            .and_then(|_| self.selected_workspace());
        let Some(cwd) = path.or(current_path).filter(|path| !path.trim().is_empty()) else {
            self.report_error(
                "Choose an existing worktree or create a separate worktree for the next thread",
                cx,
            );
            return;
        };
        let Some(sender) = self.commands.clone() else {
            self.report_error("Not connected", cx);
            return;
        };
        let context = self.checkout_context();
        let environment = self.environment_scope.clone();
        let generation = self.generation;
        self.worktree_epoch += 1;
        let epoch = self.worktree_epoch;
        self.worktree_busy = true;
        self.report_progress("Verifying checkout for the next new thread", cx);
        self.worktree_task = Some(cx.spawn(async move |entity, cx| {
            let result = connection::call(
                &sender,
                "vcs.listRefs",
                json!({"cwd":cwd,"refresh":true,"limit":100}),
            )
            .await;
            let _ = entity.update(cx, |app, cx| {
                if app.generation != generation
                    || app.environment_scope != environment
                    || app.worktree_epoch != epoch
                    || app.checkout_context() != context
                    || !app.worktree_busy
                {
                    return;
                }
                app.worktree_busy = false;
                if !app.can_change_checkout(cx) {
                    return;
                }
                let verified = result.and_then(|value| {
                    let rows: workspaces::Refs = serde_json::from_value(value)
                        .map_err(|_| "Invalid checkout refs".to_owned())?;
                    if !rows.is_repo {
                        return Err("The selected path is not a repository".into());
                    }
                    rows.refs
                        .into_iter()
                        .find(|row| row.current && !row.name.trim().is_empty())
                        .map(|row| row.name)
                        .ok_or_else(|| {
                            "No current branch was verified; choose another worktree".into()
                        })
                });
                match verified {
                    Ok(branch) => {
                        let path = (std::path::Path::new(&cwd) != std::path::Path::new(&root))
                            .then_some(cwd);
                        app.new_thread_checkout = Some((project_id, branch, path));
                        app.set_status(
                            "Checkout selected for the next new thread; current thread retained",
                            cx,
                        );
                    }
                    Err(error) => app.report_error(error, cx),
                }
                cx.notify();
            });
        }));
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
        if self.selected_thread.is_some() && self.selected_workspace().is_none() {
            self.report_error(
                "The selected checkout is unavailable; select a valid thread",
                cx,
            );
            return;
        }
        if self.checkout_transition_pending() {
            self.adopt_observed_checkout(&branch, window, cx);
            return;
        }
        self.close_branch_picker(window, cx);
        if self.selected_thread.as_ref().is_some_and(|id| {
            self.archived_shell
                .threads
                .iter()
                .any(|thread| &thread.id == id)
                && !self.shell.threads.iter().any(|thread| &thread.id == id)
        }) {
            self.report_error(
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
            self.report_progress("Wait for the worktree operation to finish", cx);
            return;
        }
        let (id, project_id) = (thread.id.clone(), thread.project_id.clone());
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
        let known_ref = self.refs.iter().any(|row| row.name == branch);
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
        let materialized = self.shell.threads.iter().any(|thread| thread.id == id);
        if self.prepare_new_worktree && (!materialized || self.empty_root_checkout_eligible()) {
            self.worktree_ref
                .update(cx, |input, cx| input.set_value(branch, window, cx));
            self.update_draft_checkout(cx);
            self.reset_checkout_context(window, cx);
            return;
        }
        let verify_worktree = reuse && !known_ref;
        let reuse = (reuse && known_ref).then(|| super::checkout_operations::Destination {
            branch: Some(branch.clone()),
            path: path.clone(),
            on_head: true,
        });
        self.begin_checkout_operation(
            if verify_worktree {
                "verify-existing-worktree"
            } else {
                "vcs.switchRef"
            }
            .into(),
            json!({"cwd":cwd,"refName":branch}),
            None,
            reuse,
            window,
            cx,
        );
    }

    pub(crate) fn update_draft_checkout(&mut self, cx: &mut Context<Self>) {
        if !self.can_change_checkout(cx) {
            return;
        }
        if self.pending_actions > 0 {
            self.report_progress("Wait for the pending server receipt", cx);
            cx.notify();
            return;
        }
        if self.update_materialized_checkout_recipe(cx) {
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
            self.report_error("Select an unsent draft to change its checkout", cx);
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
            self.report_error("Select a base branch for the new worktree", cx);
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
        self.report_success("Draft checkout updated; message retained", cx);
        cx.notify();
    }

    pub(crate) fn worktree_operation(&mut self, tag: &str, more: bool, cx: &mut Context<Self>) {
        if tag != "vcs.listRefs"
            && self.selected_thread.is_some()
            && self.selected_workspace().is_none()
        {
            self.report_error(
                "The selected checkout is unavailable; select a valid thread",
                cx,
            );
            return;
        }
        if self.worktree_busy {
            self.report_progress("Wait for the worktree operation to finish", cx);
            cx.notify();
            return;
        }
        let Some(project) = self
            .shell
            .projects
            .iter()
            .find(|p| Some(&p.id) == self.selected_project.as_ref())
        else {
            self.report_error("Select a project first", cx);
            cx.notify();
            return;
        };
        let Some(sender) = self.commands.clone() else {
            self.report_error("Not connected", cx);
            cx.notify();
            return;
        };
        let cwd = project.workspace_root.clone();
        let context = self.checkout_context();
        let base = self.worktree_ref.read(cx).value().trim().to_owned();
        let path = self.worktree_path.read(cx).value().trim().to_owned();
        let payload = match tag {
            "vcs.listRefs" => {
                let listing_cwd = self
                    .checkout_recovery()
                    .map(|op| op.root.clone())
                    .or_else(|| self.selected_workspace())
                    .unwrap_or(cwd);
                let mut payload = json!({"cwd":listing_cwd,"refresh":!more,"limit":100});
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
                    self.report_error(error, cx);
                    cx.notify();
                    return;
                }
            },
            "vcs.removeWorktree" if !path.is_empty() => {
                if self
                    .shell
                    .threads
                    .iter()
                    .chain(self.archived_shell.threads.iter())
                    .any(|t| t.worktree_path.as_deref() == Some(path.as_str()))
                {
                    self.report_error("Move threads out of this worktree before removing it", cx);
                    return;
                }
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
                self.report_error("Enter a branch or worktree path", cx);
                cx.notify();
                return;
            }
        };
        if tag != "vcs.listRefs" {
            if !self.can_change_checkout(cx) {
                return;
            }
            self.begin_worktree_management(tag.to_owned(), payload, cx);
            return;
        }
        self.refs_error = None;
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
                                    app.report_error("Invalid branch listing", cx)
                                }
                            }
                        } else {
                            app.confirm_worktree_removal = None;
                            match tag.as_str() {
                                "vcs.createWorktree" => app.report_success("Worktree created", cx),
                                "vcs.removeWorktree" => app.report_success("Worktree removed", cx),
                                _ => app.report_success("Worktree operation completed", cx),
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
                        app.report_error(error, cx)
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
        if self.selected_thread.is_some() && selected.is_none() {
            return None;
        }
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
    #[cfg(test)]
    pub(crate) fn push_and_create_pr(&mut self, cx: &mut Context<Self>) {
        self.run_git_action("commit_push_pr", true, cx);
    }

    pub(crate) fn run_git_action(
        &mut self,
        action: &str,
        feature_branch: bool,
        cx: &mut Context<Self>,
    ) {
        let action = action.to_owned();
        let Some(id) = self.selected_thread.clone() else {
            self.report_error("Select a thread first", cx);
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
        let Some(sender) = self.commands.clone() else {
            self.report_error("Not connected", cx);
            cx.notify();
            return;
        };
        let generation = self.generation;
        let context = self.checkout_context();
        self.pending_actions += 1;
        self.git_header.progress.clear();
        self.git_header.error = None;
        self.git_header.status = None;
        self.git_header.open = true;
        self.git_header.context = Some(context.clone());
        let commit_message = self.git_header.commit_message.take();
        let file_paths = self.git_header.file_paths.take();
        let mut payload = json!({
            "actionId": uuid::Uuid::new_v4().to_string(),
            "cwd": cwd,
            "action": action,
            "featureBranch": feature_branch,
            "threadId": id,
        });
        if let Some(message) = commit_message {
            payload["commitMessage"] = json!(message);
        }
        if let Some(paths) = file_paths {
            payload["filePaths"] = json!(paths);
        }
        self.report_progress("Running git action\u{2026}", cx);
        cx.notify();
        cx.spawn(async move |entity, cx| {
            let stream = connection::call_stream_live(&sender, "git.runStackedAction", payload).await;
            let result = match stream {
                Err(error) => Err(error),
                Ok(stream) => {
                    let mut last = None;
                    loop {
                        match stream.recv().await {
                            Ok(Ok(Some(event))) => {
                                last = Some(event.clone());
                                let _ = entity.update(cx, |app, cx| {
                                    if app.generation == generation && app.checkout_context() == context {
                                        app.record_git_progress(&event, cx);
                                    }
                                });
                            }
                            Ok(Ok(None)) => break Ok(last.into_iter().collect::<Vec<_>>()),
                            Ok(Err(error)) => break Err(error),
                            Err(_) => break Err("Connection lost before git receipt; outcome unknown. Refresh status before retrying.".into()),
                        }
                    }
                }
            };
            let _ = entity.update(cx, |app, cx| {
                app.pending_actions = app.pending_actions.saturating_sub(1);
                if app.generation != generation || app.checkout_context() != context {
                    cx.notify();
                    return;
                }
                match result {
                    Ok(events) => match events.last() {
                        Some(event) if event["kind"] == "action_failed" => app.report_error(
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
                        _ => app.report_error("Git action failed: ended without a result", cx),
                    },
                    Err(error) => {
                        app.git_header.error = Some(error.clone());
                        app.report_error(error, cx);
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }
}
