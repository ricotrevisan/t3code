use gpui_kit::component::{
    button::{Button, ButtonVariants as _},
    input::Input,
};
use gpui_kit::{Context, div, prelude::*};
use t3code_desktop_gpui::palette;

use super::field;
use crate::NativeApp;

impl NativeApp {
    pub(super) fn render_project_section(
        &self,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        div()
            .flex()
            .flex_col()
            .gap_2()
            .child(field(
                "Project or thread title",
                Input::new(&self.title),
                cx,
            ))
            .child(field("Project directory", Input::new(&self.workspace), cx))
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap_2()
                    .child(
                        Button::new("add-project")
                            .label("Add project")
                            .primary()
                            .on_click(cx.listener(|app, _, window, cx| {
                                app.execute_command(palette::Command::AddProject, window, cx)
                            })),
                    )
                    .child(
                        Button::new("new-thread")
                            .label("New thread")
                            .secondary()
                            .on_click(cx.listener(|app, _, window, cx| {
                                app.execute_command(palette::Command::NewThread, window, cx)
                            })),
                    )
                    .child(
                        Button::new("rename-project")
                            .debug_selector(|| "rename-project".into())
                            .label("Rename project")
                            .ghost()
                            .on_click(cx.listener(|app, _, _, cx| app.update_project(false, cx))),
                    )
                    .child(
                        Button::new("remove-project")
                            .debug_selector(|| "remove-project".into())
                            .label("Remove project")
                            .danger()
                            .outline()
                            .on_click(cx.listener(|app, _, _, cx| app.update_project(true, cx))),
                    ),
            )
    }
}
