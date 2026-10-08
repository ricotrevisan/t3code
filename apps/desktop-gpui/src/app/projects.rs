//! Project records, project defaults, project actions, and cloning.

use crate::NativeApp;
use gpui_kit::Context;
use serde_json::{Value, json};
use t3code_desktop_gpui::{commands, connection, project_actions, workspaces};

impl NativeApp {
    pub(crate) fn clone_project(&mut self, cx: &mut Context<Self>) {
        if self.pending_actions > 0 {
            self.report_progress("Wait for the pending server receipt", cx);
            cx.notify();
            return;
        }
        match workspaces::clone_request(
            self.title.read(cx).value().as_str(),
            self.clone_remote.read(cx).value().as_str(),
            self.workspace.read(cx).value().as_str(),
        ) {
            Ok(payload) => self.perform("projectClone.start", payload, None, cx),
            Err(error) => {
                self.report_error(error, cx);
                cx.notify();
            }
        }
    }

    pub(crate) fn update_project_default(&mut self, clear: bool, cx: &mut Context<Self>) {
        let selection = if clear {
            None
        } else {
            let previous = self
                .selected_thread
                .as_ref()
                .and_then(|id| self.shell.threads.iter().find(|thread| &thread.id == id))
                .map(|thread| &thread.model_selection)
                .unwrap_or(&Value::Null);
            match self.model_selection(previous, cx) {
                Ok(selection) => Some(selection),
                Err(error) => {
                    self.report_error(error, cx);
                    cx.notify();
                    return;
                }
            }
        };
        self.project_actions(
            Some(json!({"operation":"model-default","selection":selection})),
            cx,
        );
    }

    pub(crate) fn update_project(&mut self, remove: bool, cx: &mut Context<Self>) {
        let Some(id) = self.selected_project.clone() else {
            self.report_error("Select a project first", cx);
            cx.notify();
            return;
        };
        if remove && self.confirm_project_removal.as_ref() != Some(&id) {
            self.confirm_project_removal = Some(id);
            self.set_status("Remove project again to confirm. Project threads are removed; workspace files remain untouched.", cx);
            cx.notify();
            return;
        }
        let result = if remove {
            commands::remove_project(&id)
        } else {
            commands::rename_project(&id, &self.title.read(cx).value())
        };
        match result {
            Ok(command) => {
                self.confirm_project_removal = None;
                self.dispatch(command, cx);
            }
            Err(error) => {
                self.report_error(error, cx);
                cx.notify();
            }
        }
    }

    pub(crate) fn project_actions(&mut self, mutation: Option<Value>, cx: &mut Context<Self>) {
        if self.action_busy {
            self.report_progress("Another action change is pending", cx);
            cx.notify();
            return;
        }
        let Some(project) = self.selected_project.clone() else {
            self.report_error("Select a project first", cx);
            cx.notify();
            return;
        };
        let Some(sender) = self.commands.clone() else {
            self.report_error("Not connected", cx);
            cx.notify();
            return;
        };
        let legacy = self
            .shell
            .projects
            .iter()
            .find(|p| p.id == project)
            .map(|p| p.scripts.clone())
            .unwrap_or_default();
        let generation = self.generation;
        self.action_busy = true;
        self.report_progress("Reading project actions…", cx);
        self.action_task = Some(cx.spawn(async move |entity, cx| {
            let saved_change = mutation.clone();
            let result = async {
                let settings = connection::call(&sender, "server.getSettings", json!({})).await?;
                let Some(change) = mutation else {
                    return Ok(settings);
                };
                if change["operation"] == "model-default" || change["operation"] == "model-inherit"
                {
                    return connection::call(
                        &sender,
                        "server.updateSettings",
                        project_actions::model_patch(
                            &settings,
                            &project,
                            if change["operation"] == "model-inherit" {
                                None
                            } else {
                                Some(change["selection"].clone())
                            },
                        ),
                    )
                    .await;
                }
                let current = project_actions::resolve(&settings, &project, &legacy);
                if change["extra"].is_object()
                    && current.iter().any(|script| {
                        script["command"] == change["command"]
                            || script["name"]
                                .as_str()
                                .unwrap_or_default()
                                .eq_ignore_ascii_case(change["name"].as_str().unwrap_or_default())
                    })
                {
                    return Err("Action already exists; edit the saved action instead".into());
                }
                let scripts = match change["operation"].as_str() {
                    Some("inherit") => None,
                    Some("delete") => Some(
                        current
                            .into_iter()
                            .filter(|s| s["id"] != change["id"])
                            .collect(),
                    ),
                    Some("save") => {
                        let mut saved = project_actions::save(
                            &current,
                            change["previousId"].as_str(),
                            change["id"].as_str().unwrap_or_default(),
                            change["name"].as_str().unwrap_or_default(),
                            change["command"].as_str().unwrap_or_default(),
                            change["setup"] == true,
                            change["wait"] == true,
                        )
                        .map_err(str::to_owned)?;
                        if let Some(icon) = change["icon"].as_str() {
                            if !crate::settings_project_actions::ICONS.contains(&icon) {
                                return Err("Choose a supported action icon".into());
                            }
                            if let Some(script) =
                                saved.iter_mut().find(|script| script["id"] == change["id"])
                            {
                                script["icon"] = icon.into();
                                if let Some(extra) = change["extra"].as_object() {
                                    if let Some(value) = extra.get("previewUrl") {
                                        script["previewUrl"] = value.clone();
                                    }
                                    script["autoOpenPreview"] =
                                        (extra.get("previewUrl").is_some_and(Value::is_string)
                                            && extra.get("autoOpenPreview")
                                                == Some(&Value::Bool(true)))
                                        .into();
                                }
                            }
                        }
                        Some(saved)
                    }
                    _ => return Err("Unknown project action operation".into()),
                };
                connection::call(
                    &sender,
                    "server.updateSettings",
                    project_actions::patch(&settings, &project, scripts),
                )
                .await
            }
            .await;
            let _ = entity.update(cx, |app, cx| {
                if app.generation != generation || app.selected_project.as_ref() != Some(&project) {
                    return;
                }
                app.action_busy = false;
                match result {
                    Ok(settings) => {
                        if let Some(change) = &saved_change {
                            if change["operation"] == "save"
                                && app.action_name.read(cx).value().as_str()
                                    == change["name"].as_str().unwrap_or_default()
                                && app.action_command.read(cx).value().as_str()
                                    == change["command"].as_str().unwrap_or_default()
                            {
                                app.action_edit_id = change["id"].as_str().map(str::to_owned);
                            } else if change["operation"] == "delete"
                                && app.action_edit_id.as_deref() == change["id"].as_str()
                            {
                                app.action_edit_id = None;
                            }
                        }
                        app.project_settings = Some(settings);
                        app.confirm_action_delete = None;
                        app.set_status("Project actions loaded from this environment", cx);
                    }
                    Err(error) => app.report_error(error, cx),
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    pub(crate) fn save_project_action(&mut self, cx: &mut Context<Self>) {
        let name = self.action_name.read(cx).value().to_string();
        let command = self.action_command.read(cx).value().to_string();
        if name.trim().is_empty() || command.trim().is_empty() {
            self.report_error("Action name and command are required", cx);
            cx.notify();
            return;
        }
        self.project_actions(Some(json!({"operation":"save","previousId":self.action_edit_id,"id":self.action_edit_id.clone().unwrap_or_else(||uuid::Uuid::new_v4().to_string()),"name":name,"command":command,"setup":self.action_setup,"wait":self.action_wait,"icon":self.action_extras.icon})),cx);
    }
}
