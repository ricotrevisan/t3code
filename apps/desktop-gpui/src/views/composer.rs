use gpui_kit::assets::IconName;
use gpui_kit::base::Selectable;
use gpui_kit::component::{
    ActiveTheme as _, Disableable, Icon, Sizable,
    button::{Button, ButtonVariants as _},
    input::Textarea,
    menu::{DropdownMenu as _, PopupMenuItem},
    progress::Progress,
};
use gpui_kit::{Anchor, App, Context, Window, anchored, deferred, div, prelude::*, px};
use serde_json::json;
use t3code_desktop_gpui::conversation::{ContextWindowLevel, context_window_snapshot};
use t3code_desktop_gpui::palette;

use super::list_row;
use crate::{
    MentionClose, MentionNext, MentionPick, MentionPrevious, NativeApp, SlashClear, SlashNext,
    SlashPrevious, SlashRun,
};

impl NativeApp {
    fn slash_entries(&self, cx: &App) -> Vec<&'static palette::Entry> {
        palette::slash_matches(
            &self.composer.read(cx).value(),
            self.selected_thread.is_some(),
        )
        .unwrap_or_default()
    }

    /// Runs the "/" menu row at `index`; with no such row the prompt is sent as typed.
    fn run_slash(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(command) = self.slash_entries(cx).get(index).map(|entry| entry.command) else {
            self.send_prompt(window, cx);
            return;
        };
        self.slash_index = 0;
        self.composer
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.execute_command(command, window, cx);
    }

    /// The "@" file menu, opening upward from the composer text.
    fn render_mention_menu(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let notice = if self.mention_loading {
            Some("Searching files…")
        } else if self.mention_entries.is_empty() {
            Some("No matching files or folders")
        } else {
            None
        };
        div()
            .id("mention-menu")
            .debug_selector(|| "mention-menu".into())
            .occlude()
            .mb_1()
            .w(px(480.))
            .max_h(px(240.))
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .p_2()
            .rounded_md()
            .bg(cx.theme().popover)
            .border_1()
            .border_color(cx.theme().border)
            .shadow_lg()
            .when_some(notice, |view, notice| {
                view.child(
                    div()
                        .px_2()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(notice),
                )
            })
            .children(
                self.mention_entries
                    .iter()
                    .enumerate()
                    .map(|(index, entry)| {
                        list_row(
                            format!("mention-{index}"),
                            None,
                            entry.path.clone(),
                            index == self.mention_index,
                        )
                        .on_click(cx.listener(
                            move |app, _, window, cx| app.pick_mention(index, window, cx),
                        ))
                    }),
            )
    }

    /// Kit owns menu focus, arrow navigation, Escape and outside-click dismissal.
    fn composer_more(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let app = cx.entity().downgrade();
        Button::new("composer-more")
            .debug_selector(|| "composer-more".into())
            .icon(IconName::Ellipsis)
            .accessibility_label("More composer actions")
            .tooltip("More composer actions")
            .ghost()
            .small()
            .dropdown_menu_with_anchor(Anchor::BottomRight, move |menu, _, cx| {
                let Some(app) = app.upgrade() else {
                    return menu;
                };
                let view = app.read(cx);
                let text = view.composer.read(cx).value();
                let empty = text.is_empty();
                let blank = text.trim().is_empty();
                // Persistent storage currently exposes only a destructive pop, not a
                // peek. Do not claim that it is empty based on the in-memory stack.
                let no_stash = view.draft_store.is_none() && view.stash_memory.is_empty();
                let persistent = view.draft_store.is_some();
                let stash_app = app.downgrade();
                let restore_app = app.downgrade();
                menu.item(
                    PopupMenuItem::element(move |_, _| {
                        div()
                            .id("stash-label")
                            .debug_selector(|| "stash".into())
                            .aria_label("Stash prompt")
                            .child("Stash prompt")
                    })
                    .icon(IconName::Bookmark)
                    .action(Box::new(crate::StashPrompt))
                    .disabled(blank)
                    .on_click(move |_, window, cx| {
                        _ = stash_app.update(cx, |app, cx| app.stash_prompt(false, window, cx));
                    }),
                )
                .when(blank, |menu| menu.label("Type a prompt to stash it"))
                .item(
                    PopupMenuItem::element(move |_, _| {
                        div()
                            .id("restore-stash-label")
                            .debug_selector(|| "restore-stash".into())
                            .aria_label("Restore stashed prompt")
                            .child("Restore stashed prompt")
                    })
                    .icon(IconName::Undo2)
                    .disabled(!empty || no_stash)
                    .on_click(move |_, window, cx| {
                        _ = restore_app.update(cx, |app, cx| app.stash_prompt(true, window, cx));
                    }),
                )
                .when(!empty, |menu| {
                    menu.label("Stash or clear the current prompt first")
                })
                .when(empty && no_stash, |menu| menu.label("Nothing stashed yet"))
                .when(empty && persistent, |menu| {
                    menu.label("Restores the latest saved prompt, if any")
                })
            })
    }

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
        let slash_entries = palette::slash_matches(&composer_value, self.selected_thread.is_some());
        let mention_menu = self.mention_open(cx).then(|| self.render_mention_menu(cx));
        let thread_id = self.selected_thread.clone().unwrap_or_default();
        let queued = self.queues.messages(&thread_id);
        // A head that is not on the wire while the thread is idle waits on a
        // rejected send; Send now retries it.
        let stalled = !turn_running && !self.queues.dispatching(&thread_id);
        let context_window = (!self.context_meter_hidden)
            .then(|| {
                self.detail
                    .thread
                    .as_ref()?
                    .get("activities")?
                    .as_array()
                    .and_then(|activities| context_window_snapshot(activities))
            })
            .flatten();
        let meter_color = match context_window.as_ref().map(|snapshot| snapshot.level()) {
            Some(ContextWindowLevel::Danger) => cx.theme().danger,
            Some(ContextWindowLevel::Warning) => cx.theme().warning,
            _ => muted_foreground,
        };
        div()
            .debug_selector(|| "composer".into())
            // While the "/" menu shows, arrows, Enter, and Escape drive it instead of the text.
            .when(slash_entries.is_some(), |view| {
                view.key_context("NativeSlashMenu")
            })
            // The "@" menu likewise takes arrows, Enter, Tab, and Escape while it shows.
            .when(mention_menu.is_some(), |view| {
                view.key_context("NativeMentionMenu")
            })
            .on_action(cx.listener(|app, _: &MentionNext, _, cx| {
                let count = app.mention_entries.len();
                app.mention_index = palette::step(app.mention_index, count, true);
                cx.notify();
            }))
            .on_action(cx.listener(|app, _: &MentionPrevious, _, cx| {
                let count = app.mention_entries.len();
                app.mention_index = palette::step(app.mention_index, count, false);
                cx.notify();
            }))
            .on_action(cx.listener(|app, _: &MentionPick, window, cx| {
                app.pick_mention(app.mention_index, window, cx)
            }))
            .on_action(cx.listener(|app, _: &MentionClose, _, cx| app.close_mention(cx)))
            .on_action(cx.listener(|app, _: &SlashNext, _, cx| {
                let count = app.slash_entries(cx).len();
                app.slash_index = palette::step(app.slash_index, count, true);
                cx.notify();
            }))
            .on_action(cx.listener(|app, _: &SlashPrevious, _, cx| {
                let count = app.slash_entries(cx).len();
                app.slash_index = palette::step(app.slash_index, count, false);
                cx.notify();
            }))
            .on_action(cx.listener(|app, _: &SlashRun, window, cx| {
                app.run_slash(app.slash_index, window, cx)
            }))
            .on_action(cx.listener(|app, _: &SlashClear, window, cx| {
                app.composer
                    .update(cx, |input, cx| input.set_value("", window, cx));
                app.slash_index = 0;
                cx.notify();
            }))
            .flex()
            .flex_col()
            .gap_2()
            .flex_shrink_0()
            .p_3()
            .border_t_1()
            .border_color(border)
            .when(!queued.is_empty(), |view| {
                view.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .children(queued.iter().enumerate().map(|(index, message)| {
                            let id = message.id;
                            let remove_thread = thread_id.clone();
                            let send_thread = thread_id.clone();
                            div()
                                .debug_selector(move || format!("queued-{index}"))
                                .flex()
                                .items_center()
                                .gap_2()
                                .px_2()
                                .rounded_sm()
                                .bg(muted)
                                .text_xs()
                                .child(div().text_color(muted_foreground).child("Queued"))
                                .child(div().flex_1().min_w_0().truncate().child(
                                    message.text.lines().next().unwrap_or_default().to_owned(),
                                ))
                                .when(!message.attachments.is_empty(), |view| {
                                    view.child(
                                        div()
                                            .text_color(muted_foreground)
                                            .child(format!("+{}", message.attachments.len())),
                                    )
                                })
                                .when(index == 0 && stalled, |view| {
                                    view.child(
                                        Button::new("queued-send-now")
                                            .debug_selector(|| "queued-send-now".into())
                                            .label("Send now")
                                            .ghost()
                                            .small()
                                            .on_click(cx.listener(move |app, _, _, cx| {
                                                app.send_queued(&send_thread, id, cx)
                                            })),
                                    )
                                })
                                .child(
                                    Button::new(gpui_kit::SharedString::from(format!(
                                        "queued-remove-{index}"
                                    )))
                                    .debug_selector(move || format!("queued-remove-{index}"))
                                    .label("×")
                                    .ghost()
                                    .small()
                                    .tooltip("Remove from the queue")
                                    .on_click(cx.listener(move |app, _, _, cx| {
                                        app.remove_queued(&remove_thread, id, cx)
                                    })),
                                )
                        })),
                )
            })
            .child(
                div()
                    .flex()
                    .flex_col()
                    .when_some(mention_menu, |view, menu| {
                        view.child(deferred(
                            anchored()
                                .anchor(Anchor::BottomLeft)
                                .snap_to_window()
                                .child(menu),
                        ))
                    })
                    .child(Textarea::new(&self.composer)),
            )
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
                                    .px_2()
                                    .text_xs()
                                    .text_color(muted_foreground)
                                    .child("No matching commands"),
                            )
                        })
                        .children(entries.into_iter().enumerate().map(|(index, entry)| {
                            let label = entry.label;
                            list_row(
                                format!("slash-{label}"),
                                None,
                                label,
                                index == self.slash_index,
                            )
                            .on_click(cx.listener(
                                move |app, _, window, cx| app.run_slash(index, window, cx),
                            ))
                        })),
                )
            })
            .child(
                div()
                    .debug_selector(|| "composer-actions".into())
                    .flex()
                    .min_w_0()
                    .items_center()
                    .gap_1()
                    .child(
                        Button::new("attach")
                            .debug_selector(|| "attach".into())
                            .icon(IconName::Paperclip)
                            .accessibility_label("Attach files")
                            .ghost()
                            .small()
                            .tooltip("Attach files to the prompt")
                            .on_click(cx.listener(|app, _, _, cx| app.attach_files(cx))),
                    )
                    .child(
                        Button::new("runtime-mode")
                            .debug_selector(|| "runtime-mode".into())
                            .child(
                                Icon::new(if full_access {
                                    IconName::ShieldOff
                                } else {
                                    IconName::ShieldCheck
                                })
                                .small()
                                .text_color(if full_access {
                                    cx.theme().warning
                                } else {
                                    muted_foreground
                                }),
                            )
                            .accessibility_label(if full_access {
                                "Full access"
                            } else {
                                "Approval required"
                            })
                            .ghost()
                            .small()
                            .tooltip(if full_access {
                                "Full access · click to require approval"
                            } else {
                                "Approval required · click to allow full access"
                            })
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
                            .icon(if plan_mode {
                                IconName::ListTodo
                            } else {
                                IconName::MessageSquare
                            })
                            .accessibility_label(if plan_mode {
                                "Plan mode"
                            } else {
                                "Default mode"
                            })
                            .ghost()
                            .small()
                            .selected(plan_mode)
                            .tooltip(if plan_mode {
                                "Plan mode · click for default interaction"
                            } else {
                                "Default interaction · click to plan first"
                            })
                            .on_click(cx.listener(move |app, _, _, cx| {
                                app.change_thread_mode(
                                    "thread.interaction-mode.set",
                                    "interactionMode",
                                    if plan_mode { "default" } else { "plan" },
                                    cx,
                                )
                            })),
                    )
                    // The catalog opens upward from its chip rather than a fixed window position.
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .flex_1()
                            .min_w_0()
                            .when_some(self.render_catalog(cx), |view, catalog| {
                                view.child(deferred(
                                    anchored()
                                        .anchor(Anchor::BottomLeft)
                                        .snap_to_window()
                                        .child(catalog),
                                ))
                            })
                            .child(
                                Button::new("model-catalog")
                                    .debug_selector(|| "model-catalog".into())
                                    .icon(IconName::Bot)
                                    .child(div().min_w_0().truncate().child(
                                        if active_model.is_empty() {
                                            "Select model".to_owned()
                                        } else {
                                            active_model.clone()
                                        },
                                    ))
                                    .accessibility_label("Choose provider and model")
                                    .ghost()
                                    .small()
                                    .selected(self.catalog_open)
                                    .tooltip(if active_model.is_empty() {
                                        "Choose the provider and model".to_owned()
                                    } else {
                                        format!("{} · choose provider and model", active_model)
                                    })
                                    .on_click(cx.listener(|app, _, _, cx| {
                                        app.catalog_open = !app.catalog_open;
                                        cx.notify();
                                    })),
                            ),
                    )
                    .child(self.composer_more(cx))
                    .when_some(context_window, |view, snapshot| {
                        let label = snapshot.label();
                        view.child(
                            Button::new("context-meter")
                                .debug_selector(|| "context-meter".into())
                                .ghost()
                                .small()
                                .tooltip(snapshot.tooltip())
                                .child(
                                    div()
                                        .debug_selector({
                                            let label = label.clone();
                                            move || format!("context-meter-{label}")
                                        })
                                        .text_color(meter_color)
                                        .child(label),
                                )
                                .when_some(snapshot.used_percentage, |button, percentage| {
                                    button.child(
                                        Progress::new("context-meter-bar")
                                            .value(percentage as f32)
                                            .color(meter_color)
                                            .xsmall()
                                            .w(px(32.)),
                                    )
                                }),
                        )
                    })
                    // One slot: Send while idle, Stop while a turn runs.
                    .child(if turn_running {
                        Button::new("stop-turn")
                            .debug_selector(|| "stop-turn".into())
                            .icon(IconName::Square)
                            .label("Stop")
                            .small()
                            .danger()
                            .outline()
                            .tooltip("Interrupt the running turn (⌘⇧↩ steers with the prompt)")
                            .on_click(cx.listener(|app, _, _, cx| {
                                app.lifecycle("thread.turn.interrupt", cx)
                            }))
                    } else {
                        Button::new("send")
                            .debug_selector(|| "send".into())
                            .icon(IconName::Send)
                            .label("Send")
                            .small()
                            .primary()
                            .loading(self.pending_actions > 0)
                            .disabled(
                                self.selected_thread.is_none()
                                    || self.composer.read(cx).value().trim().is_empty(),
                            )
                            .tooltip("Send the prompt (↩)")
                            .on_click(cx.listener(|app, _, window, cx| app.send_prompt(window, cx)))
                    }),
            )
    }
}
