//! First-send overrides for an empty materialized root thread. The public
//! turn bootstrap supports prepareWorktree without createThread.
use crate::NativeApp;
use gpui_kit::Context;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Serialize, Deserialize)]
pub(crate) struct CheckoutRecipe {
    environment: String,
    project: String,
    thread: String,
    prepare: Value,
    run_setup: bool,
}

#[cfg(not(test))]
fn path() -> std::path::PathBuf {
    t3code_desktop_gpui::drafts::default_path().with_file_name("checkout-recipes.json")
}
pub(crate) fn restore() -> Vec<CheckoutRecipe> {
    #[cfg(test)]
    return Vec::new();
    #[cfg(not(test))]
    std::fs::read(path())
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}
impl NativeApp {
    pub(crate) fn empty_root_checkout_eligible(&self) -> bool {
        let Some(thread) = self.shell.threads.iter().find(|t| {
            Some(&t.id) == self.selected_thread.as_ref()
                && Some(&t.project_id) == self.selected_project.as_ref()
        }) else {
            return false;
        };
        let Some(detail) = self.detail.thread.as_ref().filter(|t| t["id"] == thread.id) else {
            return false;
        };
        self.detail.synchronized
            && thread.worktree_path.is_none()
            && detail["worktreePath"].is_null()
            && thread
                .session
                .as_ref()
                .is_none_or(|s| s.status == "stopped")
            && detail
                .get("session")
                .is_none_or(|s| s.is_null() || s["status"] == "stopped")
            && detail["messages"]
                .as_array()
                .is_some_and(|messages| messages.is_empty())
            && detail["latestTurn"].is_null()
    }
    pub(crate) fn saved_materialized_recipe(&self) -> Option<Value> {
        if !self.empty_root_checkout_eligible() {
            return None;
        }
        self.checkout_recipes
            .iter()
            .find(|r| {
                r.environment == self.environment_scope
                    && Some(&r.thread) == self.selected_thread.as_ref()
                    && Some(&r.project) == self.selected_project.as_ref()
            })
            .map(|r| r.prepare.clone())
    }
    pub(crate) fn set_materialized_recipe(
        &mut self,
        recipe: Option<Value>,
        cx: &mut Context<Self>,
    ) {
        self.checkout_recipes.retain(|r| {
            !(r.environment == self.environment_scope
                && Some(&r.thread) == self.selected_thread.as_ref())
        });
        if let (Some(prepare), Some(project), Some(thread)) = (
            recipe,
            self.selected_project.clone(),
            self.selected_thread.clone(),
        ) {
            self.checkout_recipes.push(CheckoutRecipe {
                environment: self.environment_scope.clone(),
                project,
                thread,
                prepare,
                run_setup: self.run_setup_script,
            });
        }
        #[cfg(not(test))]
        {
            let path = path();
            let result = serde_json::to_vec(&self.checkout_recipes)
                .map_err(std::io::Error::other)
                .and_then(|bytes| super::checkout_operations::persist(&path, &bytes));
            if let Err(error) = result {
                self.set_status(format!("Could not save checkout recipe: {error}"), cx);
            }
        }
        cx.notify();
    }
    /// Called before serializing any first-send command, including queued retry.
    /// Never createThread for an existing server record. Never consume a recipe
    /// merely because send failed; read-model eligibility prevents stale reuse.
    pub(crate) fn materialized_checkout_bootstrap(
        &self,
        id: &str,
    ) -> Result<Option<Value>, String> {
        let Some(recipe) = self
            .checkout_recipes
            .iter()
            .find(|r| r.environment == self.environment_scope && r.thread == id)
        else {
            return Ok(None);
        };
        // A projected checkout/message consumes first-send interest. Retaining
        // the saved recipe cannot replay physical preparation on later turns.
        if self
            .shell
            .threads
            .iter()
            .find(|t| t.id == id)
            .is_some_and(|t| t.worktree_path.is_some())
            || self
                .detail
                .thread
                .as_ref()
                .filter(|t| t["id"] == id)
                .is_some_and(|t| {
                    t["messages"].as_array().is_some_and(|m| !m.is_empty())
                        || t["worktreePath"].is_string()
                })
        {
            return Ok(None);
        }
        if self.selected_thread.as_deref() != Some(id)
            || self.selected_project.as_deref() != Some(recipe.project.as_str())
            || !self.empty_root_checkout_eligible()
        {
            return Err("Saved first-send checkout recipe no longer eligible; return to its empty root thread or explicitly choose Local".into());
        }
        Ok(Some(
            json!({"prepareWorktree":recipe.prepare,"runSetupScript":recipe.run_setup}),
        ))
    }
    pub(crate) fn update_materialized_checkout_recipe(&mut self, cx: &mut Context<Self>) -> bool {
        if !self.empty_root_checkout_eligible() {
            return false;
        }
        let base = self.worktree_ref.read(cx).value().trim().to_owned();
        if self.prepare_new_worktree && base.is_empty() {
            self.set_status("Select a base branch for the new worktree", cx);
            return true;
        }
        let Some(root) = self.selected_workspace() else {
            return true;
        };
        let branch = self.worktree_branch.read(cx).value().trim().to_owned();
        let recipe = self.prepare_new_worktree.then(|| {
            let mut value = json!({"projectCwd":root,"baseBranch":base,"startFromOrigin":self.start_from_origin});
            if !branch.is_empty() { value["branch"] = json!(branch); }
            value
        });
        self.set_materialized_recipe(recipe, cx);
        self.set_status("First-send checkout recipe updated; message retained", cx);
        true
    }
}
