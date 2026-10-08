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
use t3code_desktop_gpui::conversation::{ContextWindowLevel, context_window_snapshot};
use t3code_desktop_gpui::palette;

use super::list_row;
use crate::{
    MentionClose, MentionNext, MentionPick, MentionPrevious, NativeApp, SlashClear, SlashNext,
    SlashPrevious, SlashRun,
};

#[derive(Clone)]
enum ComposerCommand {
    Model,
    Mode(&'static str),
    Compact,
    UsageLimits,
    Insert(String),
}

#[derive(Clone)]
struct CommandPick {
    label: String,
    description: String,
    skill: Option<serde_json::Value>,
    command: ComposerCommand,
}

impl NativeApp {
    pub(crate) fn render_follow_up_settings(
        &self,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        use t3code_desktop_gpui::queue::FollowUpBehavior;
        div()
            .flex()
            .flex_col()
            .gap_2()
            .child("Send shortcut (device-local)")
            .child(
                div().flex().flex_wrap().gap_1().children(
                    [
                        (
                            t3code_desktop_gpui::drafts::SendShortcut::Enter,
                            "Enter",
                            "send-shortcut-enter",
                        ),
                        (
                            t3code_desktop_gpui::drafts::SendShortcut::ModEnter,
                            "⌘/Ctrl+Enter",
                            "send-shortcut-mod-enter",
                        ),
                        (
                            t3code_desktop_gpui::drafts::SendShortcut::ModEnterMultiline,
                            "Modifier for multiline",
                            "send-shortcut-multiline",
                        ),
                    ]
                    .into_iter()
                    .map(|(shortcut, label, id)| {
                        Button::new(id)
                            .debug_selector(move || id.into())
                            .ghost()
                            .small()
                            .label(label)
                            .selected(self.send_shortcut() == shortcut)
                            .on_click(
                                cx.listener(move |app, _, _, cx| {
                                    app.set_send_shortcut(shortcut, cx)
                                }),
                            )
                    }),
                ),
            )
            .child(
                Button::new("skills-in-slash-menu")
                    .debug_selector(|| "skills-in-slash-menu".into())
                    .ghost()
                    .small()
                    .label("Show skills in / menu ($ always includes skills)")
                    .selected(self.show_skills_in_slash_menu())
                    .on_click(cx.listener(|app, _, _, cx| {
                        app.set_show_skills_in_slash_menu(!app.show_skills_in_slash_menu(), cx)
                    })),
            )
            .child("Follow-up behavior (⌘⇧↩ uses the alternate)")
            .child(
                div().flex().gap_2().children(
                    [
                        (FollowUpBehavior::Queue, "Queue", "follow-up-queue"),
                        (FollowUpBehavior::Steer, "Steer", "follow-up-steer"),
                    ]
                    .into_iter()
                    .map(|(behavior, label, id)| {
                        Button::new(id)
                            .debug_selector(move || id.into())
                            .label(label)
                            .ghost()
                            .small()
                            .selected(self.follow_up_behavior == behavior)
                            .on_click(cx.listener(move |app, _, _, cx| {
                                app.follow_up_behavior = behavior;
                                if let Some(mut state) = app.saved_window {
                                    state.follow_up_behavior = behavior;
                                    app.saved_window = Some(state);
                                    t3code_desktop_gpui::window_state::save(state);
                                }
                                cx.notify();
                            }))
                    }),
                ),
            )
    }

