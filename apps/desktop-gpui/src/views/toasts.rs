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
        let visible: Vec<_> = self
            .toasts
            .iter()
            .filter(|t| t.id != 0)
            .take(notifications::VISIBLE_TOASTS)
            .collect();
        let hidden: Vec<_> = self
            .toasts
            .iter()
            .filter(|t| !visible.iter().any(|v| v.id == t.id))
            .map(|t| t.id)
            .collect();
        let more_errors = self
            .toasts
            .iter()
            .filter(|t| hidden.contains(&t.id) && t.kind == ToastKind::Error)
            .fold(0u64, |count, t| count.saturating_add(t.count));
        let cards = visible.into_iter().map(|toast| {
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
                        .child(if toast.count > 1 {
                            format!("{} (×{})", toast.message, toast.count)
                        } else {
                            toast.message.clone()
                        }),
                )
                .when_some(toast.thread.clone(), |card, thread| {
                    let tag = notifications::tag(&self.environment_scope, &thread);
                    card.child(
                        Button::new(("toast-open", id as usize))
                            .debug_selector(move || format!("toast-open-{id}"))
                            .label("Open thread")
                            .ghost()
                            .xsmall()
                            .on_click(cx.listener(move |app, _, window, cx| {
                                app.dismiss_toast(id, cx);
                                app.open_notified_thread(&tag, window, cx);
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
                .id("toast-stack")
                .occlude()
                .max_h(px(320.))
                .absolute()
                .right_3()
                .bottom(px(40.))
                .w(px(360.))
                .flex()
                .flex_col()
                .gap_2()
                .children(cards.collect::<Vec<_>>())
                .when(more_errors > 0, |stack| {
                    stack.child(
                        Button::new("toast-more-errors")
                            .debug_selector(|| "toast-more-errors".into())
                            .ghost()
                            .small()
                            .label(format!("{more_errors} more errors · dismiss"))
                            .on_click(cx.listener(move |app, _, _, cx| {
                                app.toasts.retain(|t| {
                                    t.kind != ToastKind::Error || !hidden.contains(&t.id)
                                });
                                cx.notify();
                            })),
                    )
                }),
        )
    }
}
