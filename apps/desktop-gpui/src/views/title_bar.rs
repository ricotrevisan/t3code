use gpui_kit::component::{
    ActiveTheme as _, Sizable, TitleBar,
    button::{Button, ButtonVariants as _},
};
use gpui_kit::{Context, FontWeight, div, prelude::*};

use crate::NativeApp;

impl NativeApp {
    pub(crate) fn render_title_bar(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let muted_foreground = cx.theme().muted_foreground;
        let active_thread_title = self.selected_thread_title();
        TitleBar::new()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .min_w_0()
                    .child(div().font_weight(FontWeight::SEMIBOLD).child("T3 Code"))
                    .child(
                        div()
                            .text_sm()
                            .text_color(muted_foreground)
                            .truncate()
                            .child(active_thread_title),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(
                        Button::new("commands")
                            .debug_selector(|| "commands".into())
                            .label("Commands ⌘K")
                            .ghost()
                            .small()
                            .on_click(
                                cx.listener(|app, _, window, cx| app.toggle_palette(window, cx)),
                            ),
                    )
                    .child(
                        Button::new("settings-open")
                            .debug_selector(|| "settings-open".into())
                            .icon(gpui_kit::assets::IconName::Settings)
                            .label("Settings")
                            .ghost()
                            .small()
                            .on_click(cx.listener(|app, _, _, cx| {
                                app.settings_open = true;
                                cx.notify();
                            })),
                    ),
            )
    }
}
