//! Availability derives from the server's setup descriptor, never the driver name.
use crate::NativeApp;
use gpui_kit::{AppContext as _, Context};
use serde_json::{Value, json};
use t3code_desktop_gpui::connection;

impl NativeApp {
    pub(crate) fn accept_provider_auth(&mut self, state: Value, open_browser: bool, cx: &mut Context<Self>) {
        let succeeded = state["phase"] == "succeeded"
            && self.auth_state.as_ref().is_none_or(|old| old["phase"] != "succeeded");
        let new_flow = self.auth_state.as_ref().is_none_or(|old|
            old["instanceId"] != state["instanceId"] || old["flowId"] != state["flowId"]);
        if open_browser && new_flow && let Some(url) = browser_url(&state) { cx.open_url(url); }
        if open_browser && new_flow && let Some(instance) = state["instanceId"].as_str() {
            self.enqueue_subscription(connection::Command::Subscribe {
                key: format!("auth:{instance}"), tag: "provider.auth.subscribe".into(),
                payload: json!({"instanceId":instance}),
            }, cx);
        }
        self.auth_state = Some(state);
        if succeeded { self.refresh_settings_providers(true, cx); }
        cx.notify();
    }

    pub(crate) fn cancel_provider_auth(&mut self, cx: &mut Context<Self>) {
        let Some(state) = self.auth_state.as_ref() else { return; };
        let (Some(instance), Some(flow)) = (state["instanceId"].as_str(), state["flowId"].as_str()) else { return; };
        self.perform("provider.auth.cancel", json!({"instanceId":instance,"flowId":flow}), None, cx);
    }

    pub(crate) fn create_settings_query(
        window: &mut gpui_kit::Window,
        cx: &mut Context<Self>,
    ) -> gpui_kit::Entity<gpui_kit::component::input::InputState> {
        let query =
            cx.new(|cx| crate::single_line_input(window, cx).placeholder("Search settings…"));
        cx.subscribe(
            &query,
            |_, _, _: &gpui_kit::component::input::InputEvent, cx| cx.notify(),
        )
        .detach();
        query
    }
    pub(crate) fn refresh_settings_providers(&mut self, refresh: bool, cx: &mut Context<Self>) {
        if self.settings_provider_busy == Some(self.generation) {
            return;
        }
        let Some(sender) = self.commands.clone() else {
            self.report_error("Not connected; cannot refresh providers", cx);
            return;
        };
        let generation = self.generation;
        self.settings_provider_busy = Some(generation);
        self.settings_provider_task = Some(cx.spawn(async move |entity, cx| {
            let result = async {
                if refresh {
                    connection::call(
                        &sender,
                        "server.refreshProviders",
                        json!({"refreshModels":true}),
                    )
                    .await?;
                }
                let config = connection::call(&sender, "server.getConfig", json!({})).await?;
                if !config.is_object() {
                    return Err("Invalid provider catalog response".to_owned());
                }
                let settings = connection::call(&sender, "server.getSettings", json!({})).await?;
                Ok::<_, String>((config, settings))
            }
            .await;
            let _ = entity.update(cx, |app, cx| {
                if app.generation != generation {
                    return;
                }
                app.settings_provider_busy = None;
                match result {
                    Ok((value, settings)) => {
                        let mut value = value;
                        value["settings"] = settings;
                        app.accept_config_snapshot(value, cx);
                    }
                    Err(error) => app.report_error(error, cx),
                }
                cx.notify();
            });
        }));
        cx.notify();
    }
    pub(crate) fn set_provider_enabled(&mut self, id: &str, enabled: bool, cx: &mut Context<Self>) {
        if self.settings_provider_busy == Some(self.generation) {
            return;
        }
        let Some(provider) = self.settings_provider_details.iter().find(|provider| {
            provider["instanceId"].as_str() == Some(id)
                && provider["__nativeGeneration"].as_u64() == Some(self.generation)
        }) else {
            self.report_error(
                "Provider details are stale; refresh before changing them",
                cx,
            );
            return;
        };
        if !provider["__nativeConfiguration"].is_object() {
            self.report_error(
                "This provider instance is not explicitly configured on the server",
                cx,
            );
            return;
        }
        let Some(sender) = self.commands.clone() else {
            self.report_error("Not connected; provider change retained", cx);
            return;
        };
        let id = id.to_owned();
        let generation = self.generation;
        self.settings_provider_busy = Some(generation);
        self.settings_provider_task = Some(cx.spawn(async move |entity, cx| {
            let result = async {
                let settings = connection::call(&sender, "server.getSettings", json!({})).await?;
                let patch = enabled_patch(&settings, &id, enabled)?;
                connection::call(&sender, "server.updateSettings", patch).await
            }
            .await;
            let _ = entity.update(cx, |app, cx| {
                if app.generation != generation {
                    return;
                }
                app.settings_provider_busy = None;
                match result {
                    Ok(_) => app.refresh_settings_providers(true, cx),
                    Err(error) => app.report_error(error, cx),
                }
                cx.notify();
            });
        }));
        cx.notify();
    }
}
fn enabled_patch(settings: &Value, id: &str, enabled: bool) -> Result<Value, String> {
    // providerInstances is a whole-map replacement, unlike project override patches.
    let mut instances = settings["providerInstances"]
        .as_object()
        .cloned()
        .ok_or("Provider instances are not supported by this server")?;
    let configuration = instances
        .get_mut(id)
        .filter(|value| value.is_object())
        .ok_or("Provider instance was removed; refresh before changing it")?;
    configuration["enabled"] = enabled.into();
    Ok(json!({"patch":{"providerInstances":instances}}))
}

