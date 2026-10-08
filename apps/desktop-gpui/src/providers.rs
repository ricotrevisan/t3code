//! Instance IDs route requests; driver names and display labels never do.
use serde::Deserialize;
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Model {
    pub slug: String,
    pub name: String,
    #[serde(default)]
    pub is_default: bool,
    #[serde(default)]
    pub capabilities: serde_json::Value,
}
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Provider {
    pub instance_id: String,
    pub driver: String,
    pub display_name: Option<String>,
    pub enabled: bool,
    pub installed: bool,
    pub status: String,
    pub message: Option<String>,
    pub availability: Option<String>,
    pub unavailable_reason: Option<String>,
    #[serde(default)]
    pub requires_new_thread_for_model_change: bool,
    pub models: Vec<Model>,
    #[serde(default)]
    pub supported_runtime_modes: Option<Vec<String>>,
    #[serde(default)]
    pub default_runtime_mode: Option<String>,
    #[serde(default)]
    pub show_interaction_mode_toggle: Option<bool>,
    #[serde(default)]
    pub slash_commands: Vec<serde_json::Value>,
    #[serde(default)]
    pub skills: Vec<serde_json::Value>,
    #[serde(default)]
    pub workspace_snapshots: Vec<serde_json::Value>,
}
#[derive(Deserialize)]
pub struct Config {
    pub providers: Vec<Provider>,
}
impl Provider {
    pub fn label(&self) -> &str {
        self.display_name.as_deref().unwrap_or(&self.driver)
    }
    pub fn selectable(&self) -> bool {
        self.enabled
            && self.installed
            && self.availability.as_deref() != Some("unavailable")
            && matches!(self.status.as_str(), "ready" | "warning")
    }
    pub fn explanation(&self) -> &str {
        if !self.enabled {
            return "Disabled in this environment";
        }
        if !self.installed {
            return "Provider runtime is not installed";
        }
        self.unavailable_reason
            .as_deref()
            .or(self.message.as_deref())
            .unwrap_or(&self.status)
    }
    pub fn runtime_modes(&self) -> Vec<&str> {
        [
            "approval-required",
            "auto-accept-edits",
            "auto",
            "full-access",
        ]
        .into_iter()
        .filter(|mode| {
            self.supported_runtime_modes
                .as_ref()
                .is_none_or(|modes| modes.iter().any(|supported| supported == mode))
        })
        .collect()
    }
    pub fn commands_for_cwd(&self, cwd: Option<&str>) -> Vec<serde_json::Value> {
        self.workspace_snapshots
            .iter()
            .find(|snapshot| snapshot["cwd"].as_str() == cwd && cwd.is_some())
            .and_then(|snapshot| snapshot["slashCommands"].as_array())
            .cloned()
            .unwrap_or_else(|| self.slash_commands.clone())
    }
    pub fn skills_for_cwd(&self, cwd: Option<&str>) -> Vec<serde_json::Value> {
        let skills = self
            .workspace_snapshots
            .iter()
            .find(|snapshot| snapshot["cwd"].as_str() == cwd && cwd.is_some())
            .and_then(|snapshot| snapshot["skills"].as_array())
            .unwrap_or(&self.skills);
        let mut seen = std::collections::HashSet::new();
        skills
            .iter()
            .filter(|skill| {
                skill["enabled"] == true
                    && skill["userInvocable"] != false
                    && skill["name"].as_str().is_some_and(|name| {
                        !name.trim().is_empty() && seen.insert(name.trim().to_lowercase())
                    })
            })
            .cloned()
            .collect()
    }

    pub fn default_model(&self) -> Option<&Model> {
        self.models
            .iter()
            .find(|model| model.is_default)
            .or_else(|| self.models.first())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn instance_routing_and_explicit_availability_are_preserved() {
        let mut provider:Provider=serde_json::from_value(serde_json::json!({"instanceId":"second-codex","driver":"codex","displayName":"Work account","enabled":true,"installed":true,"status":"ready","models":[{"slug":"a","name":"A"},{"slug":"b","name":"B","isDefault":true}]})).unwrap();
        assert_eq!(provider.instance_id, "second-codex");
        assert_eq!(provider.label(), "Work account");
        assert!(provider.selectable());
        assert_eq!(provider.default_model().unwrap().slug, "b");
        provider.availability = Some("unavailable".into());
        provider.unavailable_reason = Some("Adapter missing".into());
        assert!(!provider.selectable());
        assert_eq!(provider.explanation(), "Adapter missing");
        provider.availability = None;
        provider.enabled = false;
        assert!(!provider.selectable());
    }
}
