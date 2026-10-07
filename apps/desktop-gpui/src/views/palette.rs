use gpui_kit::component::{ActiveTheme as _, input::Input};
use gpui_kit::{Context, div, prelude::*, px};
use t3code_desktop_gpui::palette;

use super::list_row;
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
                .debug_selector(|| "command-palette".into())
                .key_context("NativePalette")
                .occlude()
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
                .child(Input::new(&self.palette_query))
                .when(entries.is_empty(), |view| {
                    view.child(
                        div()
                            .px_2()
                            .text_color(muted_foreground)
                            .child("No matching commands"),
                    )
                })
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .children(entries.into_iter().enumerate().map(|(index, entry)| {
                            let command = entry.command;
                            list_row(
                                format!("palette-{index}"),
                                None,
                                entry.label,
                                index == self.palette_index,
                            )
                            .on_click(cx.listener(
                                move |app, _, window, cx| {
                                    app.close_palette(window, cx);
                                    app.execute_command(command, window, cx);
                                },
                            ))
                        })),
                )
        })
    }
}