/// Modern terminal/credential/consent methods are not interactive in this
/// native slice: only browser callback flows can submit a pasted callback.
pub fn auth_active(state: &Value) -> bool {
    state["flowId"].is_string() && matches!(state["phase"].as_str(), Some("starting" | "waiting" | "verifying"))
}

pub fn accepts_callback(state: &Value) -> bool {
    matches!(state["phase"].as_str(), Some("waiting" | "verifying"))
        && state["flowId"].as_str().is_some_and(|id| !id.is_empty())
        && state["instanceId"].as_str().is_some_and(|id| !id.is_empty())
        && if state["interaction"].is_null() {
            state["authorizationUrl"].is_string()
        } else {
            state["interaction"]["type"] == "browser"
                && state["interaction"]["acceptsCallback"] == true
                && state["interaction"]["requiresConsent"] == false
        }
}

pub fn browser_url(state: &Value) -> Option<&str> {
    if matches!(state["phase"].as_str(), Some("waiting" | "verifying")) {
        match state["interaction"]["type"].as_str() {
            Some("browser") if state["interaction"]["requiresConsent"] == false => state["interaction"]["url"].as_str(),
            Some("deviceCode") => state["interaction"]["url"].as_str(),
            None if state["interaction"].is_null() => state["authorizationUrl"].as_str(),
            _ => None,
        }
    } else { None }
}

pub fn can_sign_in(provider: &Value) -> bool {
    provider["enabled"].as_bool() == Some(true)
        && provider["installed"].as_bool() == Some(true)
        && provider["availability"].as_str() != Some("unavailable")
        && provider["setup"]["canAuthenticate"].as_bool() == Some(true)
        && provider["auth"]["status"].as_str() == Some("unauthenticated")
}
pub fn details(provider: &Value) -> String {
    let version = provider["version"]
        .as_str()
        .unwrap_or("Version unavailable");
    let capabilities = provider["adapterCapabilities"]["features"]
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>()
                .join(", ")
        })
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "Capabilities not reported".into());
    let status = provider["status"].as_str().unwrap_or("Status unknown");
    let auth = provider["auth"]["status"]
        .as_str()
        .unwrap_or("Auth status unknown");
    format!("{status} · {auth} · {version} · {capabilities}")
}
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn enabled_patch_preserves_other_instances_and_rejects_removed_instance() {
        let settings = json!({"providerInstances":{"one":{"driver":"codex","enabled":true,"config":{"homePath":"/work"}},"two":{"driver":"custom","enabled":true,"config":{"secret":"keep"}}}});
        let patch = enabled_patch(&settings, "one", false).unwrap();
        assert_eq!(patch["patch"]["providerInstances"]["one"]["enabled"], false);
        assert_eq!(
            patch["patch"]["providerInstances"]["one"]["config"],
            settings["providerInstances"]["one"]["config"]
        );
        assert_eq!(
            patch["patch"]["providerInstances"]["two"],
            settings["providerInstances"]["two"]
        );
        assert!(enabled_patch(&settings, "removed", true).is_err());
    }
    #[test]
    fn sign_in_requires_descriptor_and_auth_need() {
        let mut provider = json!({"enabled":true,"installed":true,"setup":{"canAuthenticate":true},"auth":{"status":"unauthenticated"}});
        assert!(can_sign_in(&provider));
        provider["auth"]["status"] = json!("authenticated");
        assert!(!can_sign_in(&provider));
        provider["auth"]["status"] = json!("unknown");
        assert!(!can_sign_in(&provider));
        provider["auth"]["status"] = json!("unauthenticated");
        provider["setup"] = Value::Null;
        assert!(!can_sign_in(&provider));
        provider["setup"] = json!({"canAuthenticate":true});
        provider["enabled"] = json!(false);
        assert!(!can_sign_in(&provider));
        assert!(!can_sign_in(&Value::Null));
    }
}
