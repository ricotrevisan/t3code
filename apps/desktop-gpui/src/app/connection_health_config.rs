//! Generation-scoped server configuration, following client-runtime's projection.
use crate::NativeApp;
use gpui_kit::Context;
use serde_json::{Value, json};
use t3code_desktop_gpui::providers;

#[derive(Default)]
pub(crate) struct Projection {
    generation: u64,
    config: Option<Value>,
    live: bool,
}
impl Projection {
    pub(crate) fn config(&self, generation: u64) -> Option<&Value> {
        (generation == self.generation)
            .then_some(self.config.as_ref())
            .flatten()
    }
    /// RPC hydration must never roll back an already-arrived stream event.
    fn seed(&mut self, generation: u64, config: Value) -> bool {
        if self.generation == generation && self.live {
            return false;
        }
        let changed = self.apply(generation, &json!({"type":"snapshot", "config":config}));
        self.live = false;
        changed
    }
    pub(crate) fn apply(&mut self, generation: u64, event: &Value) -> bool {
        if generation != self.generation {
            self.generation = generation;
            self.config = None;
            self.live = false;
        }
        match event["type"].as_str() {
            Some("snapshot") => {
                let mut config = event["config"].clone();
                if !config.is_object() {
                    return false;
                }
                for (field, capability) in [
                    ("environmentThemes", "environmentThemes"),
                    ("usageLimitSources", "usageLimitSources"),
                ] {
                    config.as_object_mut().unwrap().remove(field);
                    if config["environment"]["capabilities"][capability] == true
                        && let Some(carried) = self.config.as_ref().and_then(|c| c.get(field))
                    {
                        config[field] = carried.clone();
                    }
                }
                self.config = Some(config);
            }
            Some(kind) => {
                let Some(config) = self.config.as_mut() else {
                    return false;
                };
                let payload = &event["payload"];
                match kind {
                    "providerStatuses" => config["providers"] = payload["providers"].clone(),
                    "settingsUpdated" => config["settings"] = payload["settings"].clone(),
                    "keybindingsUpdated" => {
                        config["keybindings"] = payload["keybindings"].clone();
                        config["issues"] = payload["issues"].clone();
                    }
                    "usageLimitSourcesUpdated" | "environmentThemesUpdated" => {
                        let (field, source) = if kind == "usageLimitSourcesUpdated" {
                            ("usageLimitSources", "sources")
                        } else {
                            ("environmentThemes", "themes")
                        };
                        if payload[source].as_array().is_some_and(|a| !a.is_empty()) {
                            config[field] = payload[source].clone();
                        } else {
                            config.as_object_mut().unwrap().remove(field);
                        }
                    }
                    _ => return false,
                }
            }
            _ => return false,
        }
        self.live = true;
        true
    }
}
impl NativeApp {
    pub(crate) fn accept_config_snapshot(&mut self, value: Value, cx: &mut Context<Self>) {
        if !self.verify_environment_config(&value, cx) { return; }
        if self.server_config.seed(self.generation, value) {
            self.publish_server_config(cx);
        }
    }
    pub(crate) fn apply_server_config_event(&mut self, event: Value, cx: &mut Context<Self>) {
        if event["type"] == "snapshot" && !self.verify_environment_config(&event["config"], cx) { return; }
        if !self.server_config.apply(self.generation, &event) {
            return;
        }
        self.publish_server_config(cx);
    }
    fn publish_server_config(&mut self, cx: &mut Context<Self>) {
        let value = self.server_config.config(self.generation).unwrap().clone();
        self.update_environment_identity(&value);
        self.remember_environment_identity(&value, cx);
        self.sidebar_state.capabilities = value["environment"]["capabilities"].clone();
        self.project_settings = value.get("settings").cloned();
        match serde_json::from_value::<providers::Config>(value.clone()) {
            Ok(config) => self.provider_catalog = config.providers,
            Err(_) => self.report_error("Invalid provider catalog response", cx),
        }
        self.settings_provider_details = value["providers"]
            .as_array()
            .into_iter()
            .flatten()
            .cloned()
            .map(|mut provider| {
                provider["__nativeGeneration"] = self.generation.into();
                let id = provider["instanceId"]
                    .as_str()
                    .unwrap_or_default()
                    .to_owned();
                if let Some(configuration) = value["settings"]["providerInstances"]
                    .get(&id)
                    .filter(|c| c.is_object())
                {
                    provider["__nativeConfiguration"] = configuration.clone();
                }
                provider
            })
            .collect();
        if self.sessions.selected().is_some() && self.environment_identity_verified {
            self.flush_queues(cx);
            self.publish_environment_phase(cx);
        }
        cx.notify();
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fixa_projection_replays_sources_status_settings_and_fences_generations() {
        let mut p = Projection::default();
        assert!(!p.apply(
            1,
            &json!({"type":"providerStatuses", "payload":{"providers":[]}})
        ));
        p.apply(1, &json!({"type":"snapshot","config":{"providers":[],"environment":{"capabilities":{"usageLimitSources":true}}}}));
        p.apply(1, &json!({"type":"usageLimitSourcesUpdated","payload":{"sources":[{"label":"External", "error":"probe failed"}]}}));
        p.apply(1, &json!({"type":"providerStatuses","payload":{"providers":[{"instanceId":"live","status":"ready"}]}}));
        p.apply(
            1,
            &json!({"type":"settingsUpdated","payload":{"settings":{"providerInstances":{}}}}),
        );
        assert!(
            !p.seed(1, json!({"providers":[]})),
            "late RPC hydration cannot roll back a live projection"
        );
        assert_eq!(p.config(1).unwrap()["providers"][0]["instanceId"], "live");
        assert!(p.config(1).unwrap()["settings"]["providerInstances"].is_object());
        assert_eq!(
            t3code_desktop_gpui::surfaces::usage::accounts(p.config(1).unwrap())[0]
                .notice
                .as_deref(),
            Some("probe failed")
        );
        assert!(p.config(2).is_none());
        p.apply(1, &json!({"type":"snapshot","config":{"environment":{"capabilities":{"usageLimitSources":true}}}}));
        assert_eq!(
            p.config(1).unwrap()["usageLimitSources"][0]["label"],
            "External"
        );
        p.apply(
            1,
            &json!({"type":"usageLimitSourcesUpdated","payload":{"sources":[]}}),
        );
        assert!(p.config(1).unwrap().get("usageLimitSources").is_none());
        p.apply(
            2,
            &json!({"type":"snapshot","config":{"environment":{"capabilities":{}}}}),
        );
        assert!(p.config(2).unwrap().get("usageLimitSources").is_none());
    }
}
