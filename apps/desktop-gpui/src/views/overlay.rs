use gpui_kit::component::ActiveTheme as _;
use gpui_kit::{AnyElement, Context, div, prelude::*, px};

use crate::NativeApp;

impl NativeApp {
    /// Window-wide layer under every open overlay, and the one outside-click path.
    /// A press on it closes the topmost overlay and never reaches the controls
    /// beneath, so a trigger pressed while its overlay is open closes it rather
    /// than reopening it. Overlay surfaces `occlude()` so presses inside them do
    /// not land here. Palette and Settings are centred on it; the thread menu and
    /// model catalog are deferred, so they paint above it at their triggers.
    pub(crate) fn render_overlay_layer(
        &self,
        cx: &mut Context<Self>,
    ) -> Option<impl IntoElement + use<>> {
        let modal = self.palette_open || self.settings_open;
        if !(modal || self.catalog_open || self.thread_menu_open) {
            return None;
        }
        let centred = |panel: AnyElement| {
            div()
                .absolute()
                .inset_0()
                .flex()
                .justify_center()
                .items_start()
                .pt(px(72.))
                .child(panel)
        };
        Some(
            div()
                .id("overlay-backdrop")
                .debug_selector(|| "overlay-backdrop".into())
                .absolute()
                .inset_0()
                .occlude()
                .when(modal, |layer| layer.bg(cx.theme().overlay))
                .on_any_mouse_down(cx.listener(|app, _, window, cx| {
                    app.dismiss_overlay(window, cx);
                }))
                .children(
                    self.render_settings(cx)
                        .map(|panel| centred(panel.into_any_element())),
                )
                .children(
                    self.render_palette(cx)
                        .map(|panel| centred(panel.into_any_element())),
                ),
        )
    }
}
