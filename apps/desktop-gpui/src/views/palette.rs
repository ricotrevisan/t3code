use gpui_kit::base::Selectable;
use gpui_kit::component::{
    ActiveTheme as _, Sizable,
    button::{Button, ButtonVariants as _},
    input::Input,
};
use gpui_kit::{Context, div, prelude::*, px};
use t3code_desktop_gpui::palette;

use crate::NativeApp;

impl NativeApp {
    pub(crate) fn render_palette(
        &self,
        cx: &mut Context<Self>,
    ) -> Option<impl IntoElement + use<>> {
        let muted_foreground = cx.theme().muted_foreground;
        let border = cx.theme().border;
        let popover = cx.theme().popover;
        self.palette_open.then(|| {
            let entries = palette::matching(
                &self.palette_query.read(cx).value(),
                self.selected_thread.is_some(),
            );
            div()
                .id("command-palette")
                .key_context("NativePalette")
                .absolute()
                .top(px(72.))
                .left(px(320.))
                .w(px(640.))
                .max_h(px(600.))
                .overflow_y_scroll()
                .flex()
                .flex_col()
                .gap_2()
                .p_3()
                .rounded_lg()
                .bg(popover)
                .border_1()
                .border_color(border)
                .shadow_lg()
                .child(
                    div()
                        .flex()
                        .justify_between()
                        .items_center()
                        .text_sm()
                        .text_color(muted_foreground)
                        .child("Commands")
                        .child(
                            Button::new("close-palette")
                                .label("Close")
                                .ghost()
                                .small()
                                .on_click(
                                    cx.listener(|app, _, window, cx| app.close_palette(window, cx)),
                                ),
                        ),
                )
                .child(Input::new(&self.palette_query))
                .when(entries.is_empty(), |view| {
                    view.child(
                        div()
                            .text_color(muted_foreground)
                            .child("No matching commands"),
                    )
                })
                .children(entries.into_iter().enumerate().map(|(index, entry)| {
                    let command = entry.command;
                    Button::new(gpui_kit::SharedString::from(format!("palette-{index}")))
                        .debug_selector(move || format!("palette-{index}"))
                        .ghost()
                        .selected(index == self.palette_index)
                        .label(format!(
                            "{}{}",
                            if index == self.palette_index {
                                "› "
                            } else {
                                ""
                            },
                            entry.label
                        ))
                        .on_click(cx.listener(move |app, _, window, cx| {
                            app.close_palette(window, cx);
                            app.execute_command(command, window, cx);
                        }))
                }))
        })
    }
}
