use gpui_kit::component::{
    ActiveTheme as _, Disableable, Sizable,
    button::{Button, ButtonVariants as _},
    input::Textarea,
};
use gpui_kit::{Context, div, prelude::*, px};
use serde_json::json;
use t3code_desktop_gpui::palette;

use crate::NativeApp;

impl NativeApp {
    pub(crate) fn render_composer(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let muted_foreground = cx.theme().muted_foreground;
        let border = cx.theme().border;
        let muted = cx.theme().muted;
        let popover = cx.theme().popover;
        let (runtime_mode, interaction_mode) = self.selected_modes();
        let full_access = runtime_mode == "full-access";
        let plan_mode = interaction_mode == "plan";
        let turn_running = self.turn_running();
        let active_model = self.active_model_label(cx);
        let composer_value = self.composer.read(cx).value().to_string();
        let slash_entries = composer_value
            .trim()
            .strip_prefix('/')
            .map(|rest| palette::matching(rest.trim(), self.selected_thread.is_some()));
        div()
            .flex()
            .flex_col()
            .gap_2()
            .flex_shrink_0()
            .p_3()
            .border_t_1()
            .border_color(border)
            .child(Textarea::new(&self.composer))
            .when(!self.attachments.is_empty(), |view| {
                view.child(
                    div().flex().flex_wrap().gap_1().children(
                        self.attachments
                            .iter()
                            .enumerate()
                            .map(|(index, attachment)| {
                                let id = attachment["id"].as_str().unwrap_or_default().to_owned();
                                let name = attachment["name"]
                                    .as_str()
                                    .unwrap_or("attachment")
                                    .to_owned();
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_1()
                                    .px_2()
                                    .rounded_sm()
                                    .bg(muted)
                                    .text_xs()
                                    .text_color(muted_foreground)
                                    .child(name)
                                    .child(
                                        Button::new(gpui_kit::SharedString::from(format!(
                                            "detach-{index}"
                                        )))
                                        .debug_selector(move || format!("detach-{index}"))
                                        .label("×")
                                        .ghost()
                                        .small()
                                        .on_click(
                                            cx.listener(move |app, _, _, cx| {
                                                app.attachments.retain(|attachment| {
                                                    attachment["id"].as_str() != Some(&id)
                                                });
                                                app.perform(
                                                    "attachments.delete",
                                                    json!({"attachmentId": id}),
                                                    None,
                                                    cx,
                                                );
                                                cx.notify();
                                            }),
                                        ),
                                    )
                            }),
                    ),
                )
            })
            .when_some(slash_entries, |view, entries| {
                view.child(
                    div()
                        .id("slash-commands")
                        .flex()
                        .flex_col()
                        .gap_1()
                        .max_h(px(200.))
                        .overflow_y_scroll()
                        .p_2()
                        .rounded_md()
                        .bg(popover)
                        .border_1()
                        .border_color(border)
                        .when(entries.is_empty(), |view| {
                            view.child(
                                div()
                                    .text_xs()
                                    .text_color(muted_foreground)
                                    .child("No matching commands"),
                            )
                        })
                        .children(entries.into_iter().take(8).map(|entry| {
                            let command = entry.command;
                            let label = entry.label;
                            Button::new(gpui_kit::SharedString::from(format!("slash-{label}")))
                                .debug_selector(move || format!("slash-{label}"))
                                .label(label)
                                .ghost()
                                .small()
                                .on_click(cx.listener(move |app, _, window, cx| {
                                    app.composer
                                        .update(cx, |input, cx| input.set_value("", window, cx));
                                    app.execute_command(command, window, cx);
                                }))
                        })),
                )
            })
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap_2()
                    .child(
                        Button::new("attach")
                            .debug_selector(|| "attach".into())
                            .icon(gpui_kit::assets::IconName::Paperclip)
                            .label("Attach")
                            .ghost()
                            .small()
                            .tooltip("Attach files to the prompt")
                            .on_click(cx.listener(|app, _, _, cx| app.attach_files(cx))),
                    )
                    .child(
                        Button::new("runtime-mode")
                            .debug_selector(|| "runtime-mode".into())
                            .label(if full_access {
                                "Full access"
                            } else {
                                "Approval required"
                            })
                            .ghost()
                            .small()
                            .tooltip("Toggle whether this thread may run tools without asking")
                            .on_click(cx.listener(move |app, _, _, cx| {
                                app.change_thread_mode(
                                    "thread.runtime-mode.set",
                                    "runtimeMode",
                                    if full_access {
                                        "approval-required"
                                    } else {
                                        "full-access"
                                    },
                                    cx,
                                )
                            })),
                    )
                    .child(
                        Button::new("interaction-mode")
                            .debug_selector(|| "interaction-mode".into())
                            .label(if plan_mode { "Plan" } else { "Default" })
                            .ghost()
                            .small()
                            .tooltip("Toggle plan-first interaction for this thread")
                            .on_click(cx.listener(move |app, _, _, cx| {
                                app.change_thread_mode(
                                    "thread.interaction-mode.set",
                                    "interactionMode",
                                    if plan_mode { "default" } else { "plan" },
                                    cx,
                                )
                            })),
                    )
                    .child(
                        Button::new("model-catalog")
                            .debug_selector(|| "model-catalog".into())
                            .label(if active_model.is_empty() {
                                "Select model".to_owned()
                            } else {
                                active_model.clone()
                            })
                            .ghost()
                            .small()
                            .tooltip("Choose the provider and model")
                            .on_click(cx.listener(|app, _, _, cx| {
                                app.catalog_open = !app.catalog_open;
                                cx.notify();
                            })),
                    )
                    .child(div().flex_1())
                    // One slot: Send while idle, Stop while a turn runs.
                    .child(if turn_running {
                        Button::new("stop-turn")
                            .debug_selector(|| "stop-turn".into())
                            .label("Stop")
                            .danger()
                            .outline()
                            .tooltip("Interrupt the running turn")
                            .on_click(cx.listener(|app, _, _, cx| {
                                app.lifecycle("thread.turn.interrupt", cx)
                            }))
                    } else {
                        Button::new("send")
                            .debug_selector(|| "send".into())
                            .icon(gpui_kit::assets::IconName::Send)
                            .label("Send")
                            .primary()
                            .loading(self.pending_actions > 0)
                            .disabled(
                                self.selected_thread.is_none()
                                    || self.composer.read(cx).value().trim().is_empty(),
                            )
                            .tooltip("Send the prompt (↩)")
                            .on_click(cx.listener(|app, _, window, cx| app.send_prompt(window, cx)))
                    })
                    .child(
                        Button::new("stash")
                            .debug_selector(|| "stash".into())
                            .label("Stash")
                            .ghost()
                            .on_click(cx.listener(|app, _, window, cx| {
                                app.stash_prompt(false, window, cx)
                            })),
                    )
                    .child(
                        Button::new("restore-stash")
                            .debug_selector(|| "restore-stash".into())
                            .label("Restore")
                            .ghost()
                            .on_click(
                                cx.listener(|app, _, window, cx| {
                                    app.stash_prompt(true, window, cx)
                                }),
                            ),
                    ),
            )
    }
}
