use gpui_kit::base::Disableable;
use gpui_kit::component::{
    button::{Button, ButtonVariants as _},
    input::Input,
};
use gpui_kit::{Context, div, prelude::*};
use serde_json::json;

use super::field;
use crate::NativeApp;

impl NativeApp {
    pub(super) fn render_clone(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        div()
            .flex()
            .flex_col()
            .gap_2()
            .child(self.render_repository_chooser(cx))
            .child(field(
                "Repository URL / owner/repository",
                Input::new(&self.clone_remote),
                cx,
            ))
            .child(field("Project title", Input::new(&self.title), cx))
            .child(field(
                "Destination directory",
                Input::new(&self.workspace),
                cx,
            ))
            .child(
                Button::new("clone-start")
                    .disabled(
                        !self.connection_ready()
                            || self.repository_chooser.provider.is_some()
                            || (self.repository_chooser.busy
                                && self.repository_chooser.generation == self.generation
                                && self.repository_chooser.environment == self.environment_scope),
                    )
                    .debug_selector(|| "clone-start".into())
                    .label("Start clone to project directory")
                    .primary()
                    .on_click(cx.listener(|app, _, _, cx| app.clone_project(cx))),
            )
            .children(self.clone_progress.iter().map(|row| {
                let cancel_id = row.project_id.clone();
                let retry_id = cancel_id.clone();
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(format!(
                        "{} · {} · {}{}",
                        row.destination_path,
                        row.phase,
                        row.stage,
                        row.percent
                            .map_or(String::new(), |percent| format!(" {percent}%"))
                    ))
                    .children(row.detail.clone())
                    .children(row.error.clone())
                    .when(row.phase == "running", |view| {
                        view.child(
                            Button::new(gpui_kit::SharedString::from(format!(
                                "clone-cancel-{cancel_id}"
                            )))
                            .label("Cancel clone")
                            .danger()
                            .outline()
                            .on_click(cx.listener(
                                move |app, _, _, cx| {
                                    app.perform(
                                        "projectClone.cancel",
                                        json!({"projectId":cancel_id}),
                                        None,
                                        cx,
                                    )
                                },
                            )),
                        )
                    })
                    .when(row.phase == "failed" || row.phase == "cancelled", |view| {
                        view.child(
                            Button::new(gpui_kit::SharedString::from(format!(
                                "clone-retry-{retry_id}"
                            )))
                            .label("Retry clone")
                            .secondary()
                            .on_click(cx.listener(
                                move |app, _, _, cx| {
                                    app.perform(
                                        "projectClone.retry",
                                        json!({"projectId":retry_id}),
                                        None,
                                        cx,
                                    )
                                },
                            )),
                        )
                    })
            }))
    }
}
