//! Direct saved-action entry; configuration remains a separate entry point.
use crate::{InspectorSection, NativeApp, RightTab};
use gpui_kit::component::{
    Disableable, Sizable,
    button::{Button, ButtonVariants},
    menu::{DropdownMenu, PopupMenuItem},
};
use gpui_kit::{Context, div, prelude::*};
use serde_json::Value;
use t3code_desktop_gpui::{palette, project_actions};
#[derive(Default, serde::Serialize, serde::Deserialize)]
pub(crate) struct ActionPreferences(std::collections::BTreeMap<String, String>);
impl ActionPreferences {
    pub(crate) fn restore() -> Self {
        #[cfg(test)]
        {
            Self::default()
        }
        #[cfg(not(test))]
        {
            std::fs::read(
                t3code_desktop_gpui::drafts::default_path()
                    .with_file_name("preferred-actions.json"),
            )
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default()
        }
    }
    fn key(scope: &str, project: &str) -> String {
        serde_json::to_string(&(scope, project)).unwrap()
    }
}
#[derive(Clone)]
struct ActionTarget {
    generation: u64,
    environment: String,
    checkout: (Option<String>, Option<String>, Option<String>),
}
impl NativeApp {
    pub(crate) fn remember_header_action(&mut self, id: &str, cx: &mut Context<Self>) {
        let Some(project) = self.selected_project.as_deref() else {
            return;
        };
        self.header_action_preferences.0.insert(
            ActionPreferences::key(&self.environment_scope, project),
            id.into(),
        );
        #[cfg(not(test))]
        if let Err(error) = super::checkout_operations::persist(
            &t3code_desktop_gpui::drafts::default_path().with_file_name("preferred-actions.json"),
            &serde_json::to_vec(&self.header_action_preferences).unwrap(),
        ) {
            self.report_error(format!("Could not save preferred action: {error}"), cx);
        }
        cx.notify();
    }
    pub(crate) fn header_scripts(&self) -> Vec<Value> {
        self.selected_project
            .as_ref()
            .and_then(|id| {
                let project = self.shell.projects.iter().find(|p| &p.id == id)?;
                Some(project_actions::resolve(
                    self.project_settings.as_ref()?,
                    id,
                    &project.scripts,
                ))
            })
            .unwrap_or_default()
    }
    pub(crate) fn primary_header_script(&self, scripts: &[Value]) -> Option<Value> {
        let preferred = self
            .action_runs
            .iter()
            .rev()
            .find(|run| {
                run.scope == self.environment_scope
                    && Some(&run.project) == self.selected_project.as_ref()
            })
            .map(|run| &run.script["id"]);
        let saved = self.selected_project.as_deref().and_then(|project| {
            self.header_action_preferences
                .0
                .get(&ActionPreferences::key(&self.environment_scope, project))
        });
        preferred
            .and_then(|id| scripts.iter().find(|s| &s["id"] == id))
            .or_else(|| saved.and_then(|id| scripts.iter().find(|s| s["id"] == *id)))
            .or_else(|| scripts.iter().find(|s| s["runOnWorktreeCreate"] != true))
            .or_else(|| scripts.first())
            .cloned()
    }
    fn run_header_script(&mut self, id: &str, target: &ActionTarget, cx: &mut Context<Self>) {
        if !self.connection_ready() {
            return;
        }
        if target.generation != self.generation
            || target.environment != self.environment_scope
            || target.checkout != self.checkout_context()
        {
            self.report_error(
                "Checkout changed; reopen the action menu before running",
                cx,
            );
            return;
        }
        self.remember_header_action(id, cx);
        self.inspector_open = true;
        self.right_tab = RightTab::Inspector;
        self.inspector_section = Some(InspectorSection::ProjectActions);
        self.run_project_action(id, cx);
    }
    pub(crate) fn render_header_actions(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let scripts = self.header_scripts();
        let primary = self.primary_header_script(&scripts);
        let cwd = self.selected_workspace();
        let ready = self.connection_ready() && self.selected_thread.is_some();
        let busy = |id: &str| {
            self.action_runs.iter().any(|run| {
                run.busy
                    && run.generation == self.generation
                    && run.scope == self.environment_scope
                    && Some(&run.thread) == self.selected_thread.as_ref()
                    && Some(&run.cwd) == cwd.as_ref()
                    && run.script["id"] == id
            })
        };
        let choices = scripts
            .iter()
            .filter_map(|script| {
                Some((
                    script["id"].as_str()?.to_owned(),
                    script["name"].as_str().unwrap_or("Action").to_owned(),
                    busy(script["id"].as_str()?),
                ))
            })
            .collect::<Vec<_>>();
        let entity = cx.entity().downgrade();
        let target = ActionTarget {
            generation: self.generation,
            environment: self.environment_scope.clone(),
            checkout: self.checkout_context(),
        };
        let primary_target = target.clone();
        div()
            .flex()
            .gap_1()
            .when_some(primary, |view, script| {
                let id = script["id"].as_str().unwrap_or_default().to_owned();
                view.child(
                    Button::new("header-run-action")
                        .debug_selector(|| "header-run-action".into())
                        .small()
                        .secondary()
                        .label(script["name"].as_str().unwrap_or("Run action").to_owned())
                        .tooltip(script["command"].as_str().unwrap_or_default().to_owned())
                        .disabled(!ready || busy(&id))
                        .on_click(cx.listener(move |app, _, _, cx| {
                            app.run_header_script(&id, &primary_target, cx)
                        })),
                )
            })
            .child(
                Button::new("header-actions-menu")
                    .debug_selector(|| "header-actions-menu".into())
                    .ghost()
                    .small()
                    .label("Actions…")
                    .dropdown_menu(move |mut menu, _, _| {
                        for (id, name, busy) in &choices {
                            let id = id.clone();
                            let entity = entity.clone();
                            let target = target.clone();
                            menu = menu.item(
                                PopupMenuItem::new(name.clone())
                                    .disabled(!ready || *busy)
                                    .on_click(move |_, _, cx| {
                                        let _ = entity.update(cx, |app, cx| {
                                            app.run_header_script(&id, &target, cx)
                                        });
                                    }),
                            );
                        }
                        let entity = entity.clone();
                        menu.separator()
                            .item(PopupMenuItem::new("Configure actions…").on_click(
                                move |_, window, cx| {
                                    let _ = entity.update(cx, |app, cx| {
                                        app.execute_command(
                                            palette::Command::ProjectActions,
                                            window,
                                            cx,
                                        )
                                    });
                                },
                            ))
                    }),
            )
            .child(
                Button::new("project-actions-header")
                    .debug_selector(|| "project-actions-header".into())
                    .ghost()
                    .small()
                    .label("Setup")
                    .tooltip("Configure project actions (does not run)")
                    .on_click(cx.listener(|app, _, window, cx| {
                        app.execute_command(palette::Command::ProjectActions, window, cx)
                    })),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn r13_e_action_preference_roundtrip_keeps_environment_project_ownership() {
        let mut prefs = ActionPreferences::default();
        prefs
            .0
            .insert(ActionPreferences::key("https://office", "p"), "test".into());
        prefs
            .0
            .insert(ActionPreferences::key("https://lab", "p"), "build".into());
        let restored: ActionPreferences =
            serde_json::from_slice(&serde_json::to_vec(&prefs).unwrap()).unwrap();
        assert_eq!(
            restored.0[&ActionPreferences::key("https://office", "p")],
            "test"
        );
        assert_eq!(
            restored.0[&ActionPreferences::key("https://lab", "p")],
            "build"
        );
        assert!(
            !restored
                .0
                .contains_key(&ActionPreferences::key("https://office", "other"))
        );
    }
}
