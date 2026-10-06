use gpui_kit::component::{
    button::{Button, ButtonVariants as _},
    input::Input,
};
use gpui_kit::{Context, div, prelude::*};

use super::field;
use crate::NativeApp;

impl NativeApp {
    pub(super) fn render_thread_section(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        div()
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div()
                    .flex()
                    .items_end()
                    .gap_2()
                    .child(field("Snooze until", Input::new(&self.custom_snooze), cx))
                    .child(
                        Button::new("custom-snooze")
                            .label("Snooze")
                            .secondary()
                            .on_click(cx.listener(|app, _, _, cx| app.snooze_custom(cx))),
                    ),
            )
            .child(
                div()
                    .flex()
                    .gap_2()
                    .child(field("Provider instance", Input::new(&self.provider), cx))
                    .child(field("Model", Input::new(&self.model), cx)),
            )
    }
}
