//! Server-backed preferences capture their environment/project owner before transport.
use crate::NativeApp;
use gpui_kit::component::{
    ActiveTheme, Disableable, Selectable, Sizable,
    button::{Button, ButtonVariants},
    switch::Switch,
};
use gpui_kit::{Context, Task, div, prelude::*};
use serde_json::{Value, json};
use t3code_desktop_gpui::connection;

#[derive(Default)]
pub(crate) struct ScopedPages {
    pub(crate) generation: u64,
    pub(crate) environment: String,
    pub(crate) settings: Option<Value>,
    pub(crate) processes: Option<Value>,
    pub(crate) traces: Option<Value>,
    pub(crate) busy: bool,
    pub(crate) error: Option<String>,
    task: Option<Task<()>>,
}

fn scoped_patch(settings: &Value, project: Option<&str>, key: &str, value: Value) -> Value {
    if let Some(id) = project {
        let mut overrides = settings["projectSettingsOverrides"]
            .as_object()
            .cloned()
            .unwrap_or_default();
        let mut entry = overrides
            .get(id)
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        if value.is_null() {
            entry.remove(key);
        } else {
            entry.insert(key.into(), value);
        }
        overrides.insert(id.into(), Value::Object(entry));
        json!({"projectSettingsOverrides": overrides})
    } else {
        json!({key: value})
    }
}
impl NativeApp {
    /// Uses this environment page's authoritative settings snapshot. Creation
    /// callers pass captured project/file data and only owner-scoped choices.
    pub(crate) fn effective_thread_defaults(
        &self,
        project: &str,
        legacy_workspace: Option<&str>,
        project_file: Option<&Value>,
        explicit: Option<&t3code_desktop_gpui::project_actions::DraftDefaultsOverride>,
    ) -> Result<t3code_desktop_gpui::project_actions::ThreadDefaults, String> {
        t3code_desktop_gpui::project_actions::thread_defaults(
            self.project_settings.as_ref(),
            &self.environment_scope,
            project,
            legacy_workspace,
            project_file,
            explicit,
        )
    }

