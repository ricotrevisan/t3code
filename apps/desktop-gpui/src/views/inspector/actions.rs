use gpui_kit::base::Selectable;
use gpui_kit::component::{
    Disableable,
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
