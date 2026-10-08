//! Saved endpoint metadata is device-local. Credentials stay in the origin-keyed vault.
use crate::NativeApp;
use gpui_kit::component::{
    Disableable, Sizable,
    button::{Button, ButtonVariants},
    switch::Switch,
};
use gpui_kit::{App, Context, Global, Window, div, prelude::*};
use serde::{Deserialize, Serialize};
use std::path::Path;
use t3code_desktop_gpui::{auth, credentials};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Environment {
    origin: String,
    enabled: bool,
    #[serde(default)]
    identity: Option<Identity>,
}
/// Server-owned identity is distinct from the credential/transport origin.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Identity {
    pub id: String,
    pub label: String,
}
#[derive(Clone, Default, Serialize, Deserialize)]
struct Saved {
    environments: Vec<Environment>,
}
#[derive(Default)]
struct State {
    saved: Saved,
    editing: Option<String>,
}
impl Global for State {}
fn path() -> std::path::PathBuf {
    t3code_desktop_gpui::drafts::default_path().with_file_name("environments.json")
}
impl Saved {
    fn load(path: &Path) -> Self {
        let mut saved: Self = std::fs::read(path)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default();
        let mut seen = std::collections::HashSet::new();
        saved.environments.retain_mut(|entry| {
            if let Ok(origin) = auth::origin(&entry.origin) {
                entry.origin = origin;
                seen.insert(entry.origin.clone())
            } else {
                false
            }
        });
        saved.environments.truncate(64);
        saved
    }
    fn save(&self, path: &Path) -> Result<(), String> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let temporary = path.with_extension("json.tmp");
        std::fs::write(
            &temporary,
            serde_json::to_vec_pretty(self).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
        std::fs::rename(temporary, path).map_err(|e| e.to_string())
    }
    fn upsert(&mut self, origin: &str, editing: Option<&str>) -> Result<(), String> {
        let origin = auth::origin(origin)?.to_owned();
        if self
            .environments
            .iter()
            .any(|e| e.origin == origin && Some(e.origin.as_str()) != editing)
        {
            return Err("This environment is already saved".into());
        }
        if let Some(old) = editing {
            let entry = self
                .environments
                .iter_mut()
                .find(|e| e.origin == old)
                .ok_or("Saved environment was removed")?;
            if entry.origin != origin {
                entry.identity = None;
            }
            entry.origin = origin;
        } else {
            if self.environments.len() >= 64 {
                return Err("Forget an environment before adding more (64 maximum)".into());
            }
            self.environments.push(Environment {
                origin,
                enabled: true,
                identity: None,
            });
        }
        Ok(())
    }
}
pub(crate) fn init(cx: &mut App) {
    cx.set_global(State {
        saved: if cfg!(test) {
            Saved::default()
        } else {
            Saved::load(&path())
        },
        editing: None,
    });
    crate::runtime_usage::init(cx);
}
impl NativeApp {
    /// Available endpoint choices only, not a claim of simultaneous sessions.
    pub(crate) fn environment_choices(&self, cx: &App) -> Vec<(String, String, bool)> {
        cx.global::<State>()
            .saved
            .environments
            .iter()
            .map(|entry| {
                (
                    entry.origin.clone(),
                    entry
                        .identity
                        .as_ref()
                        .map(|identity| identity.label.clone())
                        .unwrap_or_else(|| entry.origin.clone()),
                    entry.enabled,
                )
            })
            .collect()
    }

    /// Called only after an owned config receipt; never trust a picker label as identity.
    pub(crate) fn remember_environment_identity(
        &mut self,
        config: &serde_json::Value,
        cx: &mut Context<Self>,
    ) {
        let Some(id) = config["environment"]["environmentId"]
            .as_str()
            .filter(|s| !s.trim().is_empty())
        else {
            return;
        };
        let label = config["environment"]["label"]
            .as_str()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or(id);
        let mut saved = cx.global::<State>().saved.clone();
        let Some(entry) = saved
            .environments
            .iter_mut()
            .find(|entry| entry.origin == self.environment_scope)
        else {
            return;
        };
        let identity = Identity {
            id: id.into(),
            label: label.into(),
        };
        if entry.identity.as_ref() != Some(&identity) {
            entry.identity = Some(identity);
            self.persist_environments(saved, cx);
        }
    }

