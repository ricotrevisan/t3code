//! Device-local preference getters shared by lifecycle and conversation callers.
use crate::{
    NativeApp,
    settings_appearance::{self, TimestampFormat},
};
use gpui_kit::component::{
    ActiveTheme, Sizable,
    button::{Button, ButtonVariants},
    radio::{Radio, RadioGroup},
    switch::Switch,
};
use gpui_kit::{App, Context, IntoElement, Pixels, div, prelude::*, px};

impl NativeApp {
    pub(crate) fn confirmation_required(&self, kind: &str, cx: &App) -> bool {
        let p = settings_appearance::current(cx);
        match kind {
            "thread.archive" => p.confirm_archive,
            "thread.delete" | "discard" => p.confirm_delete,
            "thread.unpin" => p.confirm_unpin,
            _ => false,
        }
    }
    pub(crate) fn timestamp_format(&self, cx: &App) -> TimestampFormat {
        settings_appearance::current(cx).timestamp_format
    }
    pub(crate) fn prose_font_size(&self, cx: &App) -> Pixels {
        let p = settings_appearance::current(cx);
        px(p.prose_size * 1.1_f32.powi(p.zoom.into()))
    }
    pub(crate) fn code_font_size(&self, cx: &App) -> Pixels {
        let p = settings_appearance::current(cx);
        px(p.code_size * 1.1_f32.powi(p.zoom.into()))
    }
    pub(crate) fn render_general_preferences(
        &self,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        div()
            .flex()
            .flex_col()
            .gap_2()
            .child(div().text_sm().child("Confirmations · device-local"))
            .children(
                [
                    ("thread.archive", "Confirm archive"),
                    ("thread.delete", "Confirm delete"),
                    ("thread.unpin", "Confirm unpin"),
                ]
                .into_iter()
                .enumerate()
                .map(|(index, (kind, label))| {
                    div()
                        .debug_selector(move || format!("settings-confirmation-{index}"))
                        .child(
                            Switch::new(("settings-confirmation", index))
                                .label(label)
                                .small()
                                .checked(self.confirmation_required(kind, cx))
                                .on_click(cx.listener(move |app, enabled: &bool, window, cx| {
                                    if let Err(error) =
                                        crate::theme::update(window, cx, |p| match kind {
                                            "thread.archive" => p.confirm_archive = *enabled,
                                            "thread.delete" => p.confirm_delete = *enabled,
                                            _ => p.confirm_unpin = *enabled,
                                        })
                                    {
                                        app.report_error(
                                            format!("Could not save confirmations: {error}"),
                                            cx,
                                        );
                                    }
                                    cx.notify();
                                })),
                        )
                }),
            )
            .child(div().text_sm().child("Timestamp format"))
            .child(
                RadioGroup::horizontal("settings-timestamp")
                    .selected_index(
                        TimestampFormat::ALL
                            .iter()
                            .position(|f| *f == self.timestamp_format(cx)),
                    )
                    .children(TimestampFormat::ALL.into_iter().enumerate().map(
                        |(index, format)| {
                            Radio::new(("timestamp-format", index)).label(format.label())
                        },
                    ))
                    .on_click(cx.listener(|app, index: &usize, window, cx| {
                        if let Err(error) = crate::theme::update(window, cx, |p| {
                            p.timestamp_format = TimestampFormat::ALL[*index]
                        }) {
                            app.report_error(
                                format!("Could not save timestamp format: {error}"),
                                cx,
                            );
                        }
                        cx.notify();
                    })),
            )
    }
    pub(crate) fn render_typography_preferences(
        &self,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let p = settings_appearance::current(cx);
        div()
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div()
                    .text_sm()
                    .child("Fallback theme family · used after resetting library selections"),
            )
            .child(
                RadioGroup::horizontal("settings-theme-variant")
                    .selected_index(
                        settings_appearance::Variant::ALL
                            .iter()
                            .position(|v| *v == p.variant),
                    )
                    .children(
                        settings_appearance::Variant::ALL
                            .into_iter()
                            .enumerate()
                            .map(|(index, variant)| {
                                Radio::new(("theme-variant", index)).label(variant.label())
                            }),
                    )
                    .on_click(cx.listener(|app, index: &usize, window, cx| {
                        if let Err(error) = crate::theme::update(window, cx, |p| {
                            p.variant = settings_appearance::Variant::ALL[*index]
                        }) {
                            app.report_error(format!("Could not save theme variant: {error}"), cx);
                        }
                        cx.notify();
                    })),
            )
            .children(
                [(false, "Prose", p.prose_size), (true, "Code", p.code_size)]
                    .into_iter()
                    .enumerate()
                    .map(|(index, (code, label, value))| {
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(div().text_sm().child(format!("{label} font: {value} px")))
                            .children(
                                [(-1., "Smaller"), (1., "Larger")]
                                    .into_iter()
                                    .enumerate()
                                    .map(|(step, (delta, text))| {
                                        Button::new((
                                            if code {
                                                "settings-code-size"
                                            } else {
                                                "settings-prose-size"
                                            },
                                            step,
                                        ))
                                        .label(text)
                                        .ghost()
                                        .small()
                                        .on_click(
                                            cx.listener(move |app, _, window, cx| {
                                                if let Err(error) =
                                                    crate::theme::update(window, cx, |p| {
                                                        if code {
                                                            p.code_size += delta;
                                                        } else {
                                                            p.prose_size += delta;
                                                        }
                                                    })
                                                {
                                                    app.report_error(
                                                        format!(
                                                            "Could not save typography: {error}"
                                                        ),
                                                        cx,
                                                    );
                                                }
                                                cx.notify();
                                            }),
                                        )
                                    }),
                            )
                            .child(
                                Button::new(("settings-font-reset", index))
                                    .label("Reset")
                                    .ghost()
                                    .small()
                                    .on_click(cx.listener(move |app, _, window, cx| {
                                        if let Err(error) = crate::theme::update(window, cx, |p| {
                                            if code {
                                                p.code_size = 13.;
                                            } else {
                                                p.prose_size = 14.;
                                            }
                                        }) {
                                            app.report_error(
                                                format!("Could not reset typography: {error}"),
                                                cx,
                                            );
                                        }
                                        cx.notify();
                                    })),
                            )
                    }),
            )
            .child(
                div()
                    .text_size(self.prose_font_size(cx))
                    .child("Prose preview — readable at your chosen scale."),
            )
            .child(
                div()
                    .font_family(cx.theme().mono_font_family.clone())
                    .text_size(self.code_font_size(cx))
                    .child("let greeting = \"Hello, world\";"),
            )
    }
}
