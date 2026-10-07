use gpui_kit::assets::IconName;
use gpui_kit::component::{
    ActiveTheme as _, Icon, Sizable,
    button::{Button, ButtonVariants as _},
};
use gpui_kit::{Context, div, prelude::*, px};
use t3code_desktop_gpui::notifications::{self, ToastKind};

use crate::NativeApp;

impl NativeApp {
    /// The toast stack, bottom-right of the conversation column just above the
    /// status bar. Only the cards take pointer input.
    pub(crate) fn render_toasts(&self, cx: &mut Context<Self>) -> Option<impl IntoElement + use<>> {
        if self.toasts.is_empty() {
            return None;
        }
        let theme = cx.theme();
        let (popover, border, foreground) = (theme.popover, theme.border, theme.foreground);
        let cards = self.toasts.iter().map(|toast| {
            let (icon, color) = match toast.kind {
                ToastKind::Info => (IconName::Info, theme.info),
                ToastKind::Success => (IconName::CircleCheck, theme.success),
                ToastKind::Error => (IconName::CircleX, theme.danger),
            };
            let id = toast.id;
            div()
                .id(("toast", id as usize))
                .debug_selector(move || format!("toast-{id}"))
                .occlude()
                .flex()
                .items_center()
                .gap_2()
                .pl_3()
                .pr_1()
                .py_1()
                .rounded_lg()
                .bg(popover)
                .border_1()
                .border_color(border)
                .shadow_lg()
                .text_sm()
                .text_color(foreground)
                .child(Icon::new(icon).small().text_color(color))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .child(toast.message.clone()),
                )
                .when_some(toast.thread.clone(), |card, thread| {
                    card.child(
                        Button::new(("toast-open", id as usize))
                            .debug_selector(move || format!("toast-open-{id}"))
                            .label("Open thread")
                            .ghost()
                            .xsmall()
                            .on_click(cx.listener(move |app, _, window, cx| {
                                app.dismiss_toast(id, cx);
                                app.open_notified_thread(&notifications::tag(&thread), window, cx);
                            })),
                    )
                })
                .child(
                    Button::new(("toast-close", id as usize))
                        .debug_selector(move || format!("toast-close-{id}"))
                        .icon(IconName::X)
                        .ghost()
                        .xsmall()
                        .tooltip("Dismiss")
                        .on_click(cx.listener(move |app, _, _, cx| app.dismiss_toast(id, cx))),
                )
        });
        Some(
            div()
                .absolute()
                .right_3()
                .bottom(px(40.))
                .w(px(360.))
                .flex()
                .flex_col()
                .gap_2()
                .children(cards.collect::<Vec<_>>()),
        )
    }
}
