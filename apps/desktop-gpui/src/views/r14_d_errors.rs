//! Dismissals own the exact environment/thread/error, not the current row index.
use super::*;

impl NativeApp {
    pub(super) fn render_thread_error(&self, error: &str, cx: &mut Context<Self>) -> gpui_kit::Div {
        let key = (
            self.environment_scope.clone(),
            self.selected_thread.clone(),
            error.to_owned(),
        );
        if self
            .conversation_presentation
            .dismissed_errors
            .contains(&key)
        {
            return div();
        }
        let generation = self.generation;
        div()
            .debug_selector(|| "thread-error-banner".into())
            .flex()
            .flex_col()
            .gap_1()
            .p_2()
            .rounded_md()
            .border_1()
            .border_color(cx.theme().danger)
            .child(
                div()
                    .flex()
                    .items_start()
                    .gap_2()
                    .child(
                        Icon::new(IconName::CircleX)
                            .xsmall()
                            .text_color(cx.theme().danger),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_xs()
                            .text_color(cx.theme().danger)
                            .child(error.to_owned()),
                    )
                    .child(
                        Button::new("thread-error-dismiss")
                            .debug_selector(|| "thread-error-dismiss".into())
                            .ghost()
                            .xsmall()
                            .icon(IconName::Close)
                            .accessibility_label("Dismiss error")
                            .tooltip("Dismiss this error for this session")
                            .on_click(cx.listener(move |app, _, _, cx| {
                                if app.generation != generation
                                    || app.environment_scope != key.0
                                    || app.selected_thread != key.1
                                {
                                    return;
                                }
                                if !app
                                    .conversation_presentation
                                    .dismissed_errors
                                    .contains(&key)
                                {
                                    if app.conversation_presentation.dismissed_errors.len() == 256 {
                                        app.conversation_presentation.dismissed_errors.pop_front();
                                    }
                                    app.conversation_presentation
                                        .dismissed_errors
                                        .push_back(key.clone());
                                }
                                cx.notify();
                            })),
                    ),
            )
            .when(!self.connection_ready(), |view| {
                view.child(
                    div().flex().items_start().child(
                        Button::new("thread-error-reconnect")
                            .ghost()
                            .xsmall()
                            .label("Retry connection")
                            .on_click(cx.listener(|app, _, _, cx| app.retry_connection_now(cx))),
                    ),
                )
            })
    }
}