    fn slash_entries(&self, cx: &App) -> Vec<CommandPick> {
        let Some((range, marker, query)) = self.command_trigger(cx) else {
            return Vec::new();
        };
        let mut entries = Vec::new();
        let Some(provider) = self
            .selected_provider(cx)
            .filter(|provider| provider.selectable())
        else {
            return entries;
        };
        if marker == '/' {
            entries.push(CommandPick {
                label: "/model".into(),
                description: "Choose provider and model".into(),
                skill: None,
                command: ComposerCommand::Model,
            });
            if provider.show_interaction_mode_toggle.unwrap_or(true) {
                for (name, label) in [("plan", "/plan"), ("default", "/default")] {
                    entries.push(CommandPick {
                        label: label.into(),
                        description: "Change interaction mode".into(),
                        skill: None,
                        command: ComposerCommand::Mode(name),
                    });
                }
            }
            for command in provider.commands_for_cwd(self.selected_workspace().as_deref()) {
                // Provider expansion is valid only at message start; local builtins
                // and skill mentions remain available at any prompt position.
                if range.start != 0 {
                    continue;
                }
                let Some(name) = command["name"].as_str() else {
                    continue;
                };
                if self.show_skills_in_slash_menu()
                    && provider
                        .skills_for_cwd(self.selected_workspace().as_deref())
                        .iter()
                        .any(|skill| {
                            skill["name"]
                                .as_str()
                                .is_some_and(|skill| skill.eq_ignore_ascii_case(name))
                        })
                {
                    continue;
                }
                if matches!(name, "model" | "plan" | "default") {
                    continue;
                }
                if name == "compact" && self.compact_disabled_reason(cx).is_some() {
                    continue;
                }
                entries.push(CommandPick {
                    label: format!("/{name}"),
                    description: command["description"]
                        .as_str()
                        .or(command["input"]["hint"].as_str())
                        .unwrap_or("Provider command")
                        .into(),
                    skill: None,
                    command: if name == "compact" {
                        ComposerCommand::Compact
                    } else if name == "usage-limits" {
                        ComposerCommand::UsageLimits
                    } else {
                        ComposerCommand::Insert(format!("/{name} "))
                    },
                });
            }
        }
        for skill in provider
            .skills_for_cwd(self.selected_workspace().as_deref())
            .into_iter()
            .filter(|_| marker == '$' || self.show_skills_in_slash_menu())
        {
            let Some(name) = skill["name"].as_str() else {
                continue;
            };
            let name = name.trim();
            entries.push(CommandPick {
                label: if marker == '/' {
                    format!("/skill:{name}")
                } else {
                    format!("${name}")
                },
                description: format!(
                    "{}{}",
                    skill["displayName"]
                        .as_str()
                        .filter(|name| !name.trim().is_empty())
                        .map(|name| format!("{} · ", name.trim()))
                        .unwrap_or_default(),
                    skill["shortDescription"]
                        .as_str()
                        .or(skill["description"].as_str())
                        .unwrap_or("Provider skill")
                ),
                skill: Some(skill.clone()),
                command: ComposerCommand::Insert(format!("${name} ")),
            });
        }
        if query.trim().is_empty() {
            return entries;
        }
        let mut ranked: Vec<_> = entries
            .into_iter()
            .filter_map(|entry| {
                let score = if let Some(skill) = &entry.skill {
                    t3code_desktop_gpui::composer_search::skill_score(skill, &query)
                } else {
                    t3code_desktop_gpui::composer_search::command_score(
                        &entry.label,
                        &entry.description,
                        &query,
                    )
                };
                score.map(|score| (score, entry))
            })
            .collect();
        ranked.sort_by(|(a, left), (b, right)| a.cmp(b).then_with(|| left.label.cmp(&right.label)));
        ranked.into_iter().map(|(_, entry)| entry).collect()
    }

