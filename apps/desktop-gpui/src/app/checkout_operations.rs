//! A physical checkout receipt outlives selection and popover interest.
use crate::NativeApp;
use gpui_kit::{Context, Window};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use t3code_desktop_gpui::{commands, connection};

#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct Destination {
    pub branch: Option<String>,
    pub path: Option<String>,
    pub on_head: bool,
}

#[derive(Serialize, Deserialize)]
pub(crate) struct CheckoutOperation {
    pub key: String,
    #[serde(default)]
    pub intent: Value,
    pub environment: String,
    pub project: String,
    pub thread: Option<String>,
    pub root: String,
    pub context: (Option<String>, Option<String>, Option<String>),
    pub generation: u64,
    pub materialized: bool,
    pub destination: Option<Destination>,
    #[serde(default)]
    pub management_outcome: Option<Value>,
    pub error: Option<String>,
    pub running: bool,
    pub interested: bool,
    #[serde(skip)]
    pub gate: Option<connection::MutationGate>,
}

impl CheckoutOperation {
    pub(crate) fn safely_dismissible(&self) -> bool {
        !self.running
            && (self.management_outcome.is_some()
                || self.destination.as_ref().is_some_and(|d| {
                    // A prepared separate worktree may be left user-owned and unused.
                    // A changed current checkout still needs metadata reconciliation.
                    d.path.as_deref().unwrap_or(&self.root)
                        != self.context.2.as_deref().unwrap_or(&self.root)
                }))
    }
}

#[cfg(not(test))]
fn journal_path() -> std::path::PathBuf {
    t3code_desktop_gpui::drafts::default_path().with_file_name("checkout-recovery.json")
}

