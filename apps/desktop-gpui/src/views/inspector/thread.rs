use gpui_kit::component::{
    Sizable,
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
                            .small()
                            .secondary()
                            .on_click(cx.listener(|app, _, _, cx| app.snooze_custom(cx))),
                    ),
            )
            .child(
                div().flex().items_center().gap_2().child("Model").child(
                    Button::new("thread-model-picker")
                        .label(self.active_model_label(cx))
                        .ghost()
                        .small()
                        .on_click(
                            cx.listener(|app, _, window, cx| app.open_model_catalog(window, cx)),
                        ),
                ),
            )
    }
}
