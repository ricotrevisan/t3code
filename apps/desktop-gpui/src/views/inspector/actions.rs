use gpui_kit::component::{
    ActiveTheme as _, Disableable, Sizable,
    button::{Button, ButtonVariants as _},
    input::Input,
    radio::{Radio, RadioGroup},
    switch::Switch,
};
use gpui_kit::{Context, div, prelude::*};
use serde_json::json;
use t3code_desktop_gpui::project_actions;

use super::field;
use crate::NativeApp;

impl NativeApp {
    pub(super) fn render_project_actions(
        &self,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let mono_font = cx.theme().mono_font_family.clone();
        let scripts = self
            .selected_project
            .as_ref()
            .and_then(|id| {
                self.project_settings.as_ref().map(|settings| {
                    project_actions::resolve(
                        settings,
                        id,
                        self.shell
                            .projects
                            .iter()
                            .find(|p| &p.id == id)
                            .map(|p| p.scripts.as_slice())
                            .unwrap_or_default(),
                    )
                })
            })
            .unwrap_or_default();
        div()
            .flex()
            .flex_col()
            .gap_2()
            .min_w_0()
            .child("Actions for the selected project on this environment")
            .child("Run uses the saved action and selected thread checkout, not the forms below.")
            .children(self.action_runs.iter().map(|run| {
                let close_key = run.key.clone();
                let stop_key = run.key.clone();
                let retry_key = run.key.clone();
                let dismiss_key = run.key.clone();
                let connected = self.connection_ready() && run.scope == self.environment_scope;
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .min_w_0()
                    .whitespace_normal()
                    .child(format!(
                        "{} · {} · {}",
                        run.script["name"].as_str().unwrap_or("Action"),
                        run.status,
                        run.cwd
                    ))
                    .child(format!(
                        "{} · project {} · thread {}",
                        run.scope, run.project, run.thread
                    ))
                    .child(run.error.clone().unwrap_or_default())
                    .child(
                        div()
                            .id(gpui_kit::SharedString::from(format!("{}-output", run.key)))
                            .max_h_64()
                            .overflow_y_scroll()
                            .font_family(mono_font.clone())
                            .child(if run.output.is_empty() {
                                "Waiting for terminal output".to_owned()
                            } else {
                                run.output.clone()
                            }),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .gap_2()
                            .child(
                                Button::new(gpui_kit::SharedString::from(format!(
                                    "{}-stop",
                                    run.key
                                )))
                                .label("Stop")
                                .ghost()
                                .disabled(!connected || run.busy || !run.terminal_exists)
                                .on_click(cx.listener(
                                    move |app, _, _, cx| {
                                        app.control_action_run(&stop_key, false, cx)
                                    },
                                )),
                            )
                            .child(
                                Button::new(gpui_kit::SharedString::from(format!(
                                    "{}-close",
                                    run.key
                                )))
                                .label("Close terminal")
                                .danger()
                                .outline()
                                .disabled(!connected || run.busy || !run.terminal_exists)
                                .on_click(cx.listener(
                                    move |app, _, _, cx| {
                                        app.control_action_run(&close_key, true, cx)
                                    },
                                )),
                            )
                            .child(
                                Button::new(gpui_kit::SharedString::from(format!(
                                    "{}-retry",
                                    run.key
                                )))
                                .label("Retry captured action")
                                .ghost()
                                .disabled(!connected || run.busy || run.terminal_exists)
                                .on_click(cx.listener(
                                    move |app, _, _, cx| app.retry_action_run(&retry_key, cx),
                                )),
                            )
                            .child(
                                Button::new(gpui_kit::SharedString::from(format!(
                                    "{}-dismiss",
                                    run.key
                                )))
                                .label("Dismiss output")
                                .ghost()
                                .disabled(run.busy || run.terminal_exists)
                                .on_click(cx.listener(
                                    move |app, _, _, cx| {
                                        app.action_runs.retain(|row| {
                                            row.key != dismiss_key
                                                || row.busy
                                                || row.terminal_exists
                                        });
                                        cx.notify();
                                    },
                                )),
                            ),
                    )
            }))
            .child(field("Action name", Input::new(&self.action_name), cx))
            .child(field("Command", Input::new(&self.action_command), cx))
            .child(div().flex().flex_col().gap_2().items_start()
                .child(Switch::new("action-setup").label("Run on worktree creation").small().checked(self.action_setup)
                    .on_click(cx.listener(|app, enabled: &bool, _, cx| { app.action_setup = *enabled; cx.notify(); })))
                .child(Switch::new("action-wait").label("Wait before starting agent").small().checked(self.action_wait)
                    .disabled(!self.action_setup)
                    .on_click(cx.listener(|app, enabled: &bool, _, cx| { app.action_wait = *enabled; cx.notify(); }))))
            .child(RadioGroup::horizontal("action-icon")
                .selected_index(crate::settings_project_actions::ICONS.iter().position(|icon| *icon == self.action_extras.icon))
                .children(crate::settings_project_actions::ICONS.into_iter().enumerate().map(|(index, icon)| Radio::new(("action-icon-choice", index)).label(icon)))
                .on_click(cx.listener(|app, index: &usize, _, cx| { app.action_extras.icon = crate::settings_project_actions::ICONS[*index].into(); cx.notify(); })))
            .child(Button::new("actions-read-t3-json").debug_selector(|| "actions-read-t3-json".into()).label("Read actions from t3.json").ghost().small()
                .on_click(cx.listener(|app, _, _, cx| app.load_checked_in_actions(cx))))
            .when(self.action_extras.target.as_ref().is_some_and(|(generation, project)| *generation == self.generation && Some(project) == self.selected_project.as_ref()), |view| {
                view.when(self.action_extras.loading, |view| view.child("Reading t3.json…"))
                    .when_some(self.action_extras.error.clone(), |view, error| view.child(div().text_xs().text_color(cx.theme().danger).child(error)))
                    .children(self.action_extras.scripts.iter().filter(|script| !scripts.iter().any(|saved| saved["command"] == script["command"] || saved["name"].as_str().unwrap_or_default().eq_ignore_ascii_case(script["name"].as_str().unwrap_or_default()))).enumerate().map(|(index, script)| {
                        let script = script.clone();
                        Button::new(("action-import", index)).debug_selector(move || format!("action-import-{index}")).label(format!("Import {}", script["name"].as_str().unwrap_or("Action"))).ghost().small().disabled(self.action_busy || !self.connection_ready())
                            .on_click(cx.listener(move |app, _, _, cx| {
                                app.project_actions(Some(json!({"operation":"save","id":uuid::Uuid::new_v4().to_string(),"name":script["name"],"command":script["command"],"setup":script["runOnWorktreeCreate"] == true,"wait":script["runOnWorktreeCreate"] == true && script["async"] == false,"icon":script["icon"].as_str().unwrap_or("play"),"extra":script})), cx);
                            }))
                    }))
            })
            .child(div().flex().flex_wrap().gap_2()
            .child(
                Button::new("action-save")
                    .debug_selector(|| "action-save".into())
                    .label("Save action")
                    .primary()
                    .disabled((self.action_busy) || !self.connection_ready())
                    .on_click(cx.listener(|app, _, _, cx| app.save_project_action(cx))),
            )
            .child(
                Button::new("action-new")
                    .label("New action")
                    .secondary()
                    .on_click(cx.listener(|app, _, window, cx| {
                        app.action_edit_id = None;
                        app.confirm_action_delete = None;
                        app.action_setup = false;
                        app.action_wait = false;
                        app.action_extras.icon = "play".into();
                        app.action_name
                            .update(cx, |input, cx| input.set_value("", window, cx));
                        app.action_command
                            .update(cx, |input, cx| input.set_value("", window, cx));
                        cx.notify();
                    })),
            )
            )
            .children(scripts.into_iter().enumerate().map(|(index, script)| {
                let edit = script.clone();
                let run_id = script["id"].as_str().unwrap_or_default().to_owned();
                let id = script["id"].as_str().unwrap_or_default().to_owned();
                let confirm = self.confirm_action_delete.as_ref() == Some(&id);
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .min_w_0()
                    .whitespace_normal()
                    .child(format!(
                        "{} · {}{}",
                        script["name"].as_str().unwrap_or_default(),
                        script["command"].as_str().unwrap_or_default(),
                        if script["runOnWorktreeCreate"] == true {
                            " · worktree setup"
                        } else {
                            ""
                        }
                    ))
                    .child(div().flex().flex_wrap().gap_1()
                    .child(
                        Button::new(gpui_kit::SharedString::from(format!("action-run-{index}")))
                            .debug_selector(move || format!("action-run-{index}"))
                            .label("Run").small()
                            .secondary()
                            .disabled(
                                !self.connection_ready()
                                    || self.selected_thread.is_none()
                                    || self.action_runs.iter().any(|run| {
                                        run.busy
                                            && run.generation == self.generation
                                            && run.scope == self.environment_scope
                                            && Some(&run.thread) == self.selected_thread.as_ref()
                                            && self.selected_workspace().as_deref()
                                                == Some(run.cwd.as_str())
                                            && run.script["id"] == run_id
                                    }),
                            )
                            .on_click(cx.listener(move |app, _, _, cx| {
                                app.run_project_action(&run_id, cx)
                            })),
                    )
                    .child(
                        Button::new(gpui_kit::SharedString::from(format!("action-edit-{index}")))
                            .label("Edit").small()
                            .ghost()
                            .on_click(cx.listener(move |app, _, window, cx| {
                                app.action_edit_id = edit["id"].as_str().map(str::to_owned);
                                app.action_extras.icon = edit["icon"].as_str().unwrap_or("play").into();
                                app.action_setup = edit["runOnWorktreeCreate"] == true;
                                app.action_wait = edit["async"] == false;
                                app.action_name.update(cx, |input, cx| {
                                    input.set_value(
                                        edit["name"].as_str().unwrap_or_default().to_owned(),
                                        window,
                                        cx,
                                    )
                                });
                                app.action_command.update(cx, |input, cx| {
                                    input.set_value(
                                        edit["command"].as_str().unwrap_or_default().to_owned(),
                                        window,
                                        cx,
                                    )
                                });
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new(gpui_kit::SharedString::from(format!(
                            "action-delete-{index}"
                        )))
                        .label(if confirm {
                            "Confirm delete action"
                        } else {
                            "Delete"
                        })
                        .danger()
                        .outline()
                        .disabled(self.action_busy || !self.connection_ready())
                        .on_click(cx.listener(move |app, _, _, cx| {
                            if app.confirm_action_delete.as_ref() == Some(&id) {
                                app.project_actions(
                                    Some(json!({"operation":"delete","id":id})),
                                    cx,
                                );
                            } else {
                                app.confirm_action_delete = Some(id.clone());
                                cx.notify();
                            }
                        })),
                    )
                    )
            }))
            .child(
                Button::new("project-default-save").disabled(!self.connection_ready())
                    .label("Use selected provider and model for new threads")
                    .secondary()
                    .on_click(cx.listener(|app, _, _, cx| app.update_project_default(false, cx))),
            )
            .child(
                Button::new("project-default-inherit").disabled(!self.connection_ready())
                    .label("Use environment model default")
                    .ghost()
                    .on_click(cx.listener(|app, _, _, cx| {
                        app.project_actions(Some(json!({"operation":"model-inherit"})), cx)
                    })),
            )
            .child(
                Button::new("project-default-clear").disabled(!self.connection_ready())
                    .label("Clear project model default")
                    .ghost()
                    .on_click(cx.listener(|app, _, _, cx| app.update_project_default(true, cx))),
            )
            .child(
                Button::new("actions-refresh")
                    .label("Refresh actions")
                    .ghost()
                    .disabled((self.action_busy) || !self.connection_ready())
                    .on_click(cx.listener(|app, _, _, cx| app.project_actions(None, cx))),
            )
            .child(
                Button::new("actions-inherit")
                    .label("Use environment action defaults")
                    .ghost()
                    .disabled((self.action_busy) || !self.connection_ready())
                    .on_click(cx.listener(|app, _, _, cx| {
                        app.project_actions(Some(json!({"operation":"inherit"})), cx)
                    })),
            )
    }
}
