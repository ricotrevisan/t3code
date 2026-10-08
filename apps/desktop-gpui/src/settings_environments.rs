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
            entry.origin = origin;
        } else {
            if self.environments.len() >= 64 {
                return Err("Forget an environment before adding more (64 maximum)".into());
            }
            self.environments.push(Environment {
                origin,
                enabled: true,
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
}
impl NativeApp {
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
                    .child(div().text_sm().child(format!("{}{}", entry.origin, if active { " · active" } else { "" })))
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
