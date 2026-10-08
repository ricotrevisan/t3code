use gpui_kit::component::{ActiveTheme as _, input::Input};
use gpui_kit::{Context, div, prelude::*, rems, uniform_list};

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
            let entries = self.palette_items(cx);
            div()
                .id("command-palette")
                .debug_selector(|| "command-palette".into())
                .key_context("NativePalette")
                .occlude()
                .w(rems(640. / 14.))
                .max_w_full()
                .max_h(gpui_kit::relative(0.85))
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
                    uniform_list("palette-results", entries.len(), {
                        let entity = cx.entity().downgrade();
                        let selected = self.palette_index;
                        move |range, _, _cx| {
                            range
                                .map(|index| {
                                    let entry = &entries[index];
                                    let target = entry.target.clone();
                                    let entity = entity.clone();
                                    list_row(
                                        format!("palette-{index}"),
                                        None,
                                        entry.label.clone(),
                                        index == selected,
                                    )
                                    .h(rems(2.5))
                                    .on_click(
                                        move |_, window, cx| {
                                            let _ = entity.update(cx, |app, cx| {
                                                app.run_palette_item(target.clone(), window, cx)
                                            });
                                        },
                                    )
                                })
                                .collect::<Vec<_>>()
                        }
                    })
                    .h(rems(30.))
                    .track_scroll(&self.palette_scroll),
                )
        })
    }
}