    /// Provider commands insert native invocation text; client builtins act locally.
    fn run_slash(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(command) = self
            .slash_entries(cx)
            .get(index)
            .map(|entry| entry.command.clone())
        else {
            if self.composer_state.command_loading {
                self.set_status("Wait for the workspace command catalog", cx);
                cx.notify();
            } else {
                self.send_prompt(window, cx);
            }
            return;
        };
        let Some((range, _, _)) = self.command_trigger(cx) else {
            return;
        };
        let replacement = match &command {
            ComposerCommand::Insert(value) => value.as_str(),
            _ => "",
        };
        self.composer.update(cx, |input, cx| {
            if let Some(name) = replacement.strip_prefix('$').map(str::trim) {
                let token = gpui_kit::component::input::InlineToken::new(
                    format!("skill:{name}"),
                    format!("${name}"),
                );
                if input
                    .replace_range_with_token(range.clone(), token, window, cx)
                    .is_ok()
                {
                    input.replace(" ", window, cx);
                    return;
                }
            }
            input.set_selected_range(range, cx);
            input.replace(replacement, window, cx);
        });
        self.slash_index = 0;
        match command {
            ComposerCommand::Model => self.open_model_catalog(window, cx),
            ComposerCommand::Mode(mode) => {
                self.change_thread_mode("thread.interaction-mode.set", "interactionMode", mode, cx)
            }
            ComposerCommand::Compact => self.compact_context(cx),
            ComposerCommand::UsageLimits => self.toggle_usage_limits(cx),
            ComposerCommand::Insert(_) => {}
        }
        self.remember_composer_draft(cx);
        cx.notify();
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
                let empty = text.trim().is_empty() && view.attachments.is_empty();
                let blank = empty;
                let entries = view.composer_stashes().to_vec();
                let no_stash = entries.is_empty() && view.stash_memory.is_empty();
                let busy = view.pending_actions > 0
                    || view.attachment_drafts.uploading
                    || view.attachment_drafts.stash_deleting
                    || view.attachment_drafts.stash_writing;
                let selected = view.selected_thread.is_some();
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
                    .disabled(blank || busy)
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
                    .disabled(!empty || no_stash || busy || !selected)
                    .on_click(move |_, window, cx| {
                        _ = restore_app.update(cx, |app, cx| app.stash_prompt(true, window, cx));
                    }),
                )
                .when(!empty, |menu| {
                    menu.label("Stash or clear the current prompt first")
                })
                .when(empty && no_stash, |menu| menu.label("Nothing stashed yet"))
                .label(format!("Stash · {} entries", entries.len()))
                .when(busy, |menu| {
                    menu.label("Wait for pending sends, uploads or deletions")
                })
                .when(!selected, |menu| menu.label("Select a thread to restore"))
                .when(!entries.is_empty(), |mut menu| {
                    for entry in entries.into_iter().rev() {
                        let restore_app = app.downgrade();
                        let delete_app = app.downgrade();
                        let restore_id = entry.id.clone();
                        let delete_id = entry.id.clone();
                        let restore = PopupMenuItem::new(format!(
                            "{} · {}",
                            entry.snippet(),
                            entry.created_at
                        ))
                        .icon(IconName::Bookmark)
                        .disabled(!empty || busy || !selected)
                        .on_click(move |_, window, cx| {
                            _ = restore_app
                                .update(cx, |app, cx| app.restore_stash(&restore_id, window, cx));
                        });
                        let delete = PopupMenuItem::new(format!("Delete: {}", entry.snippet()))
                            .icon(IconName::Trash)
                            .disabled(busy)
                            .on_click(move |_, _, cx| {
                                _ = delete_app
                                    .update(cx, |app, cx| app.delete_stash(&delete_id, cx));
                            });
                        menu = menu.item(restore).item(delete);
                    }
                    menu
                })
            })
    }

    pub(crate) fn render_composer(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        self.composer_state.release_retired_previews(cx, |image| {
            self.conversation_presentation.media.values().any(|preview| {
                matches!(preview, crate::views::message_media::MediaPreview::Ready(live) if live.id() == image.id())
            })
        });
        let muted_foreground = cx.theme().muted_foreground;
        let border = cx.theme().border;
        let muted = cx.theme().muted;
        let popover = cx.theme().popover;
        let (mut runtime_mode, mut interaction_mode) = self.selected_modes();
        if let Some(provider) = self.selected_provider(cx) {
            let supported = provider.runtime_modes();
            if !supported.contains(&runtime_mode.as_str())
                && let Some(mode) = provider
                    .default_runtime_mode
                    .as_deref()
                    .filter(|mode| supported.contains(mode))
                    .or_else(|| supported.first().copied())
            {
                runtime_mode = mode.to_owned();
            }
            if provider.show_interaction_mode_toggle == Some(false) {
                interaction_mode = "default".into();
            }
        }
        let full_access = runtime_mode == "full-access";
        let plan_mode = interaction_mode == "plan";
        let turn_running = self.turn_running();
        let active_model = self.active_model_label(cx);
        let composer_value = self.composer.read(cx).value().to_string();
        let slash_entries = self.command_trigger(cx).map(|_| self.slash_entries(cx));
        let mention_menu = self.mention_open(cx).then(|| self.render_mention_menu(cx));
        let thread_id = self.selected_thread.clone().unwrap_or_default();
        let queued = self.queues.messages(&thread_id);
        // A rejected send stays available for explicit retry even mid-turn.
        let stalled = !self.queues.dispatching(&thread_id);
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
            .capture_key_down(cx.listener(|app, event, window, cx| app.composer_key(event, window, cx)))
            .capture_action(cx.listener(|app, action: &gpui_kit::component::input::Enter, window, cx| {
                let key = if action.secondary { "cmd-enter" } else if action.shift { "shift-enter" } else { "enter" };
                app.composer_action_key(key, window, cx);
            }))
            .capture_action(cx.listener(|app, _: &gpui_kit::component::input::Copy, window, cx| app.composer_action_key("cmd-c", window, cx)))
            .capture_action(cx.listener(|app, _: &gpui_kit::component::input::Cut, window, cx| app.composer_action_key("cmd-x", window, cx)))
            .capture_action(cx.listener(|app, _: &gpui_kit::component::input::MoveUp, window, cx| app.composer_action_key("up", window, cx)))
            .capture_action(cx.listener(|app, _: &gpui_kit::component::input::MoveDown, window, cx| app.composer_action_key("down", window, cx)))
            .on_drop::<gpui_kit::ExternalPaths>(cx.listener(|app, paths: &gpui_kit::ExternalPaths, _, cx| {
                if let Some(scope) = app.attachment_scope() { app.attach_paths(scope, Some(paths.0.to_vec()), cx); }
                else { app.report_error("Select a connected thread before dropping files", cx); cx.notify(); }
            }))
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
            .when(composer_value.encode_utf16().count() > 120_000, |view| view.child(div().text_color(cx.theme().danger).child("Prompt exceeds the 120,000-character limit. Shorten or split before sending.")))
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
                                .child(div().text_color(muted_foreground).child(
                                    if index == 0 && self.queues.held(&thread_id) {
                                        "Not sent"
                                    } else {
                                        "Queued"
                                    },
                                ))
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
                                            .disabled(self.checkout_transition_pending())
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
            .when(self.composer_state.stash_picker, |view| {
                view.child(div().id("shortcut-stash-picker").debug_selector(|| "shortcut-stash-picker".into())
                    .flex().flex_col().gap_1().max_h(px(240.)).overflow_y_scroll()
                    .when_some(self.composer_state.stash_focus.as_ref(), |view, focus| view.track_focus(focus))
                    .on_key_down(cx.listener(|app, event: &gpui_kit::KeyDownEvent, window, cx| {
                        match event.keystroke.key.as_str() {
                            "up" | "down" => app.composer_state.stash_index = palette::step(app.composer_state.stash_index, app.composer_stashes().len(), event.keystroke.key == "down"),
                            "enter" => { if let Some(entry) = app.composer_stashes().get(app.composer_state.stash_index) { let id = entry.id.clone(); app.restore_stash(&id, window, cx); } },
                            "escape" => { app.composer_state.stash_picker = false; app.composer.update(cx, |input, cx| input.focus(window, cx)); },
                            _ => return,
                        }
                        cx.stop_propagation(); cx.notify();
                    }))
                    .child("Choose a stashed prompt")
                    .children(self.composer_stashes().iter().enumerate().map(|(index, entry)| {
                        let id = entry.id.clone();
                        Button::new(gpui_kit::SharedString::from(format!("shortcut-stash-{index}")))
                            .debug_selector(move || format!("shortcut-stash-{index}"))
                            .ghost().small().selected(index == self.composer_state.stash_index)
                            .disabled(self.attachment_drafts.stash_writing)
                            .label(format!("{} · {}", entry.snippet(), entry.created_at))
                            .on_click(cx.listener(move |app, _, window, cx| app.restore_stash(&id, window, cx)))
                    })))
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
                    .child(Textarea::new(&self.composer).on_paste({
                        let app = cx.entity().downgrade();
                        move |clipboard, window, cx| app.update(cx, |app, cx| app.paste_context_fragment(clipboard, window, cx) || app.paste_composer(clipboard, cx)).unwrap_or(false)
                    })),
            )
            .when(self.composer_state.pending_paste.as_ref().is_some_and(|(scope, _)| scope.origin == self.environment_scope && Some(&scope.thread) == self.selected_thread.as_ref()), |view| {
                view.child(div().flex().items_center().gap_2().child("Large text paste")
                    .child(Button::new("paste-attach-text").debug_selector(|| "paste-attach-text".into()).ghost().small().label("Attach as text file").disabled(self.attachment_drafts.uploading).on_click(cx.listener(|app, _, window, cx| app.finish_large_paste(false, window, cx))))
                    .child(Button::new("paste-as-text").debug_selector(|| "paste-as-text".into()).ghost().small().label("Paste as text").disabled(self.attachment_drafts.uploading).on_click(cx.listener(|app, _, window, cx| app.finish_large_paste(true, window, cx))))
                    .child(Button::new("paste-cancel").debug_selector(|| "paste-cancel".into()).ghost().small().label("Cancel paste").disabled(self.attachment_drafts.uploading).on_click(cx.listener(|app, _, _, cx| { app.composer_state.pending_paste = None; cx.notify(); }))))
            })
            .children(self.attachment_drafts.missing_stash_resources.iter().filter(|((origin, _), _)| origin == &self.environment_scope).flat_map(|((_, stash), resources)| resources.iter().map(move |r| (stash.clone(), r.clone()))).map(|(stash, resource)| {
                let id = resource["id"].as_str().unwrap_or_default().to_owned();
                let restore = stash.clone();
                let retry_button = format!("stash-retry-{restore}-{id}");
                div().flex().items_center().gap_2().text_xs().child(format!("Unavailable stash resource: {}", resource["name"].as_str().unwrap_or("file")))
                    .child(Button::new(gpui_kit::SharedString::from(format!("stash-drop-{id}"))).ghost().small().label("Remove unavailable file").disabled(self.attachment_drafts.stash_writing).on_click(cx.listener(move |app, _, _, cx| app.discard_unavailable_stash_resource(&stash, &id, cx))))
                    .child(Button::new(gpui_kit::SharedString::from(retry_button)).ghost().small().label("Retry restore").disabled(self.attachment_drafts.stash_writing).on_click(cx.listener(move |app, _, window, cx| app.restore_stash(&restore, window, cx))))
            }))
            .children(self.attachment_drafts.transfers.iter().filter(|r| r.owner == (self.environment_scope.clone(), self.selected_thread.clone().unwrap_or_default()) && r.phase != "Attached").map(|row| {
                let cancel_id = row.id.clone();
                let retry_id = row.id.clone();
                let discard_id = row.id.clone();
                let active = matches!(row.phase, "Waiting" | "Reading" | "Preparing" | "Uploading");
                div().flex().items_center().gap_2().text_xs().child(format!("{} — {}", row.name, row.phase))
                    .when(active, |view| view.child(Button::new(gpui_kit::SharedString::from(format!("transfer-cancel-{}", row.id))).ghost().small().label("Cancel").on_click(cx.listener(move |app, _, _, cx| app.cancel_transfer(&cancel_id, cx)))))
                    .when(row.retry.is_some(), |view| view
                        .child(Button::new(gpui_kit::SharedString::from(format!("transfer-retry-{}", row.id))).ghost().small().label("Retry").disabled(self.attachment_drafts.uploading).on_click(cx.listener(move |app, _, _, cx| app.retry_transfer(&retry_id, cx))))
                        .child(Button::new(gpui_kit::SharedString::from(format!("transfer-discard-{}", row.id))).ghost().small().label("Discard failed transfer").on_click(cx.listener(move |app, _, _, cx| app.cancel_transfer(&discard_id, cx)))))
            }))
            .children(self.draft_context_records().into_iter().filter(|r| self.composer.read(cx).value().contains(&r.reference())).map(|record| {
                let id = record.id().to_owned();
                div().flex().items_center().gap_2().text_xs()
                    .child(format!("{}: {}", record.kind(), record.label()))
                    .when(record.kind() == "assistant-citation", |view| {
                        let citation = record.value()["payload"].clone();
                        view.child(Button::new(gpui_kit::SharedString::from(format!("draft-citation-source-{id}"))).ghost().small().label("Quoted source")
                            .on_click(cx.listener(move |app, _, _, cx| app.jump_to_citation(&citation, cx))))
                    })
                    .child(Button::new(gpui_kit::SharedString::from(format!("context-remove-{id}"))).ghost().small().label("Remove")
                        .on_click(cx.listener(move |app, _, window, cx| app.remove_draft_context(&id, window, cx))))
            }))
            .when(!self.attachments.is_empty(), |view| {
                view.child(
                    div().flex().flex_wrap().gap_1().children(
                        self.attachments
                            .iter()
                            .enumerate()
                            .map(|(index, attachment)| {
                                let id = attachment["id"].as_str().unwrap_or_default().to_owned();
                                let attachment_scope = self.attachment_scope();
                                let preview_id = id.clone();
                                let expand_key = (self.environment_scope.clone(), id.clone());
                                let has_preview = self.composer_state.previews.contains_key(&expand_key);
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
                                    .when_some(self.composer_state.previews.get(&(self.environment_scope.clone(), id.clone())), |view, preview| view.child(div().id(gpui_kit::SharedString::from(format!("expand-preview-{index}"))).debug_selector(move || format!("expand-preview-{index}")).child(gpui_kit::img(preview.clone()).w(px(40.)).h(px(40.)).object_fit(gpui_kit::ObjectFit::Contain)).on_click(cx.listener(move |app, _, _, cx| { app.composer_state.expanded_preview = Some(expand_key.clone()); cx.notify(); }))))
                                    .when(!has_preview && attachment["type"] == "image", |view| view.child(Button::new(gpui_kit::SharedString::from(format!("recover-preview-{index}"))).ghost().small().label(if self.composer_state.preview_errors.contains(&(self.environment_scope.clone(), id.clone())) { "Retry preview" } else { "Load preview" }).disabled(self.composer_state.preview_loading.contains(&(self.environment_scope.clone(), id.clone()))).on_click(cx.listener(move |app, _, _, cx| app.recover_attachment_preview(preview_id.clone(), cx)))))
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
                                                if let Some(scope) = attachment_scope.clone() {
                                                    app.detach_attachment(scope, id.clone(), cx);
                                                }
                                            }),
                                        ),
                                    )
                            }),
                    ),
                )
            })
            .when_some(self.composer_state.expanded_preview.as_ref().filter(|(origin, id)| origin == &self.environment_scope && self.attachments.iter().any(|a| a["id"].as_str() == Some(id))), |view, key| {
                view.child(div().flex().flex_col().gap_2()
                    .when_some(self.composer_state.previews.get(key), |view, image| view.child(gpui_kit::img(image.clone()).w_full().h(px(240.)).object_fit(gpui_kit::ObjectFit::Contain)))
                    .child(Button::new("composer-preview-close").debug_selector(|| "composer-preview-close".into()).ghost().small().label("Close preview").on_click(cx.listener(|app, _, _, cx| { app.composer_state.expanded_preview = None; cx.notify(); }))))
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
                        .when(self.composer_state.command_loading, |view| view.child(div().text_xs().text_color(muted_foreground).child("Loading workspace commands and skills…")))
                        .when_some(self.composer_state.command_error.as_ref(), |view, error| view.child(div().text_xs().text_color(cx.theme().danger).child(error.clone())))
                        .child(Button::new("refresh-composer-catalog").debug_selector(|| "refresh-composer-catalog".into()).ghost().small().label("Refresh workspace catalog").on_click(cx.listener(|app, _, _, cx| app.refresh_composer_catalog(true, cx))))
                        .when(entries.is_empty(), |view| {
                            view.child(
                                div()
                                    .px_2()
                                    .text_xs()
                                    .text_color(muted_foreground)
                                    .child(if self.selected_provider(cx).is_none_or(|provider| !provider.selectable()) { "Select an available provider to browse commands and skills" } else { "No matching commands or eligible skills" }),
                            )
                        })
                        .children(entries.into_iter().enumerate().map(|(index, entry)| {
                            let label = entry.label;
                            list_row(
                                format!("slash-{label}"),
                                None,
                                format!("{} · {}", label, entry.description),
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
                        Button::new("attach").disabled(!self.connection_ready())
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
                            .accessibility_label(format!("Runtime mode: {runtime_mode}"))
                            .ghost()
                            .small()
                            .disabled(self.selected_thread.is_none() || self.selected_provider(cx).is_none_or(|provider| !provider.selectable() || provider.runtime_modes().len() < 2) || self.pending_actions > 0 || !self.connection_ready())
                            .tooltip(if self.selected_provider(cx).is_none() { "Load the provider catalog before changing permissions".to_owned() }
                                else if self.pending_actions > 0 { "Wait for the pending server receipt".to_owned() }
                                else if let Some(provider) = self.selected_provider(cx).filter(|provider| !provider.selectable()) { provider.explanation().to_owned() }
                                else { format!("{runtime_mode} · provider-supported permission policies") })
                            .dropdown_menu_with_anchor(Anchor::BottomLeft, {
                                let app = cx.entity().downgrade();
                                move |mut menu, _, cx| {
                                    let Some(view) = app.upgrade() else { return menu; };
                                    let modes = view.read(cx).selected_provider(cx).map(|provider| provider.runtime_modes().into_iter().map(str::to_owned).collect::<Vec<_>>()).unwrap_or_default();
                                    for mode in modes {
                                        let app = app.clone();
                                        menu = menu.item(PopupMenuItem::new(mode.clone()).on_click(move |_, _, cx| {
                                            _ = app.update(cx, |app, cx| app.change_thread_mode("thread.runtime-mode.set", "runtimeMode", &mode, cx));
                                        }));
                                    }
                                    menu
                                }
                            }),
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
                            .disabled(self.selected_thread.is_none() || self.selected_provider(cx).is_none_or(|provider| !provider.selectable() || !provider.show_interaction_mode_toggle.unwrap_or(true)) || self.pending_actions > 0 || !self.connection_ready())
                            .tooltip(if self.selected_provider(cx).is_none_or(|provider| !provider.selectable() || !provider.show_interaction_mode_toggle.unwrap_or(true)) {
                                "Plan mode is unavailable for this provider"
                            } else if self.pending_actions > 0 {
                                "Wait for the pending server receipt"
                            } else if plan_mode {
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
                                    .on_click(cx.listener(|app, _, window, cx| {
                                        app.open_model_catalog(window, cx);
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
                                })
                                .dropdown_menu_with_anchor(Anchor::BottomRight, {
                                    let app = cx.entity().downgrade();
                                    let details = snapshot.tooltip();
                                    let payload = self.detail.thread.as_ref().and_then(|thread| thread["activities"].as_array()).and_then(|activities| activities.iter().rev().find(|activity| activity["kind"] == "context-window.updated")).map(|activity| activity["payload"].clone()).unwrap_or_default();
                                    let automatic = payload["compactsAutomatically"] == true;
                                    let threshold = payload["autoCompactThreshold"].as_f64();
                                    move |mut menu, _, cx| {
                                        menu = menu.label("Context window").label(details.clone());
                                        for (field, label) in [("inputTokens", "Input"), ("cachedInputTokens", "Cached input"), ("outputTokens", "Output"), ("reasoningOutputTokens", "Reasoning output")] {
                                            if let Some(tokens) = payload[field].as_f64().filter(|tokens| tokens.is_finite() && *tokens >= 0.) { menu = menu.label(format!("{label}: {} tokens", t3code_desktop_gpui::conversation::format_context_window_tokens(tokens))); }
                                        }
                                        if automatic {
                                            menu = menu.label(threshold.map(|threshold| format!("Provider automatically compacts at {} tokens", t3code_desktop_gpui::conversation::format_context_window_tokens(threshold))).unwrap_or_else(|| "Provider automatically compacts as context fills".into()));
                                        }
                                        let Some(view) = app.upgrade() else { return menu; };
                                        let reason = view.read(cx).compact_disabled_reason(cx);
                                        let action_app = app.clone();
                                        menu = menu.item(PopupMenuItem::new("Compact context").icon(IconName::Minimize).disabled(reason.is_some()).on_click(move |_, _, cx| {
                                            _ = action_app.update(cx, |app, cx| app.compact_context(cx));
                                        }));
                                        if let Some(reason) = reason { menu = menu.label(reason); }
                                        menu
                                    }
                                }),
                        )
                    })
                    // One slot: Send while idle, Stop while a turn runs.
                    .child(if turn_running {
                        Button::new("stop-turn").disabled(!self.connection_ready())
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
                            .disabled((
                                self.selected_thread.is_none()
                                    || self.checkout_transition_pending()
                                    || self.attachment_drafts.stash_writing
                                    || self.attachment_drafts.uploading
                                    || self.composer_state.pending_paste.as_ref().is_some_and(|(scope, _)| scope.origin == self.environment_scope && Some(&scope.thread) == self.selected_thread.as_ref())
                                    || composer_value.encode_utf16().count() > 120_000
                                    || (self.composer.read(cx).value().trim().is_empty()
                                        && self.attachments.is_empty())
                            ) || !self.connection_ready())
                            .tooltip(if self.send_shortcut().requires_modifier(&composer_value) { "Send the prompt (⌘/Ctrl+↩)" } else { "Send the prompt (↩)" })
                            .on_click(cx.listener(|app, _, window, cx| app.send_prompt(window, cx)))
                    }),
            )
    }
}
