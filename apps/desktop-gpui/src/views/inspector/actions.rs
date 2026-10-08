use gpui_kit::base::Selectable;
use gpui_kit::component::{
    ActiveTheme as _, Disableable,
    button::{Button, ButtonVariants as _},
    input::Input,
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
                let connected = self.commands.is_some() && run.scope == self.environment_scope;
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
            .child(
                Button::new("action-setup")
                    .ghost()
                    .selected(self.action_setup)
                    .label(if self.action_setup {
                        "Runs on worktree creation"
                    } else {
                        "Run manually"
                    })
                    .on_click(cx.listener(|app, _, _, cx| {
                        app.action_setup = !app.action_setup;
                        cx.notify();
                    })),
            )
            .child(
                Button::new("action-wait")
                    .ghost()
                    .selected(self.action_wait)
                    .label(if self.action_wait {
                        "Wait before starting agent"
                    } else {
                        "Start agent while setup runs"
                    })
                    .on_click(cx.listener(|app, _, _, cx| {
                        app.action_wait = !app.action_wait;
                        cx.notify();
                    })),
            )
            .child(
                Button::new("action-save")
                    .debug_selector(|| "action-save".into())
                    .label("Save action")
                    .primary()
                    .disabled(self.action_busy)
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
                        app.action_name
                            .update(cx, |input, cx| input.set_value("", window, cx));
                        app.action_command
                            .update(cx, |input, cx| input.set_value("", window, cx));
                        cx.notify();
                    })),
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
                    .child(
                        Button::new(gpui_kit::SharedString::from(format!("action-run-{index}")))
                            .debug_selector(move || format!("action-run-{index}"))
                            .label("Run action")
                            .secondary()
                            .disabled(
                                self.commands.is_none()
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
                            .label("Edit action")
                            .ghost()
                            .on_click(cx.listener(move |app, _, window, cx| {
                                app.action_edit_id = edit["id"].as_str().map(str::to_owned);
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
                            "Delete action"
                        })
                        .danger()
                        .outline()
                        .disabled(self.action_busy)
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
            }))
            .child(
                Button::new("project-default-save")
                    .label("Use selected provider and model for new threads")
                    .secondary()
                    .on_click(cx.listener(|app, _, _, cx| app.update_project_default(false, cx))),
            )
            .child(
                Button::new("project-default-inherit")
                    .label("Use environment model default")
                    .ghost()
                    .on_click(cx.listener(|app, _, _, cx| {
                        app.project_actions(Some(json!({"operation":"model-inherit"})), cx)
                    })),
            )
            .child(
                Button::new("project-default-clear")
                    .label("Clear project model default")
                    .ghost()
                    .on_click(cx.listener(|app, _, _, cx| app.update_project_default(true, cx))),
            )
            .child(
                Button::new("actions-refresh")
                    .label("Refresh actions")
                    .ghost()
                    .disabled(self.action_busy)
                    .on_click(cx.listener(|app, _, _, cx| app.project_actions(None, cx))),
            )
            .child(
                Button::new("actions-inherit")
                    .label("Use environment action defaults")
                    .ghost()
                    .disabled(self.action_busy)
                    .on_click(cx.listener(|app, _, _, cx| {
                        app.project_actions(Some(json!({"operation":"inherit"})), cx)
                    })),
            )
    }
}