    pub(crate) fn refresh_scoped_pages(&mut self, diagnostics: bool, cx: &mut Context<Self>) {
        if self.scoped_pages.busy
            && self.scoped_pages.generation == self.generation
            && self.scoped_pages.environment == self.environment_scope
        {
            return;
        }
        let Some(sender) = self.commands.clone().filter(|_| self.connection_ready()) else {
            return;
        };
        let generation = self.generation;
        let environment = self.environment_scope.clone();
        self.scoped_pages = ScopedPages {
            generation,
            environment: environment.clone(),
            busy: true,
            ..Default::default()
        };
        self.scoped_pages.task = Some(cx.spawn(async move |entity, cx| {
            let result = async {
                let settings = connection::call(&sender, "server.getSettings", json!({})).await?;
                let processes = if diagnostics {
                    Some(
                        connection::call(&sender, "server.getProcessDiagnostics", json!({}))
                            .await?,
                    )
                } else {
                    None
                };
                let traces = if diagnostics {
                    Some(connection::call(&sender, "server.getTraceDiagnostics", json!({})).await?)
                } else {
                    None
                };
                Ok::<_, String>((settings, processes, traces))
            }
            .await;
            let _ = entity.update(cx, |app, cx| {
                if app.generation != generation || app.environment_scope != environment {
                    return;
                }
                app.scoped_pages.busy = false;
                match result {
                    Ok((settings, processes, traces)) => {
                        app.project_settings = Some(settings.clone());
                        app.scoped_pages.settings = Some(settings);
                        app.scoped_pages.processes = processes;
                        app.scoped_pages.traces = traces;
                    }
                    Err(error) => app.scoped_pages.error = Some(error),
                }
                cx.notify();
            });
        }));
        cx.notify();
    }
    fn update_scoped_default(
        &mut self,
        project: Option<String>,
        key: &'static str,
        value: Value,
        cx: &mut Context<Self>,
    ) {
        if self.scoped_pages.busy
            && self.scoped_pages.generation == self.generation
            && self.scoped_pages.environment == self.environment_scope
        {
            return;
        }
        let Some(sender) = self.commands.clone().filter(|_| self.connection_ready()) else {
            return;
        };
        let generation = self.generation;
        let environment = self.environment_scope.clone();
        self.scoped_pages.environment = environment.clone();
        self.scoped_pages.generation = generation;
        self.scoped_pages.busy = true;
        self.scoped_pages.error = None;
        self.scoped_pages.task = Some(cx.spawn(async move |entity, cx| {
            let result = async {
                let settings = connection::call(&sender, "server.getSettings", json!({})).await?;
                let patch = scoped_patch(&settings, project.as_deref(), key, value);
                connection::call(&sender, "server.updateSettings", json!({"patch":patch})).await
            }
            .await;
            let _ = entity.update(cx, |app, cx| {
                if app.generation != generation || app.environment_scope != environment {
                    return;
                }
                app.scoped_pages.busy = false;
                match result {
                    Ok(settings) => {
                        app.project_settings = Some(settings.clone());
                        app.scoped_pages.settings = Some(settings);
                    }
                    Err(error) => app.scoped_pages.error = Some(error),
                }
                cx.notify();
            });
        }));
        cx.notify();
    }
    pub(crate) fn render_scoped_pages(
        &self,
        diagnostics: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let owned = self.scoped_pages.generation == self.generation
            && self.scoped_pages.environment == self.environment_scope;
        let busy = owned && self.scoped_pages.busy;
        let settings = owned
            .then_some(self.scoped_pages.settings.as_ref())
            .flatten();
        let project = self.selected_project.clone();
        let project_supported = self
            .server_config
            .config(self.generation)
            .is_some_and(|v| v["environment"]["capabilities"]["projectSettingsOverrides"] == true);
        let disabled = !self.connection_ready() || busy || settings.is_none();
        let resolved = self.effective_thread_defaults(
            self.selected_project.as_deref().unwrap_or(""),
            None,
            None,
            None,
        );
        let storage_supported = self
            .server_config
            .config(self.generation)
            .is_some_and(|v| v["environment"]["capabilities"]["storageCleanup"] == true);
        div().flex().flex_col().gap_2()
            .child("Server preferences · selected environment")
            .when(!diagnostics, |view| view.child(match &resolved {
                Ok(defaults) => format!("Resolved settings · {} · {} (workspace may defer to checkout t3.json)", defaults.runtime_mode, defaults.workspace_mode),
                Err(error) => error.clone(),
            }))
            .when(self.selected_project.is_some() && !project_supported, |view| view.child("Project overrides unavailable: this server does not advertise project-scoped settings support."))
            .child(Button::new("scoped-refresh").debug_selector(|| "scoped-refresh".into()).label(if busy { "Loading…" } else { "Refresh server data" }).ghost().disabled(!self.connection_ready() || busy)
                .on_click(cx.listener(move |app, _, _, cx| app.refresh_scoped_pages(diagnostics, cx))))
            .when_some(owned.then_some(self.scoped_pages.error.clone()).flatten(), |view, error| view.child(div().text_color(cx.theme().danger).child(error)))
            .when(!diagnostics, |view| {
                view.children([None, project.filter(|_| project_supported)].into_iter().enumerate().filter(|(i,p)| *i == 0 || p.is_some()).map(|(index, project)| {
                    let effective = project.as_ref().and_then(|id| settings.map(|s| &s["projectSettingsOverrides"][id]));
                    div().flex().flex_col().gap_1().child(project.as_ref().map(|id| format!("Project {id} · override (Inherit removes only this field)")).unwrap_or_else(|| "Environment defaults".into()))
                        .children([("defaultThreadEnvMode", vec!["local", "worktree"]), ("defaultRuntimeMode", vec!["approval-required", "auto-accept-edits", "auto", "full-access"])].into_iter().map(|(key, values)| {
                            let override_value = effective.and_then(|s| s.get(key)).and_then(Value::as_str);
                            let current = if project.is_some() { override_value.unwrap_or("inherit") } else { settings.and_then(|s| s.get(key)).and_then(Value::as_str).unwrap_or("inherit") };
                            div().flex().flex_wrap().gap_1().child(format!("{}: {current}", if key == "defaultThreadEnvMode" { "Workspace default" } else { "Permissions default" }))
                                .children(values.into_iter().chain((project.is_some() || key == "defaultThreadEnvMode").then_some("inherit")).enumerate().map(|(choice, value)| {
                                    let project = project.clone();
                                    Button::new(gpui_kit::SharedString::from(format!("scope-{index}-{key}-{choice}"))).debug_selector(move || format!("scope-{index}-{key}-{choice}")).label(value).ghost().small().selected(current == value).disabled(disabled)
                                        .on_click(cx.listener(move |app, _, _, cx| app.update_scoped_default(project.clone(), key, if value == "inherit" { Value::Null } else { json!(value) }, cx)))
                                }))
                        }))
                })).when_some(settings.and_then(|s| s.get("storageCleanup")), |view, storage| view
                    .child("Environment storage retention · automatic cleanup keeps worktrees with local changes")
                    .when(!storage_supported, |view| view.child("Automatic storage cleanup controls require a server advertising storageCleanup support."))
                    .children([("worktreeOnDelete", "Delete unused worktrees with deleted threads"), ("worktreeOnMerge", "Delete merged worktrees"), ("worktreeUnchanged", "Delete unchanged worktrees")].into_iter().map(|(key, label)| {
                        Switch::new(gpui_kit::SharedString::from(format!("storage-{key}"))).label(label).checked(storage[key] == true).small().disabled(disabled || !storage_supported)
                            .on_click(cx.listener(move |app, value: &bool, _, cx| app.update_scoped_default(None, "storageCleanup", json!({key: value}), cx)))
                    }))
                    .children([("worktreeAfterDays", "Inactive worktrees"), ("browserArtifactsAfterDays", "Browser artifacts"), ("logsAfterDays", "Logs")].into_iter().map(|(key, label)| {
                        div().flex().flex_wrap().gap_1().child(format!("{label}: {} days (null = keep)", storage[key]))
                            .children([None, Some(7), Some(30), Some(90)].into_iter().enumerate().map(|(index, days)| {
                                Button::new(gpui_kit::SharedString::from(format!("storage-{key}-{index}"))).debug_selector(move || format!("storage-{key}-{index}")).label(days.map_or("Keep".into(), |days| format!("{days} days"))).ghost().small().disabled(disabled || !storage_supported)
                                    .on_click(cx.listener(move |app, _, _, cx| app.update_scoped_default(None, "storageCleanup", json!({key: days}), cx)))
                            }))
                    })))
            })
            .when(diagnostics && owned, |view| view
                .when_some(self.scoped_pages.processes.as_ref(), |view, data| view.child(format!("Read at {} · Server PID {} · {} child processes · {} bytes RSS · {}% CPU", data["readAt"], data["serverPid"], data["processCount"], data["totalRssBytes"], data["totalCpuPercent"]))
                    .children(data["processes"].as_array().into_iter().flatten().take(100).map(|p| div().text_sm().child(format!("PID {} · {} · {} bytes · {}", p["pid"], p["status"], p["rssBytes"], p["command"]))))
                    .child(format!("Query error: {}", data["error"])))
                .when_some(self.scoped_pages.traces.as_ref(), |view, data| view.child(format!("Traces read at {} · {} records · {} failures · {} parse errors · partial {}", data["readAt"], data["recordCount"], data["failureCount"], data["parseErrorCount"], data["partialFailure"]))
                    .children(data["latestWarningAndErrorLogs"].as_array().into_iter().flatten().take(50).map(|entry| div().text_sm().child(entry.to_string())))
                    .child(format!("Query error: {}", data["error"]))))
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn r14_f_scope_reset_preserves_other_projects_and_fields() {
        let settings = json!({"projectSettingsOverrides":{"one":{"defaultRuntimeMode":"full-access","defaultThreadEnvMode":"local"},"two":{"defaultThreadEnvMode":"worktree"}}});
        let patch = scoped_patch(&settings, Some("one"), "defaultRuntimeMode", Value::Null);
        assert_eq!(
            patch["projectSettingsOverrides"]["one"],
            json!({"defaultThreadEnvMode":"local"})
        );
        assert_eq!(
            patch["projectSettingsOverrides"]["two"],
            settings["projectSettingsOverrides"]["two"]
        );
        assert_eq!(
            scoped_patch(&settings, None, "defaultThreadEnvMode", json!("worktree")),
            json!({"defaultThreadEnvMode":"worktree"})
        );
    }
}