pub(crate) fn restore() -> Vec<CheckoutOperation> {
    #[cfg(test)]
    return Vec::new();
    #[cfg(not(test))]
    {
        let loaded = match std::fs::read(journal_path()) {
            Ok(bytes) => {
                serde_json::from_slice::<Vec<CheckoutOperation>>(&bytes).map_err(|e| e.to_string())
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
            Err(error) => Err(error.to_string()),
        };
        let mut rows = loaded.unwrap_or_else(|error| vec![CheckoutOperation {
            key: "unreadable-journal".into(), intent: Value::Null, environment: "*".into(), project: String::new(), thread: None, root: String::new(), context: (None, None, None), generation: 0, materialized: false, destination: None, management_outcome: None, error: Some(format!("Recovery journal unreadable: {error}. Repair {} before sending; checkout outcome is unknown", journal_path().display())), running: false, interested: false, gate: None,
        }]);
        for row in &mut rows {
            row.running = false;
            row.interested = false;
            if row.key != "unreadable-journal" {
                row.error = Some("Checkout interrupted: outcome may be unknown. Inspect actual Git state before adopting".into());
            }
        }
        rows
    }
}

/// Same private, atomic, fsynced replacement pattern as native draft storage.
pub(super) fn persist(path: &std::path::Path, data: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    let directory = path
        .parent()
        .ok_or_else(|| std::io::Error::other("Recovery path has no parent"))?;
    std::fs::create_dir_all(directory)?;
    let temporary = directory.join(format!(".checkout-{}.tmp", uuid::Uuid::new_v4()));
    let result = (|| {
        let mut options = std::fs::OpenOptions::new();
        options.create_new(true).write(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&temporary)?;
        file.write_all(data)?;
        file.sync_all()?;
        std::fs::rename(&temporary, path)?;
        std::fs::File::open(directory)?.sync_all()
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(temporary);
    }
    result
}

impl NativeApp {
    fn save_checkout_operations(&self) -> Result<(), String> {
        #[cfg(test)]
        return Ok(());
        #[cfg(not(test))]
        {
            if self
                .checkout_operations
                .iter()
                .any(|op| op.key == "unreadable-journal")
            {
                return Err("Recovery journal unreadable; original file preserved".into());
            }
            let path = journal_path();
            let bytes = serde_json::to_vec(&self.checkout_operations).map_err(|e| e.to_string())?;
            persist(&path, &bytes).map_err(|e| e.to_string())
        }
    }

    pub(crate) fn checkout_transition_pending(&self) -> bool {
        self.checkout_operations
            .iter()
            .any(|op| op.environment == self.environment_scope || op.environment == "*")
    }

    pub(crate) fn cancel_checkout_interest(&mut self) {
        for op in &mut self.checkout_operations {
            op.interested = false;
            if let Some(gate) = &op.gate {
                gate.cancel_before_send();
            }
        }
        let _ = self.save_checkout_operations();
    }

    pub(crate) fn checkout_recovery(&self) -> Option<&CheckoutOperation> {
        self.checkout_operations
            .iter()
            .find(|op| op.environment == self.environment_scope || op.environment == "*")
    }

    pub(crate) fn dismiss_checkout_outcome(&mut self, cx: &mut Context<Self>) {
        // Unknown or outstanding results cannot be dismissed as unchanged.
        if self
            .checkout_recovery()
            .is_none_or(|op| !op.safely_dismissible())
        {
            self.report_error(
                "This checkout still needs reconciliation; it cannot be dismissed as unchanged",
                cx,
            );
            return;
        }
        self.checkout_operations
            .retain(|op| op.environment != self.environment_scope || !op.safely_dismissible());
        let _ = self.save_checkout_operations();
        self.set_status("Known checkout outcome dismissed; Git was not undone", cx);
        cx.notify();
    }

    /// Explicit adoption of a listed physical checkout, never a switch/create.
    pub(crate) fn adopt_observed_checkout(
        &mut self,
        branch: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(op) = self.checkout_recovery() else {
            return;
        };
        if op.running || op.destination.is_some() || op.management_outcome.is_some() {
            return;
        }
        if op.context.0 != self.selected_project || op.context.1 != self.selected_thread {
            self.report_error(
                "Return to the captured thread before adopting actual Git state",
                cx,
            );
            return;
        }
        let Some(row) = self
            .refs
            .iter()
            .find(|r| r.name == branch && (r.current || r.worktree_path.is_some()))
        else {
            self.set_status("Refresh actual refs and select a current branch or existing worktree to adopt; no Git switch will run", cx);
            return;
        };
        let destination = Destination {
            branch: Some(row.name.clone()),
            path: row.worktree_path.clone().filter(|p| p != &op.root),
            on_head: true,
        };
        let key = op.key.clone();
        if !self.can_change_checkout(cx) {
            return;
        }
        let op = self
            .checkout_operations
            .iter_mut()
            .find(|op| op.key == key)
            .unwrap();
        op.destination = Some(destination);
        let _ = self.save_checkout_operations();
        self.reconcile_checkout(key, window, cx);
    }

    pub(crate) fn retry_checkout_metadata(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(op) = self.checkout_recovery() else {
            return;
        };
        if op.running || op.destination.is_none() {
            return;
        }
        if op.context.0 != self.selected_project
            || op.context.1 != self.selected_thread
            || !self
                .shell
                .projects
                .iter()
                .any(|p| p.id == op.project && p.workspace_root == op.root)
        {
            self.report_error(
                "Return to the captured project root and thread to adopt this checkout",
                cx,
            );
            return;
        }
        if !self.can_change_checkout(cx) {
            return;
        }
        let key = self.checkout_recovery().unwrap().key.clone();
        self.reconcile_checkout(key, window, cx);
    }

    pub(crate) fn begin_worktree_management(
        &mut self,
        tag: String,
        payload: Value,
        cx: &mut Context<Self>,
    ) {
        if self.checkout_transition_pending() {
            self.report_error("Resolve the previous checkout outcome first", cx);
            return;
        }
        let Some(sender) = self.commands.clone() else {
            return;
        };
        let Some(project) = self
            .shell
            .projects
            .iter()
            .find(|p| Some(&p.id) == self.selected_project.as_ref())
        else {
            return;
        };
        let key = uuid::Uuid::new_v4().to_string();
        let gate = connection::MutationGate::new();
        let scope = self.environment_scope.clone();
        let context = self.checkout_context();
        self.checkout_operations.push(CheckoutOperation {
            key: key.clone(),
            intent: json!({"tag":tag,"payload":payload}),
            environment: scope.clone(),
            project: project.id.clone(),
            thread: self.selected_thread.clone(),
            root: project.workspace_root.clone(),
            context: context.clone(),
            generation: self.generation,
            materialized: false,
            destination: None,
            management_outcome: None,
            error: None,
            running: true,
            interested: true,
            gate: Some(gate.clone()),
        });
        if let Err(error) = self.save_checkout_operations() {
            self.checkout_operations.pop();
            self.report_error(error, cx);
            return;
        }
        self.worktree_busy = true;
        cx.spawn(async move |entity, cx| {
            let current = entity.update(cx, |app, cx| app.environment_scope == scope && app.checkout_context() == context && app.can_change_checkout(cx)).unwrap_or(false);
            if !current { gate.cancel_before_send(); }
            let result = if current { connection::call_mutation(&sender, &tag, payload, gate.clone()).await } else { Err("Checkout selection changed or has unsaved edits".into()) };
            let _ = entity.update(cx, |app, cx| {
                let Some(index) = app.checkout_operations.iter().position(|op| op.key == key) else { return; };
                app.checkout_operations[index].running = false;
                match result {
                    Ok(value) => {
                        app.checkout_operations[index].management_outcome = Some(value.clone());
                        app.checkout_operations[index].error = Some("Worktree operation completed; Git was changed".into());
                        if app.environment_scope == scope && app.checkout_context() == context {
                            app.worktree_outcome = Some(value);
                            app.set_status("Worktree operation completed; dismiss its known outcome in Choose checkout", cx);
                        }
                    }
                    Err(error) => {
                        if !gate.may_have_been_sent() { app.checkout_operations.remove(index); }
                        else { app.checkout_operations[index].error = Some(format!("Worktree outcome may be unknown: {error}")); }
                        if app.environment_scope == scope && app.checkout_context() == context {
                            app.report_error(error, cx);
                        }
                    }
                }
                let _ = app.save_checkout_operations();
                if app.environment_scope == scope {
                    app.worktree_busy = false; app.refs.clear(); app.refs_cursor = None;
                    app.worktree_operation("vcs.listRefs", false, cx);
                }
                cx.notify();
            });
        }).detach();
        cx.notify();
    }

    /// Native receipts only: this is not a lock on other clients or host sessions.
    fn checkout_operation_preflight(
        &self,
        tag: &str,
        payload: &Value,
        reuse: bool,
    ) -> Option<&'static str> {
        if self.pending_actions > 0 {
            return Some("Wait for the pending server receipt; message retained");
        }
        let switches_checkout = !reuse
            && (matches!(tag, "vcs.switchRef" | "vcs.createRef")
                || (tag == "git.preparePullRequestThread" && payload["mode"] == "local"));
        let cwd = payload["cwd"].as_str();
        let affected = |thread: &t3code_desktop_gpui::shell::Thread| {
            switches_checkout
                && cwd.is_some_and(|cwd| {
                    let path = thread.worktree_path.as_deref().or_else(|| {
                        self.shell
                            .projects
                            .iter()
                            .find(|p| p.id == thread.project_id)
                            .map(|p| p.workspace_root.as_str())
                    });
                    path.is_some_and(|path| std::path::Path::new(path) == std::path::Path::new(cwd))
                })
        };
        if self.queues.thread_ids().iter().any(|id| {
            self.queues.dispatching(id)
                && (self.selected_thread.as_ref() == Some(id)
                    || self
                        .shell
                        .threads
                        .iter()
                        .any(|t| &t.id == id && affected(t)))
        }) {
            return Some(
                "Wait for the queued send receipt before changing checkout; queue retained",
            );
        }
        if self.shell.threads.iter().any(|thread| {
            self.selected_thread.as_ref() != Some(&thread.id)
                && affected(thread)
                && thread
                    .session
                    .as_ref()
                    .is_some_and(|s| s.status != "stopped")
        }) {
            return Some(
                "Another thread has an active session in this checkout. Stop it first or use a separate worktree",
            );
        }
        None
    }

    /// `resolve` is read-only; mutation is gate-tracked. Reuse supplies a known
    /// existing-worktree destination and issues no switch RPC.
    pub(crate) fn begin_checkout_operation(
        &mut self,
        tag: String,
        payload: Value,
        resolve: Option<Value>,
        reuse: Option<Destination>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.checkout_transition_pending() {
            self.report_error("Resolve the previous checkout outcome first", cx);
            return;
        }
        if let Some(reason) = self.checkout_operation_preflight(&tag, &payload, reuse.is_some()) {
            self.branch_workflow_error = Some(reason.into());
            self.report_error(reason, cx);
            return;
        }
        let Some(sender) = self.commands.clone() else {
            return;
        };
        let Some(project) = self
            .shell
            .projects
            .iter()
            .find(|p| Some(&p.id) == self.selected_project.as_ref())
        else {
            return;
        };
        let key = uuid::Uuid::new_v4().to_string();
        let context = self.checkout_context();
        let generation = self.generation;
        let root = project.workspace_root.clone();
        let id = self.selected_thread.clone();
        let materialized = id
            .as_ref()
            .is_some_and(|id| self.shell.threads.iter().any(|t| &t.id == id));
        // Do not let a caller's stale session snapshot skip authoritative shutdown.
        let stop = materialized;
        let gate = connection::MutationGate::new();
        self.checkout_operations.push(CheckoutOperation {
            key: key.clone(),
            intent: json!({"tag":tag,"payload":payload}),
            environment: self.environment_scope.clone(),
            project: project.id.clone(),
            thread: id.clone(),
            root: root.clone(),
            context: context.clone(),
            generation,
            materialized,
            destination: None,
            management_outcome: None,
            error: None,
            running: true,
            interested: true,
            gate: Some(gate.clone()),
        });
        if let Err(error) = self.save_checkout_operations() {
            self.checkout_operations.pop();
            self.report_error(format!("Cannot retain checkout recovery: {error}"), cx);
            return;
        }
        self.worktree_busy = true;
        self.branch_workflow_busy = true;
        self.branch_workflow_error = None;
        let handle = window.window_handle();
        cx.spawn(async move |entity, cx| {
            let result: Result<Destination, String> = async {
                let check = |cx: &mut gpui_kit::AsyncApp| -> Result<(), String> {
                    entity.update(cx, |app, cx| {
                        let interested = app.checkout_operations.iter().any(|op| op.key == key && op.interested);
                        if let Some(reason) = app.checkout_operation_preflight(&tag, &payload, reuse.is_some()) {
                            gate.cancel_before_send();
                            return Err(reason.into());
                        }
                        if interested && app.generation == generation && app.checkout_context() == context && app.can_change_checkout(cx) { Ok(()) }
                        else { gate.cancel_before_send(); Err("Checkout selection changed or has unsaved edits".into()) }
                    }).unwrap_or_else(|_| { gate.cancel_before_send(); Err("Window closed".into()) })
                };
                if let Some(payload) = resolve {
                    let resolved = connection::call(&sender, "git.resolvePullRequest", payload).await?;
                    if !resolved["pullRequest"].is_object() { return Err("Invalid PR resolution response".into()); }
                }
                check(cx)?;
                if stop && let Some(id) = &id {
                    super::checkout_receipts::stop_session(&entity, &sender, id, cx).await?;
                }
                check(cx)?;
                if let Some(destination) = reuse { return Ok(destination); }
                // Internal operation kind, NOT a new RPC. A manually entered
                // path is not evidence of which branch is physically there.
                if tag == "verify-existing-worktree" {
                    let cwd = payload["cwd"].as_str().ok_or("Missing checkout path")?;
                    let listing = connection::call(&sender, "vcs.listRefs", json!({"cwd":cwd,"refresh":true,"limit":100})).await?;
                    let rows: t3code_desktop_gpui::workspaces::Refs = serde_json::from_value(listing).map_err(|_| "Invalid existing checkout refs")?;
                    if !rows.is_repo { return Err("The selected path is not a repository".into()); }
                    check(cx)?;
                    return Ok(Destination { branch: rows.refs.iter().find(|r| r.current).map(|r| r.name.clone()), path: (std::path::Path::new(cwd) != std::path::Path::new(&root)).then(|| cwd.to_owned()), on_head: true });
                }
                let value = connection::call_mutation(&sender, &tag, payload.clone(), gate.clone()).await?;
                match tag.as_str() {
                    "vcs.createRef" | "vcs.switchRef" => {
                        let branch = match value.get("refName") {
                            Some(Value::String(name)) if !name.is_empty() => Some(name.clone()),
                            Some(Value::Null) if tag == "vcs.switchRef" => None,
                            _ => return Err("Checkout was transmitted but returned an invalid branch; inspect actual Git state".into()),
                        };
                        let cwd = payload["cwd"].as_str().ok_or("Missing checkout cwd")?;
                        Ok(Destination { branch, path: (cwd != root).then(|| cwd.to_owned()), on_head: true })
                    }
                    "git.preparePullRequestThread" => {
                        let branch = value["branch"].as_str().filter(|s| !s.is_empty()).ok_or("Invalid prepared branch; checkout outcome unknown")?.to_owned();
                        let path = match value.get("worktreePath") {
                            Some(Value::Null) => None,
                            Some(Value::String(path)) if !path.is_empty() => (std::path::Path::new(path) != std::path::Path::new(&root)).then(|| path.clone()),
                            _ => return Err("Invalid prepared path; checkout outcome unknown".into()),
                        };
                        Ok(Destination { branch: Some(branch), path, on_head: value["isOnPullRequestHead"].as_bool().unwrap_or(true) })
                    }
                    _ => Err("Unknown checkout operation".into()),
                }
            }.await;
            let _ = handle.update(cx, |_, window, cx| entity.update(cx, |app, cx| {
                let Some(index) = app.checkout_operations.iter().position(|op| op.key == key) else { return; };
                app.checkout_operations[index].running = false;
                let current = app.generation == generation && app.checkout_context() == context;
                if current { app.worktree_busy = false; app.branch_workflow_busy = false; }
                match result {
                    Ok(destination) => {
                        app.checkout_operations[index].destination = Some(destination);
                        app.checkout_operations[index].error = Some("Checkout prepared; metadata adoption pending".into());
                        let interested = app.checkout_operations[index].interested;
                        let _ = app.save_checkout_operations();
                        if current && interested && app.can_change_checkout(cx) { app.reconcile_checkout(key.clone(), window, cx); }
                        else if app.environment_scope == app.checkout_operations[index].environment && app.selected_project.as_deref() == Some(app.checkout_operations[index].project.as_str()) {
                            app.refs.clear(); app.refs_cursor = None;
                            app.set_status("Checkout changed; return to its thread to adopt the retained outcome", cx);
                            app.worktree_operation("vcs.listRefs", false, cx);
                        }
                    }
                    Err(error) => {
                        if !gate.may_have_been_sent() { app.checkout_operations.remove(index); }
                        else { app.checkout_operations[index].error = Some(format!("Outcome may be unknown: {error}. Inspect actual Git before adopting")); }
                        if current {
                            app.branch_workflow_error = Some(error);
                            if gate.may_have_been_sent() { app.refs.clear(); app.refs_cursor = None; app.worktree_operation("vcs.listRefs", false, cx); }
                        }
                        let _ = app.save_checkout_operations();
                    }
                }
                cx.notify();
            }));
        }).detach();
        cx.notify();
    }

    fn reconcile_checkout(&mut self, key: String, window: &mut Window, cx: &mut Context<Self>) {
        let Some(index) = self.checkout_operations.iter().position(|op| op.key == key) else {
            return;
        };
        let op = &self.checkout_operations[index];
        let Some(destination) = op.destination.clone() else {
            return;
        };
        let Some(sender) = self.commands.clone() else {
            return;
        };
        let scope = op.environment.clone();
        let project = op.project.clone();
        let thread = op.thread.clone();
        let context = op.context.clone();
        let materialized = op.materialized;
        let generation = self.generation;
        // A draft unexpectedly materialized despite the composer fence is NOT
        // adopted locally: retry must update its now-real server record.
        let server_thread = materialized
            || thread
                .as_ref()
                .is_some_and(|id| self.shell.threads.iter().any(|t| &t.id == id));
        self.checkout_operations[index].running = true;
        let _ = self.save_checkout_operations();
        let handle = window.window_handle();
        cx.spawn(async move |entity, cx| {
            let result = if server_thread && let Some(id) = &thread {
                let mut payload = commands::command("thread.meta.update", id);
                payload["branch"] = json!(destination.branch);
                payload["worktreePath"] = json!(destination.path);
                connection::call(&sender, "orchestration.dispatchCommand", payload).await.map(|_| ())
            } else { Ok(()) };
            let _ = handle.update(cx, |_, window, cx| entity.update(cx, |app, cx| {
                let Some(index) = app.checkout_operations.iter().position(|op| op.key == key) else { return; };
                app.checkout_operations[index].running = false;
                let same_selection = app.environment_scope == scope && app.generation == generation && app.selected_project == context.0 && app.selected_thread == thread;
                // Projection may arrive before the RPC receipt and change cwd.
                let safe = same_selection && (app.checkout_context() == context || app.selected_workspace() == Some(destination.path.clone().unwrap_or(app.checkout_operations[index].root.clone()))) && app.can_change_checkout(cx);
                match result {
                    Ok(()) if safe => {
                        app.apply_checkout_destination(project, destination.clone(), window, cx);
                        app.checkout_operations.remove(index);
                        app.close_branch_picker(window, cx);
                        app.set_status(if destination.on_head { "Checkout updated; message retained" } else { "Checkout selected but not on PR head; local changes were preserved" }, cx);
                    }
                    Ok(()) => { app.checkout_operations[index].error = Some("Metadata accepted; return to the captured thread to adopt".into()); }
                    Err(error) => { app.checkout_operations[index].error = Some(format!("Checkout changed; metadata failed: {error}. Retry metadata only")); }
                }
                let _ = app.save_checkout_operations();
                if same_selection {
                    app.worktree_busy = false; app.branch_workflow_busy = false;
                    app.refs.clear(); app.refs_cursor = None;
                    app.branch_workflow_error = app.checkout_operations.iter().find(|op| op.key == key).and_then(|op| op.error.clone());
                    // Read-only refresh is separate from retained reconciliation.
                    app.worktree_operation("vcs.listRefs", false, cx);
                }
                cx.notify();
            }));
        }).detach();
    }
}
