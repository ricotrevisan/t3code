//! Instance IDs route requests; driver names and display labels never do.
use serde::Deserialize;
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Model {
    pub slug: String,
    pub name: String,
    #[serde(default)]
    pub is_default: bool,
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
