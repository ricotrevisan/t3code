//! Product branch workflows; physical outcomes live in checkout_operations.
use super::checkout_operations::Destination;
use crate::NativeApp;
use gpui_kit::{Context, Window};
use serde_json::json;

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum BranchWorkflow {
    #[default]
    Browse,
    Create,
    PullRequest,
    Environment,
}

fn branch_name(raw: &str) -> Result<String, String> {
    // Same whitespace normalization as shared/git.sanitizeNewRefName; git is
    // authoritative for the remaining ref-format rules.
    let mut name = String::new();
    let mut separating = false;
    for c in raw.trim().chars() {
        if matches!(c, ' ' | '\t' | '\n' | '\r' | '\u{000c}' | '\u{000b}') {
            if !separating {
                name.push('-');
            }
            separating = true;
        } else {
            name.push(c);
            separating = false;
        }
    }
    if name.is_empty() {
        return Err("Enter a branch name".into());
    }
    if name.starts_with('-')
        || name.contains("..")
        || name.contains("@{")
        || name
            .chars()
            .any(|c| c.is_ascii_control() || "~^:?*[\\".contains(c))
        || name.split('/').any(|part| {
            part.is_empty()
                || part.starts_with('.')
                || part.ends_with('.')
                || part.ends_with(".lock")
        })
    {
        return Err("Enter a valid Git branch name".into());
    }
    Ok(name)
}

fn pr_reference(raw: &str) -> Result<String, String> {
    let value = raw.trim();
    let number = value.strip_prefix('#').unwrap_or(value);
    if number.parse::<u64>().is_ok_and(|n| n > 0) {
        return Ok(number.into());
    }
    if value.starts_with("https://")
        && (value.contains("/pull/")
            || value.contains("/pullrequest/")
            || value.contains("/merge_requests/"))
    {
        return Ok(value.into());
    }
    Err("Enter a PR number, #123, or pull request URL".into())
}

impl NativeApp {
    pub(crate) fn cancel_branch_workflow(&mut self, cx: &mut Context<Self>) {
        self.cancel_checkout_interest();
        self.clear_checkout_stops();
        self.worktree_busy = false;
        self.branch_workflow_busy = false;
        self.branch_workflow_error = None;
        self.branch_workflow = BranchWorkflow::Browse;
        self.git_picker.index = 0;
        self.git_picker.query.clear();
        self.git_picker.scroll.scroll_to_item(0);
        cx.notify();
    }

    pub(crate) fn show_branch_workflow(
        &mut self,
        page: BranchWorkflow,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.cancel_branch_workflow(cx);
        self.branch_workflow = page;
        let base = if page == BranchWorkflow::Environment {
            self.sync_checkout_recipe_controls(window, cx)
        } else {
            String::new()
        };
        self.branch_workflow_input.update(cx, |input, cx| {
            input.set_value(base, window, cx);
            input.focus(window, cx);
        });
        cx.notify();
    }

    /// Restore both directions: selecting Local clears a previous planned base.
    pub(crate) fn sync_checkout_recipe_controls(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> String {
        let recipe = self
            .pending_threads
            .iter()
            .find(|d| {
                Some(&d.thread.id) == self.selected_thread.as_ref()
                    && !self.shell.threads.iter().any(|t| t.id == d.thread.id)
            })
            .and_then(|d| d.prepare_worktree.clone())
            .or_else(|| self.saved_materialized_recipe());
        self.prepare_new_worktree = recipe.is_some();
        self.start_from_origin = recipe
            .as_ref()
            .and_then(|r| r["startFromOrigin"].as_bool())
            .unwrap_or(false);
        let base = recipe
            .as_ref()
            .and_then(|r| r["baseBranch"].as_str())
            .unwrap_or_default()
            .to_owned();
        let branch = recipe
            .as_ref()
            .and_then(|r| r["branch"].as_str())
            .unwrap_or_default()
            .to_owned();
        self.worktree_ref
            .update(cx, |input, cx| input.set_value(base.clone(), window, cx));
        self.worktree_branch
            .update(cx, |input, cx| input.set_value(branch, window, cx));

        let path = self.selected_workspace().filter(|p| {
            self.shell
                .projects
                .iter()
                .find(|p| Some(&p.id) == self.selected_project.as_ref())
                .is_some_and(|project| p != &project.workspace_root)
        });
        self.worktree_path.update(cx, |input, cx| {
            input.set_value(path.unwrap_or_default(), window, cx)
        });
        if self.inspector_open
            && self.inspector_section == Some(crate::InspectorSection::Worktrees)
            && self.refs.is_empty()
            && !self.worktree_busy
        {
            self.worktree_operation("vcs.listRefs", false, cx);
        }
        base
    }

    pub(crate) fn branch_workflow_restriction(&self) -> Option<&'static str> {
        if self.selected_thread.is_some() && self.selected_workspace().is_none() {
            return Some("The selected checkout is unavailable; select a valid thread");
        }
        if self.selected_thread.as_ref().is_some_and(|id| {
            self.archived_shell.threads.iter().any(|t| &t.id == id)
                && !self.shell.threads.iter().any(|t| &t.id == id)
        }) {
            Some("Restore the archived thread before changing its checkout")
        } else if self.branch_workflow == BranchWorkflow::Create
            && (self.pending_threads.iter().any(|draft| {
                Some(&draft.thread.id) == self.selected_thread.as_ref()
                    && draft.prepare_worktree.is_some()
                    && !self.shell.threads.iter().any(|t| t.id == draft.thread.id)
            }) || self.saved_materialized_recipe().is_some())
        {
            Some("Worktree drafts select a base branch. Choose Local before creating a branch")
        } else if self.selected_thread.is_none() && self.prepare_new_worktree {
            Some("Create an unsent draft first to change the planned worktree")
        } else if self.pending_actions > 0 {
            Some("Wait for the pending server receipt")
        } else if self.commands.is_none() {
            Some("Connect to the environment first")
        } else {
            None
        }
    }