    fn persist_environments(&mut self, saved: Saved, cx: &mut Context<Self>) -> bool {
        if !cfg!(test)
            && let Err(error) = saved.save(&path())
        {
            self.report_error(format!("Could not save environments: {error}"), cx);
            return false;
        }
        cx.global_mut::<State>().saved = saved;
        cx.notify();
        true
    }
    pub(crate) fn save_environment(&mut self, cx: &mut Context<Self>) {
        let mut saved = cx.global::<State>().saved.clone();
        let editing = cx.global::<State>().editing.clone();
        if let Err(error) = saved.upsert(&self.server.read(cx).value(), editing.as_deref()) {
            self.report_error(error, cx);
            return;
        }
        if self.persist_environments(saved, cx) {
            cx.global_mut::<State>().editing = None;
            if let Some(config) = self.server_config.config(self.generation).cloned() {
                self.remember_environment_identity(&config, cx);
            }
            self.report_success("Environment saved on this device", cx);
        }
    }
    /// One request seam for switching: guard before any context or credential change.
    /// Existing transport primitives retain their generation and receipt fencing.
    pub(crate) fn switch_environment(
        &mut self,
        origin: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let origin = match auth::origin(origin) {
            Ok(origin) => origin,
            Err(error) => {
                self.report_error(error, cx);
                return;
            }
        };
        if !cx
            .global::<State>()
            .saved
            .environments
            .iter()
            .any(|e| e.origin == origin && e.enabled)
        {
            self.report_error("Enable and save this environment before switching", cx);
            return;
        }
        if !self.can_change_checkout(cx) {
            return;
        }
        self.disconnect(cx);
        self.access = None;
        self.use_dev_credential = false;
        self.server
            .update(cx, |input, cx| input.set_value(origin, window, cx));
        self.pairing
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.credential_operation(credentials::Operation::Load, cx);
    }
    pub(crate) fn render_saved_environments(
        &self,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let state = cx.global::<State>();
        let entries = state.saved.environments.clone();
        let editing = state.editing.is_some();
        div().flex().flex_col().gap_2()
            .child(div().text_sm().child("Saved environments · device-local"))
            .child(div().text_xs().child("Edit the server origin above, then save. Switching loads that origin’s Keychain credential; pair if none is remembered. Forget entry does not revoke credentials."))
            .child(div().flex().gap_2()
                .child(Button::new("environment-save").debug_selector(|| "environment-save".into()).label(if editing { "Save environment edit" } else { "Save environment" }).secondary().small()
                    .on_click(cx.listener(|app, _, _, cx| app.save_environment(cx))))
                .when(editing, |view| view.child(Button::new("environment-edit-cancel").label("Cancel edit").ghost().small()
                    .on_click(cx.listener(|_, _, _, cx| { cx.global_mut::<State>().editing = None; cx.notify(); })))))
            .children(entries.into_iter().enumerate().map(|(index, entry)| {
                let toggle = entry.origin.clone();
                let switch = entry.origin.clone();
                let edit = entry.origin.clone();
                let forget = entry.origin.clone();
                let active = self.commands.is_some() && self.access.as_ref().is_some_and(|a| a.origin == entry.origin);
                div().flex().flex_col().gap_1()
                    .child(div().text_sm().child(format!("{}{}", entry.identity.as_ref().map(|identity| identity.label.as_str()).unwrap_or(&entry.origin), if active { " · active" } else { "" })))
                    .when(entry.identity.is_some(), |view| view.child(div().text_xs().child(entry.origin.clone())))
                    .child(div().flex().items_center().gap_1()
                        .child(Switch::new(("environment-enabled", index)).label("Enabled").checked(entry.enabled).small()
                            .on_click(cx.listener(move |app, enabled: &bool, _, cx| {
                                if !*enabled && active && !app.can_change_checkout(cx) { return; }
                                let mut saved = cx.global::<State>().saved.clone();
                                if let Some(entry) = saved.environments.iter_mut().find(|e| e.origin == toggle) { entry.enabled = *enabled; }
                                if app.persist_environments(saved, cx) && !*enabled && active { app.disconnect(cx); }
                            })))
                        .child(Button::new(("environment-switch", index)).label("Switch").secondary().small().disabled(!entry.enabled || active)
                            .on_click(cx.listener(move |app, _, window, cx| app.switch_environment(&switch, window, cx))))
                        .child(Button::new(("environment-edit", index)).label("Edit").ghost().small()
                            .on_click(cx.listener(move |app, _, window, cx| {
                                cx.global_mut::<State>().editing = Some(edit.clone());
                                app.server.update(cx, |input, cx| input.set_value(edit.clone(), window, cx));
                                cx.notify();
                            })))
                        .child(Button::new(("environment-forget", index)).label("Forget entry").ghost().small()
                            .on_click(cx.listener(move |app, _, _, cx| {
                                if active && !app.can_change_checkout(cx) { return; }
                                let mut saved = cx.global::<State>().saved.clone();
                                saved.environments.retain(|e| e.origin != forget);
                                if app.persist_environments(saved, cx) {
                                    if active { app.disconnect(cx); }
                                    if cx.global::<State>().editing.as_ref() == Some(&forget) { cx.global_mut::<State>().editing = None; }
                                }
                            }))))
            }))
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn r13_g_environment_identity_migrates_and_origin_edit_drops_old_identity() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("environments.json");
        std::fs::write(
            &path,
            r#"{"environments":[{"origin":"https://office.example/","enabled":true}]}"#,
        )
        .unwrap();
        let mut saved = Saved::load(&path);
        assert_eq!(saved.environments[0].origin, "https://office.example");
        assert!(saved.environments[0].identity.is_none());
        saved.environments[0].identity = Some(Identity {
            id: "office-id".into(),
            label: "Office Mac".into(),
        });
        saved.save(&path).unwrap();
        assert_eq!(Saved::load(&path).environments, saved.environments);
        saved
            .upsert("https://office.example", Some("https://office.example"))
            .unwrap();
        assert!(saved.environments[0].identity.is_some());
        saved
            .upsert("https://lab.example", Some("https://office.example"))
            .unwrap();
        assert!(
            saved.environments[0].identity.is_none(),
            "endpoint edit must not inherit another server's identity"
        );
    }

    #[test]
    fn settings_fixc_saved_environments_edit_deduplicate_enable_and_restart() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("environments.json");
        let mut saved = Saved::default();
        saved.upsert("https://one.example/", None).unwrap();
        assert!(saved.upsert("https://one.example", None).is_err());
        saved.upsert("https://two.example", None).unwrap();
        assert!(
            saved
                .upsert("https://two.example", Some("https://one.example"))
                .is_err()
        );
        saved
            .upsert("https://new.example", Some("https://one.example"))
            .unwrap();
        saved.environments[0].enabled = false;
        saved.save(&path).unwrap();
        assert_eq!(Saved::load(&path).environments, saved.environments);
        saved
            .environments
            .retain(|e| e.origin != "https://new.example");
        saved.save(&path).unwrap();
        assert_eq!(Saved::load(&path).environments.len(), 1);
        assert!(saved.upsert("file:///tmp/repo", None).is_err());
    }
}
