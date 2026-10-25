//! Provider envelopes are edited on the owning environment page, never in a native shadow store.
use crate::NativeApp;
use gpui_kit::component::{
    ActiveTheme, Disableable, Sizable,
    button::{Button, ButtonVariants},
    input::{Input, InputState},
};
use gpui_kit::{AppContext, Context, Entity, IntoElement, Task, Window, div, prelude::*};
use serde_json::{Value, json};
use t3code_desktop_gpui::connection;

pub(crate) struct Editor {
    pub(crate) id: Entity<InputState>,
    pub(crate) name: Entity<InputState>,
    pub(crate) binary: Entity<InputState>,
    fields: [Entity<InputState>; 4],
    auth_method: usize,
    source: Option<String>,
    base: Value,
    generation: u64,
    duplicate: bool,
    remove_confirm: bool,
    pub(crate) busy: bool,
    pub(crate) error: Option<String>,
    task: Option<Task<()>>,
}
impl Editor {
    pub(crate) fn is_open(&self) -> bool {
        self.source.is_some()
    }
    pub fn new(window: &mut Window, cx: &mut gpui_kit::App) -> Self {
        Self {
            id: cx.new(|cx| InputState::new(window, cx).placeholder("Unique instance ID")),
            name: cx.new(|cx| InputState::new(window, cx).placeholder("Display name (optional)")),
            binary: cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder("Executable on the environment host (blank = driver default)")
            }),
            fields: std::array::from_fn(|_| cx.new(|cx| InputState::new(window, cx))),
            auth_method: 0,
            source: None,
            base: Value::Null,
            generation: 0,
            duplicate: false,
            remove_confirm: false,
            busy: false,
            error: None,
            task: None,
        }
    }
}
fn binary_supported(base: &Value) -> bool {
    base["adapterPackage"].is_null()
        && matches!(
            base["driver"].as_str(),
            Some(
                "codex"
                    | "claudeAgent"
                    | "cursor"
                    | "grok"
                    | "antigravity"
                    | "primeAgent"
                    | "opencode"
            )
        )
}
// These keys/control types follow contracts/settings.ts, not guessed adapter config.
fn config_fields(base: &Value) -> Vec<(&'static str, &'static str, bool)> {
    if !binary_supported(base) {
        return vec![];
    }
    match base["driver"].as_str() {
        Some("codex") => vec![
            ("homePath", "CODEX_HOME path", false),
            ("shadowHomePath", "Shadow home path", false),
            ("launchArgs", "Launch arguments", false),
        ],
        Some("claudeAgent") => vec![
            ("homePath", "CLAUDE_CONFIG_DIR path", false),
            (
                "autoCompactWindow",
                "Auto-compact after (100,000–1,000,000 tokens)",
                false,
            ),
            ("launchArgs", "Launch arguments", false),
        ],
        Some("cursor") => vec![("apiEndpoint", "API endpoint", false)],
        Some("primeAgent") => vec![("launchArgs", "Launch arguments", false)],
        Some("opencode") => vec![
            ("serverUrl", "Server URL (blank = spawn on host)", false),
            (
                "serverPassword",
                "Server password (stored in plain text on host)",
                true,
            ),
        ],
        Some("antigravity") => vec![
            ("apiKey", "API key (stored in plain text on host)", true),
            ("gcpProject", "GCP project", false),
            ("gcpLocation", "GCP location", false),
        ],
        _ => vec![],
    }
}
const AUTH_METHODS: [&str; 4] = [
    "oauth-personal",
    "oauth-business",
    "gemini-api-key",
    "agent-platform",
];
fn configure_fields(
    base: &Value,
    next: &mut Value,
    fields: &[String],
    auth_method: usize,
) -> Result<(), String> {
    if !binary_supported(base) {
        return Ok(());
    }
    for ((key, _, _), value) in config_fields(base).into_iter().zip(fields) {
        if !base["config"][key].is_null() && !base["config"][key].is_string() {
            return Err(format!(
                "Unsupported stored {key} value; configuration preserved"
            ));
        }
        let value = value.trim();
        if key == "autoCompactWindow"
            && !value.is_empty()
            && !value
                .parse::<u32>()
                .is_ok_and(|n| (100_000..=1_000_000).contains(&n) && n.to_string() == value)
        {
            return Err("Auto-compact window must be 100,000–1,000,000 tokens, or blank".into());
        }
        if value.is_empty() {
            next["config"]
                .as_object_mut()
                .ok_or("Invalid configuration")?
                .remove(key);
        } else {
            next["config"][key] = value.into();
        }
    }
    if base["driver"] == "antigravity" {
        next["config"]["authMethod"] = AUTH_METHODS
            .get(auth_method)
            .ok_or("Invalid authentication method")?
            .to_owned()
            .into();
    }
    Ok(())
}
fn slug(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id.as_bytes()[0].is_ascii_alphabetic()
        && id
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
}
fn patch(
    settings: &Value,
    source: &str,
    base: &Value,
    destination: &str,
    replacement: Option<Value>,
) -> Result<Value, String> {
    let mut instances = settings["providerInstances"]
        .as_object()
        .cloned()
        .ok_or("Provider instances unavailable on this server")?;
    if instances.get(source) != Some(base) {
        return Err("Instance changed or was removed; cancel and reopen before saving".into());
    }
    if destination != source && instances.contains_key(destination) {
        return Err("Instance ID already exists; choose another ID".into());
    }
    match replacement {
        Some(value) => {
            instances.insert(destination.into(), value);
        }
        None => {
            instances.remove(source);
        }
    }
    Ok(json!({"patch":{"providerInstances":instances}}))
}
fn replacement(base: &Value, name: &str, binary: &str, duplicate: bool) -> Result<Value, String> {
    if duplicate
        && base["environment"]
            .as_array()
            .is_some_and(|env| env.iter().any(|v| v["valueRedacted"] == true))
    {
        return Err("Cannot duplicate redacted environment credentials; configure a new instance in the web client".into());
    }
    let mut value = base.clone();
    let object = value
        .as_object_mut()
        .ok_or("Invalid instance configuration")?;
    if name.trim().is_empty() {
        object.remove("displayName");
    } else {
        object.insert("displayName".into(), name.trim().into());
    }
    if binary_supported(base) {
        let config = object.entry("config").or_insert_with(|| json!({}));
        if config.is_null() {
            *config = json!({});
        }
        let config = config
            .as_object_mut()
            .ok_or("This configuration is not a supported object; existing values preserved")?;
        if binary.trim().is_empty() {
            if base["driver"] == "antigravity" {
                config.insert("binaryPath".into(), "".into());
            } else {
                config.remove("binaryPath");
            }
        } else {
            config.insert("binaryPath".into(), binary.trim().into());
        }
    }
    Ok(value)
}
impl NativeApp {
    pub(crate) fn open_provider_instance(
        &mut self,
        id: &str,
        duplicate: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.provider_instance_editor.is_open()
            || self.settings_provider_busy == Some(self.generation)
            || !self.connection_ready()
        {
            return;
        }
        let Some(sender) = self.commands.clone() else {
            return;
        };
        let generation = self.generation;
        let id = id.to_owned();
        let editor = &mut self.provider_instance_editor;
        editor.source = Some(id.clone());
        editor.base = Value::Null;
        editor.generation = generation;
        editor.duplicate = duplicate;
        editor.remove_confirm = false;
        editor.error = None;
        editor.busy = true;
        editor.id.update(cx, |input, cx| {
            input.set_value(
                if duplicate {
                    format!("{id}_copy")
                } else {
                    id.clone()
                },
                window,
                cx,
            )
        });
        editor
            .name
            .update(cx, |input, cx| input.set_value("", window, cx));
        editor
            .binary
            .update(cx, |input, cx| input.set_value("", window, cx));
        editor.task = Some(cx.spawn(async move |entity, cx| {
            let result = connection::call(&sender, "server.getSettings", json!({})).await;
            let _ = entity.update_in(cx, |app, window, cx| {
                if app.generation != generation { return; }
                let editor = &mut app.provider_instance_editor;
                editor.busy = false;
                match result {
                    Ok(settings) => {
                        if let Some(base) = settings["providerInstances"].get(&id).filter(|v| v.is_object()) {
                            editor.base = base.clone();
                            for (index, (key, _, sensitive)) in config_fields(base).into_iter().enumerate() {
                                editor.fields[index] = cx.new(|cx| InputState::new(window, cx).masked(sensitive));
                                editor.fields[index].update(cx, |input, cx| input.set_value(base["config"][key].as_str().unwrap_or(""), window, cx));
                            }
                            editor.auth_method = AUTH_METHODS.iter().position(|method| base["config"]["authMethod"] == *method).unwrap_or(0);
                            editor.name.update(cx, |input, cx| input.set_value(base["displayName"].as_str().unwrap_or(""), window, cx));
                            editor.binary.update(cx, |input, cx| input.set_value(base["config"]["binaryPath"].as_str().unwrap_or(""), window, cx));
                        } else { editor.error = Some("Instance is not explicitly configured; use the web client to configure this legacy default".into()); }
                    }
                    Err(error) => editor.error = Some(error),
                }
                cx.notify();
            });
        }));
        cx.notify();
    }
    pub(crate) fn save_provider_instance(&mut self, remove: bool, cx: &mut Context<Self>) {
        let editor = &self.provider_instance_editor;
        if editor.busy
            || self.settings_provider_busy == Some(self.generation)
            || editor.generation != self.generation
            || !editor.base.is_object()
            || !self.connection_ready()
        {
            return;
        }
        if remove && (!editor.remove_confirm || editor.duplicate) {
            return;
        }
        let editor = &mut self.provider_instance_editor;
        let Some(source) = editor.source.clone() else {
            return;
        };
        let destination = if editor.duplicate {
            editor.id.read(cx).value().trim().to_owned()
        } else {
            source.clone()
        };
        if !slug(&destination) {
            editor.error = Some(
                "ID must start with a letter, use letters/digits/-/_, and be at most 64 characters"
                    .into(),
            );
            cx.notify();
            return;
        }
        if editor.duplicate && destination == source {
            editor.error = Some("Choose a new instance ID".into());
            cx.notify();
            return;
        }
        let next = if remove {
            Ok(None)
        } else {
            replacement(
                &editor.base,
                &editor.name.read(cx).value(),
                &editor.binary.read(cx).value(),
                editor.duplicate,
            )
            .and_then(|mut value| {
                let fields = editor
                    .fields
                    .iter()
                    .map(|input| input.read(cx).value().to_string())
                    .collect::<Vec<_>>();
                configure_fields(&editor.base, &mut value, &fields, editor.auth_method)?;
                Ok(Some(value))
            })
        };
        let next = match next {
            Ok(value) => value,
            Err(error) => {
                editor.error = Some(error);
                cx.notify();
                return;
            }
        };
        let Some(sender) = self.commands.clone() else {
            return;
        };
        let base = editor.base.clone();
        let generation = self.generation;
        editor.busy = true;
        editor.error = None;
        self.settings_provider_busy = Some(generation);
        editor.task = Some(cx.spawn(async move |entity, cx| {
            let result = async {
                let settings = connection::call(&sender, "server.getSettings", json!({})).await?;
                if !entity
                    .update(cx, |app, _| {
                        app.generation == generation
                            && app.connection_ready()
                            && app.provider_instance_editor.source.as_deref()
                                == Some(source.as_str())
                    })
                    .unwrap_or(false)
                {
                    return Err("Connection changed before saving; reopen the instance".into());
                }
                let payload = patch(&settings, &source, &base, &destination, next)?;
                let observed = entity
                    .update(cx, |app, _| app.project_settings.clone())
                    .ok()
                    .flatten();
                let settings = connection::call(&sender, "server.updateSettings", payload).await?;
                if !settings["providerInstances"].is_object() {
                    return Err(
                        "Save accepted, but settings receipt was invalid; reload before retrying"
                            .into(),
                    );
                }
                Ok::<_, String>((settings, observed))
            }
            .await;
            let _ = entity.update(cx, |app, cx| {
                if app.generation != generation {
                    return;
                }
                app.provider_instance_editor.busy = false;
                app.settings_provider_busy = None;
                match result {
                    Ok((settings, observed)) => {
                        // A newer settings stream owns projection precedence over this unary receipt.
                        if app.project_settings == observed {
                            app.apply_server_config_event(
                                json!({"type":"settingsUpdated","payload":{"settings":settings}}),
                                cx,
                            );
                        }
                        app.provider_instance_editor.source = None;
                        app.refresh_settings_providers(true, cx);
                    }
                    Err(error) => app.provider_instance_editor.error = Some(error),
                }
                cx.notify();
            });
        }));
        cx.notify();
    }
    pub(crate) fn render_provider_instance_editor(
        &self,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let editor = &self.provider_instance_editor;
        let active = editor.source.is_some();
        let ready = !editor.busy
            && self.settings_provider_busy != Some(self.generation)
            && editor.generation == self.generation
            && editor.base.is_object()
            && self.connection_ready();
        div().flex().flex_col().gap_2().when(active, |view| {
            view.child(div().text_sm().child(if editor.duplicate { "Add instance from existing configuration" } else { "Edit provider instance" }))
                .child(div().text_xs().child("This environment only. Unedited driver settings and credentials are preserved. Removing an instance does not delete its threads or uninstall its executable."))
                .when(editor.duplicate, |view| view.child(div().debug_selector(|| "provider-instance-id".into()).child(Input::new(&editor.id).disabled(!ready))))
                .child(div().text_xs().child("Display name"))
                .child(div().debug_selector(|| "provider-instance-name".into()).child(Input::new(&editor.name).disabled(!ready)))
                .when(binary_supported(&editor.base), |view| view.child(div().text_xs().child("Executable path on environment host"))
                    .child(div().debug_selector(|| "provider-instance-binary".into()).child(Input::new(&editor.binary).disabled(!ready))))
                .children(config_fields(&editor.base).into_iter().enumerate().map(|(index, (key, label, _))| div().flex().flex_col().gap_1()
                    .child(div().text_xs().child(label))
                    .child(div().debug_selector(move || format!("provider-instance-field-{key}")).child(Input::new(&editor.fields[index]).disabled(!ready)))))
                .when(editor.base["driver"] == "antigravity" && binary_supported(&editor.base), |view| {
                    use gpui_kit::component::radio::{Radio, RadioGroup};
                    view.child(div().text_xs().child("Sign-in method. API key / Agent Platform bill usage; account methods use subscription."))
                        .child(RadioGroup::vertical("provider-instance-auth-method").selected_index(Some(editor.auth_method)).disabled(!ready)
                            .children(["Google account", "Gemini Enterprise", "Gemini API key", "Agent Platform (Vertex AI)"].into_iter().enumerate().map(|(i, label)| Radio::new(("provider-instance-auth", i)).label(label)))
                            .on_click(cx.listener(|app, index: &usize, _, cx| { app.provider_instance_editor.auth_method = *index; cx.notify(); })))
                })
                .when(editor.busy, |view| view.child("Waiting for server…"))
                .when(editor.generation != self.generation, |view| view.child("Connection changed; cancel and reopen this editor"))
                .when_some(editor.error.clone(), |view, error| view.child(div().text_xs().text_color(cx.theme().danger).child(error)))
                .when(editor.remove_confirm, |view| view.child(div().text_sm().child("Remove this instance? Existing threads keep their routing ID and may become unavailable. Credentials and executable will not be deleted."))
                    .child(Button::new("provider-instance-confirm-remove").debug_selector(|| "provider-instance-confirm-remove".into()).label("Confirm removal").danger().small().disabled(!ready)
                        .on_click(cx.listener(|app, _, _, cx| app.save_provider_instance(true, cx))))
                    .child(Button::new("provider-instance-keep").label("Keep instance").ghost().small().disabled(!ready)
                        .on_click(cx.listener(|app, _, _, cx| { app.provider_instance_editor.remove_confirm = false; cx.notify(); }))))
                .child(div().flex().flex_wrap().gap_2()
                    .child(Button::new("provider-instance-save").debug_selector(|| "provider-instance-save".into()).label("Save instance").primary().small().disabled(!ready)
                        .on_click(cx.listener(|app, _, _, cx| app.save_provider_instance(false, cx))))
                    .when(!editor.duplicate, |view| view.child(Button::new("provider-instance-remove").debug_selector(|| "provider-instance-remove".into()).label("Remove instance…").danger().small().disabled(!ready)
                        .on_click(cx.listener(|app, _, _, cx| { app.provider_instance_editor.remove_confirm = true; cx.notify(); }))))
                    .child(Button::new("provider-instance-cancel").debug_selector(|| "provider-instance-cancel".into()).label("Cancel").ghost().small().disabled(editor.busy && editor.generation == self.generation)
                        .on_click(cx.listener(|app, _, _, cx| { app.provider_instance_editor.source = None; app.provider_instance_editor.busy = false; cx.notify(); }))))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn r18_f_typed_config_validation_and_driver_boundaries() {
        let base = json!({"driver":"claudeAgent","config":{"homePath":"/keep","customModels":[{"slug":"custom"}]}});
        let mut next = replacement(&base, "", "claude", false).unwrap();
        assert!(
            configure_fields(
                &base,
                &mut next,
                &["/home".into(), "99999".into(), "--chrome".into()],
                0
            )
            .is_err()
        );
        configure_fields(
            &base,
            &mut next,
            &["/home".into(), "1000000".into(), "--chrome".into()],
            0,
        )
        .unwrap();
        assert_eq!(next["config"]["autoCompactWindow"], "1000000");
        assert_eq!(
            next["config"]["customModels"],
            base["config"]["customModels"]
        );
        let external =
            json!({"driver":"cursor","adapterPackage":{"name":"external"},"config":{"opaque":1}});
        let mut next = replacement(&external, "External", "must-not-apply", false).unwrap();
        configure_fields(&external, &mut next, &["must-not-apply".into()], 0).unwrap();
        assert_eq!(next["config"], external["config"]);
        let antigravity = json!({"driver":"antigravity","config":{}});
        let mut next = replacement(&antigravity, "", "", false).unwrap();
        configure_fields(
            &antigravity,
            &mut next,
            &["key".into(), "project".into(), "region".into()],
            2,
        )
        .unwrap();
        assert_eq!(
            next["config"],
            json!({"binaryPath":"","authMethod":"gemini-api-key","apiKey":"key","gcpProject":"project","gcpLocation":"region"})
        );
    }
    #[test]
    fn r18_f_instance_slug_validation_matches_contract() {
        assert!(slug("My_instance-42"));
        for id in ["", "1bad", "two words", "../bad", "with.dot", "ébad"] {
            assert!(!slug(id));
        }
        assert!(!slug(&"a".repeat(65)));
    }
    #[test]
    fn r18_f_instance_patch_preserves_unrelated_values_and_detects_conflict() {
        let base = json!({"driver":"codex","config":{"binaryPath":"old","homePath":"/home","launchArgs":["--foo"]},"environment":[{"name":"TOKEN","value":"","sensitive":true,"valueRedacted":true}]});
        let settings = json!({"providerInstances":{"one":base,"other":{"driver":"custom","config":{"unknown":9}}}});
        let next = replacement(&base, "Renamed", "new", false).unwrap();
        let result = patch(&settings, "one", &base, "one", Some(next)).unwrap();
        assert_eq!(
            result["patch"]["providerInstances"]["one"]["config"]["homePath"],
            "/home"
        );
        assert_eq!(
            result["patch"]["providerInstances"]["one"]["environment"],
            base["environment"]
        );
        assert_eq!(
            result["patch"]["providerInstances"]["other"],
            settings["providerInstances"]["other"]
        );
        assert!(replacement(&base, "Copy", "new", true).is_err());
        assert!(patch(&settings, "one", &json!({}), "one", None).is_err());
        assert!(patch(&settings, "one", &base, "other", Some(base.clone())).is_err());
        assert!(
            patch(&settings, "one", &base, "one", None).unwrap()["patch"]["providerInstances"]
                .get("one")
                .is_none()
        );
    }
}