    pub(crate) fn branch_environment_restriction(&self) -> Option<&'static str> {
        if self.empty_root_checkout_eligible() {
            return None;
        }
        if self
            .selected_thread
            .as_ref()
            .is_some_and(|id| self.shell.threads.iter().any(|t| &t.id == id))
        {
            Some(
                "Environment mode requires unsent drafts or a synchronized empty root thread without an active session",
            )
        } else if !self
            .pending_threads
            .iter()
            .any(|d| Some(&d.thread.id) == self.selected_thread.as_ref())
        {
            Some("Create an unsent draft first to choose its environment")
        } else {
            None
        }
    }

    /// Mode is a draft recipe, not a request to move a materialized thread. As on
    /// web, a thread already in a worktree has a locked environment mode.
    pub(crate) fn select_branch_environment(
        &mut self,
        worktree: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(reason) = self
            .branch_workflow_restriction()
            .or_else(|| self.branch_environment_restriction())
        {
            self.branch_workflow_error = Some(reason.into());
            cx.notify();
            return;
        }
        if !self.can_change_checkout(cx) {
            self.branch_workflow_error = Some(self.status.clone());
            cx.notify();
            return;
        }
        if self.worktree_busy {
            return;
        }
        let Some(id) = self.selected_thread.clone() else {
            return;
        };
        let index = self
            .pending_threads
            .iter()
            .position(|d| d.thread.id == id && !self.shell.threads.iter().any(|t| t.id == id));
        let Some(project_id) = self.selected_project.clone() else {
            return;
        };
        let Some(root) = self
            .shell
            .projects
            .iter()
            .find(|p| p.id == project_id)
            .map(|p| p.workspace_root.clone())
        else {
            return;
        };
        let base = self
            .branch_workflow_input
            .read(cx)
            .value()
            .trim()
            .to_owned();
        if worktree && base.is_empty() {
            self.branch_workflow_error = Some("Enter the base branch for the new worktree".into());
            cx.notify();
            return;
        }
        self.choose_next_workspace(worktree);
        self.worktree_ref.update(cx, |input, cx| {
            input.set_value(
                if worktree {
                    base.clone()
                } else {
                    String::new()
                },
                window,
                cx,
            )
        });
        if !worktree {
            self.start_from_origin = false;
            self.worktree_branch
                .update(cx, |input, cx| input.set_value("", window, cx));
        }
        self.worktree_path
            .update(cx, |input, cx| input.set_value("", window, cx));
        let branch = self.worktree_branch.read(cx).value().trim().to_owned();
        let recipe = worktree.then(|| {
            let mut recipe = json!({"projectCwd":root,"baseBranch":base,"startFromOrigin":self.start_from_origin});
            if !branch.is_empty() { recipe["branch"] = json!(branch); }
            recipe
        });
        if let Some(index) = index {
            let draft = &mut self.pending_threads[index];
            draft.prepare_worktree = recipe;
            draft.defaults_override.environment = self.environment_scope.clone();
            draft.defaults_override.project = project_id.clone();
            draft.defaults_override.workspace_mode = Some(if worktree { "worktree" } else { "local" }.into());
            draft.defaults_override.start_from_origin = Some(self.start_from_origin);
            draft.defaults_override.run_setup_script = Some(self.run_setup_script);
            draft.base_branch = (!base.is_empty()).then_some(base);
            // Local selects the root, never arbitrary Worktree base text.
            draft.thread.branch = None;
            draft.thread.worktree_path = None;
            if let Some(store) = &mut self.draft_store {
                store.put_pending(&self.environment_scope, draft.clone());
            }
        } else {
            self.set_materialized_recipe(recipe, cx);
        }
        self.new_thread_checkout = None;
        self.schedule_draft_save(cx);
        self.reset_checkout_context(window, cx);
        self.close_branch_picker(window, cx);
        self.set_status(
            if worktree {
                "Draft will prepare its worktree when sent; message retained"
            } else {
                "Draft uses the project root; message retained"
            },
            cx,
        );
    }

    pub(crate) fn run_branch_workflow(
        &mut self,
        worktree: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.worktree_busy || self.branch_workflow_busy {
            return;
        }
        if let Some(reason) = self.branch_workflow_restriction() {
            self.branch_workflow_error = Some(reason.into());
            cx.notify();
            return;
        }
        if !self.can_change_checkout(cx) {
            self.branch_workflow_error = Some(self.status.clone());
            cx.notify();
            return;
        }
        let page = self.branch_workflow;
        let raw = self.branch_workflow_input.read(cx).value().to_string();
        let value = match page {
            BranchWorkflow::Create => branch_name(&raw),
            BranchWorkflow::PullRequest => pr_reference(&raw),
            _ => return,
        };
        let value = match value {
            Ok(value) => value,
            Err(error) => {
                self.branch_workflow_error = Some(error);
                cx.notify();
                return;
            }
        };
        if page == BranchWorkflow::Create && self.refs.iter().any(|r| r.name == value) {
            self.branch_workflow_error =
                Some("That branch already exists; select it from the picker".into());
            cx.notify();
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
        let root = project.workspace_root.clone();
        let cwd = if page == BranchWorkflow::Create {
            self.selected_workspace().unwrap_or(root.clone())
        } else {
            root.clone()
        };
        let materialized = self
            .selected_thread
            .as_ref()
            .and_then(|id| self.shell.threads.iter().find(|t| &t.id == id));
        let id = materialized.map(|t| t.id.clone());
        // A queued accepted start may not have projected a session yet.
        // The host's correlated stop also authoritatively completes no-op stops.
        let (tag, payload, resolve) = if page == BranchWorkflow::Create {
            (
                "vcs.createRef",
                json!({"cwd":cwd,"refName":value,"switchRef":true}),
                None,
            )
        } else {
            let mut payload =
                json!({"cwd":cwd,"reference":value,"mode":if worktree {"worktree"} else {"local"}});
            if worktree && let Some(id) = &id {
                payload["threadId"] = json!(id);
            }
            (
                "git.preparePullRequestThread",
                payload,
                Some(json!({"cwd":cwd,"reference":value})),
            )
        };
        self.begin_checkout_operation(tag.into(), payload, resolve, None, window, cx);
    }

    /// Applies an already-prepared checkout. Never switch again through pick_checkout.
    pub(crate) fn apply_checkout_destination(
        &mut self,
        project: String,
        destination: Destination,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let branch = destination.branch;
        let path = destination.path;
        self.prepare_new_worktree = false;
        self.start_from_origin = false;
        self.worktree_ref
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.worktree_branch
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.worktree_path.update(cx, |input, cx| {
            input.set_value(path.clone().unwrap_or_default(), window, cx)
        });
        self.set_materialized_recipe(None, cx);
        if let Some(id) = self.selected_thread.clone() {
            if let Some(draft) = self
                .pending_threads
                .iter_mut()
                .find(|d| d.thread.id == id && !self.shell.threads.iter().any(|t| t.id == id))
            {
                draft.prepare_worktree = None;
                draft.thread.branch = branch.clone();
                draft.thread.worktree_path = path.clone();
                if let Some(store) = &mut self.draft_store {
                    store.put_pending(&self.environment_scope, draft.clone());
                }
                self.schedule_draft_save(cx);
            }
            for thread in self.shell.threads.iter_mut().filter(|t| t.id == id) {
                thread.branch = branch.clone();
                thread.worktree_path = path.clone();
            }
            if let Some(detail) = self.detail.thread.as_mut().filter(|t| t["id"] == id) {
                let detail = std::sync::Arc::make_mut(detail);
                detail["branch"] = json!(branch);
                detail["worktreePath"] = json!(path);
            }
        }
        if !self
            .selected_thread
            .as_ref()
            .is_some_and(|id| self.shell.threads.iter().any(|t| &t.id == id))
        {
            self.new_thread_checkout = branch.map(|branch| (project, branch, path));
        }
        self.reset_checkout_context(window, cx);
    }
}
