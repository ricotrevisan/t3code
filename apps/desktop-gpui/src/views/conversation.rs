use std::rc::Rc;

use gpui_kit::assets::IconName;
use gpui_kit::base::{Disableable, Selectable};
use gpui_kit::component::{
    ActiveTheme as _, Icon, Sizable,
    button::{Button, ButtonVariants as _},
    input::{Input, InputEvent},
    text::TextView,
};
use gpui_kit::{AnyElement, Context, FontWeight, Window, div, list, prelude::*, px};
use serde_json::Value;
use t3code_desktop_gpui::conversation::{
    self, ActivityItem, PendingApproval, PendingUserInput, TimelineEntry, TimelineRow, ToolCall,
    ToolState,
};

use crate::{NativeApp, single_line_input};

impl NativeApp {
    /// Search may call this before the matching thread snapshot arrives.
    pub(crate) fn scroll_to_message(&mut self, id: &str) {
        self.activity_expanded
            .retain(|key, _| !key.starts_with("fold:"));
        self.conversation_presentation.jump = Some(crate::conversation_presentation::MessageJump {
            generation: self.generation,
            environment: self.environment_scope.clone(),
            thread: self.selected_thread.clone(),
            message: id.to_owned(),
        });
    }

    fn copy_conversation_text(&mut self, key: String, text: String, cx: &mut Context<Self>) {
        cx.write_to_clipboard(gpui_kit::ClipboardItem::new_string(text));
        self.conversation_presentation.copied = Some(key.clone());
        self.conversation_presentation.copied_epoch =
            self.conversation_presentation.copied_epoch.wrapping_add(1);
        let epoch = self.conversation_presentation.copied_epoch;
        cx.notify();
        cx.spawn(async move |entity, cx| {
            cx.background_executor()
                .timer(std::time::Duration::from_secs(2))
                .await;
            let _ = entity.update(cx, |app, cx| {
                if app.conversation_presentation.copied_epoch == epoch
                    && app.conversation_presentation.copied.as_ref() == Some(&key)
                {
                    app.conversation_presentation.copied = None;
                    cx.notify();
                }
            });
        })
        .detach();
    }

    /// Authenticated RPC issues an exact-resource capability; never expose the access token.
    pub(crate) fn open_conversation_asset(&mut self, resource: Value, cx: &mut Context<Self>) {
        let Some(sender) = self.commands.clone() else {
            self.report_error("Connect to open attachments", cx);
            return;
        };
        let origin = self.environment_scope.clone();
        let generation = self.generation;
        let thread = self.selected_thread.clone();
        cx.spawn(async move |entity, cx| {
            let result = t3code_desktop_gpui::connection::call(
                &sender,
                "assets.createUrl",
                serde_json::json!({"resource":resource}),
            )
            .await;
            let _ = entity.update(cx, |app, cx| {
                if app.generation != generation
                    || app.environment_scope != origin
                    || app.selected_thread != thread
                {
                    return;
                }
                match result {
                    Ok(value) => match super::message_media::signed_asset_url(&origin, &value) {
                        Ok(url) => cx.open_url(&url),
                        Err(_) => app.report_error("Invalid attachment access response", cx),
                    },
                    Err(error) => {
                        app.report_error(format!("Could not open attachment: {error}"), cx)
                    }
                }
            });
        })
        .detach();
    }

    pub(crate) fn render_conversation(
        &mut self,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        if self.pending_actions == 0
            && self.composer.read(cx).value().is_empty()
            && self
                .conversation_presentation
                .plan_followup
                .as_ref()
                .is_some_and(|record| {
                    record.generation == self.generation
                        && record.environment == self.environment_scope
                        && self.selected_thread.as_deref() == Some(record.thread.as_str())
                })
        {
            self.conversation_presentation.plan_followup = None;
        }
        self.conversation_scroll.set_scroll_handler(cx.listener(
            |app, event: &gpui_kit::ListScrollEvent, _, cx| {
                let reading = !event.is_following_tail;
                if app.conversation_presentation.reading_history != reading {
                    app.conversation_presentation.reading_history = reading;
                    cx.notify();
                }
            },
        ));
        let foreground = cx.theme().foreground;
        let muted_foreground = cx.theme().muted_foreground;
        let provider_banner = self
            .selected_provider(cx)
            .filter(|provider| !provider.selectable() || provider.status == "warning")
            .map(|provider| {
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .p_2()
                    .text_xs()
                    .text_color(cx.theme().warning)
                    .child(format!("{} · {}", provider.label(), provider.explanation()))
                    .child(
                        div()
                            .flex()
                            .gap_1()
                            .child(
                                Button::new("timeline-provider-settings")
                                    .ghost()
                                    .small()
                                    .label("Provider settings…")
                                    .on_click(cx.listener(|app, _, window, cx| {
                                        app.execute_command(
                                            t3code_desktop_gpui::palette::Command::Settings,
                                            window,
                                            cx,
                                        );
                                    })),
                            )
                            .child(
                                Button::new("timeline-provider-refresh")
                                    .ghost()
                                    .small()
                                    .label("Refresh status")
                                    .on_click(cx.listener(|app, _, _, cx| {
                                        app.perform(
                                            "server.getConfig",
                                            serde_json::json!({}),
                                            None,
                                            cx,
                                        )
                                    })),
                            ),
                    )
            });
        let conversation = div()
            .id("conversation")
            .relative()
            .min_h(px(140.))
            .flex_shrink_0()
            .flex()
            .flex_col()
            .flex_1()
            .children(provider_banner)
            .children(self.render_timeline_setup(cx));
        let search_target = self
            .sidebar_state
            .message_anchor
            .as_ref()
            .filter(|anchor| {
                anchor.environment == self.environment_scope
                    && Some(anchor.thread_id.as_str()) == self.selected_thread.as_deref()
            })
            .and_then(|anchor| {
                self.detail.thread.as_ref().and_then(|detail| {
                    t3code_desktop_gpui::sidebar::resolve_search_anchor(anchor, detail)
                })
            });
        if let Some(id) = search_target {
            self.scroll_to_message(&id);
        }
        if self.detail.thread.is_none() {
            self.conversation_presentation.needs_preparation(
                self.generation,
                0,
                self.selected_thread.as_deref().unwrap_or(""),
                &self.activity_expanded,
                None,
            );
            for image in std::mem::take(&mut self.conversation_presentation.retired_images) {
                self.release_message_image(image, cx);
            }
        }
        if let Some(detail) = &self.detail.thread {
            let incremental = self
                .detail
                .tail_change
                .as_ref()
                .filter(|(base, turn)| {
                    self.conversation_presentation.can_prepare_tail(
                        self.generation,
                        *base,
                        detail["id"].as_str().unwrap_or(""),
                        &self.activity_expanded,
                    ) && !self.conversation_presentation.same_projection(Some(detail))
                        && self.conversation_presentation.tail_matches(detail, turn)
                })
                .map(|(_, turn)| turn.clone());
            if self.conversation_presentation.needs_preparation(
                self.generation,
                self.detail.revision,
                detail["id"].as_str().unwrap_or(""),
                &self.activity_expanded,
                Some(detail),
            ) {
                for image in std::mem::take(&mut self.conversation_presentation.retired_images) {
                    self.release_message_image(image, cx);
                }
                let detail = self.detail.thread.as_ref().expect("prepared thread");
                let tail = incremental.and_then(|turn| {
                    conversation::timeline_tail_rows(
                        detail,
                        &self.activity_expanded,
                        &self.conversation_rows,
                        &turn,
                    )
                });
                let rows = if let Some(rows) = tail {
                    self.conversation_presentation.prepared_turns += 1;
                    rows
                } else {
                    let (rows, count) =
                        conversation::timeline_rows_counted(detail, &self.activity_expanded);
                    self.conversation_presentation.prepared_turns += count;
                    rows
                };
                let diff = conversation::diff_rows(&self.conversation_rows, &rows);
                if !diff.replaced.is_empty() || diff.inserted > 0 {
                    self.conversation_scroll
                        .splice(diff.replaced, diff.inserted);
                }
                for index in diff.changed {
                    self.conversation_scroll.remeasure_items(index..index + 1);
                }
                let open_request = rows.iter().find_map(|row| match &row.entry {
                    TimelineEntry::UserInput(request) => Some(request.request_id.clone()),
                    _ => None,
                });
                if open_request != self.user_input_request {
                    self.user_input_request = open_request;
                    self.user_input_selected.clear();
                    self.user_input_custom.clear();
                }
                self.conversation_rows = Rc::new(rows);
                self.detail.tail_change = None;
            }
            if self
                .conversation_presentation
                .jump
                .as_ref()
                .is_some_and(|jump| {
                    jump.generation != self.generation
                        || jump.environment != self.environment_scope
                        || jump.thread != self.selected_thread
                })
            {
                self.conversation_presentation.jump = None;
            }
            if let Some(jump) = self.conversation_presentation.jump.as_ref()
                && let Some(index) = self
                    .conversation_rows
                    .iter()
                    .position(|row| row.key == format!("message:{}", jump.message))
            {
                self.conversation_scroll.pause_following_tail();
                self.conversation_scroll.scroll_to_reveal_item(index);
                self.conversation_presentation.jump = None;
                self.sidebar_state.message_anchor = None;
            }
            let reading_history = !self.conversation_scroll.is_following_tail();
            self.conversation_presentation.reading_history = reading_history;
            // Only rows near the viewport are built; each frame renders them afresh.
            let rows = self.conversation_rows.clone();
            let app = cx.weak_entity();
            return conversation
                .py_2()
                .child(
                    list(
                        self.conversation_scroll.clone(),
                        move |index, window, cx| {
                            let Some(row) = rows.get(index) else {
                                return div().into_any_element();
                            };
                            app.update(cx, |app, cx| app.render_row(row, window, cx))
                                .unwrap_or_else(|_| div().into_any_element())
                        },
                    )
                    .flex_1()
                    .min_h_0(),
                )
                .when(reading_history, |view| {
                    view.child(
                        Button::new("return-to-latest")
                            .debug_selector(|| "return-to-latest".into())
                            .ghost()
                            .small()
                            .label("Return to latest")
                            .on_click(cx.listener(|app, _, _, cx| {
                                app.conversation_scroll.scroll_to_end();
                                cx.notify();
                            })),
                    )
                })
                .children(self.render_message_gallery(cx));
        }
        let conversation = conversation.overflow_y_scroll().gap_2().p_3();
        if self
            .pending_threads
            .iter()
            .any(|draft| Some(&draft.thread.id) == self.selected_thread.as_ref())
        {
            let phase = self.selected_setup().map(|setup| setup.phase.clone());
            conversation
                .child(match phase.as_deref() {
                    Some("running") => {
                        "Preparing worktree · your first message is waiting for setup"
                    }
                    Some("cancelled") => {
                        "Setup cancelled · your draft and message are retained for retry"
                    }
                    Some("failed") => {
                        "Setup failed · your draft and message are retained for retry"
                    }
                    _ => "New thread draft · send a message to start the conversation",
                })
                .when(phase.is_some(), |view| {
                    view.child(self.composer.read(cx).value().to_string())
                })
        } else if self.selected_thread.is_some() {
            conversation.child(
                div()
                    .text_color(muted_foreground)
                    .child("Loading thread history…"),
            )
        } else {
            conversation.child(
                div()
                    .flex()
                    .flex_col()
                    .items_start()
                    .gap_2()
                    .pt_6()
                    .h(px(360.))
                    .text_color(muted_foreground)
                    .child(
                        div()
                            .text_lg()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(foreground)
                            .child("Select a thread"),
                    )
                    .child("Choose a thread in the sidebar, or start a new one with ⌘N"),
            )
        }
    }

    fn render_timeline_setup(&self, cx: &mut Context<Self>) -> Option<gpui_kit::Div> {
        let setup = self.selected_setup()?;
        let id = setup.thread_id.clone();
        let expanded = self.setup_expanded;
        Some(
            div()
                .flex()
                .flex_col()
                .gap_1()
                .p_2()
                .border_1()
                .border_color(cx.theme().border)
                .child(format!("Worktree setup · {}", setup.phase))
                .children(setup.error.clone())
                .child(
                    Button::new("timeline-setup-details")
                        .ghost()
                        .small()
                        .label(if expanded {
                            "Hide setup details"
                        } else {
                            "Show setup details"
                        })
                        .on_click(cx.listener(|app, _, _, cx| {
                            app.setup_expanded = !app.setup_expanded;
                            cx.notify();
                        })),
                )
                .when(expanded, |view| {
                    view.children(setup.stages.iter().map(|stage| {
                        div()
                            .text_xs()
                            .child(format!("{} · {}", stage.id, stage.status))
                            .children(stage.detail.clone())
                            .children(stage.tail.clone())
                    }))
                })
                .when(setup.phase == "running", |view| {
                    view.child(
                        Button::new("timeline-setup-cancel")
                            .ghost()
                            .small()
                            .label("Cancel setup")
                            .on_click(cx.listener(move |app, _, _, cx| {
                                app.perform(
                                    "worktreeSetup.cancel",
                                    serde_json::json!({"threadId":id}),
                                    None,
                                    cx,
                                )
                            })),
                    )
                }),
        )
    }

    fn render_row(
        &mut self,
        row: &TimelineRow,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let muted_foreground = cx.theme().muted_foreground;
        let danger = cx.theme().danger;
        let message_id = match &row.entry {
            TimelineEntry::Message { message, .. } => message["id"].as_str().map(str::to_owned),
            _ => None,
        };
        let content = match &row.entry {
            TimelineEntry::Message {
                message,
                show_header,
                turn,
            } => {
                let content = self.render_message(message, *show_header, cx);
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .when_some(turn.clone(), |view, turn| {
                        let key = format!("fold:{turn}");
                        let folded = self.activity_expanded.get(&key) == Some(&true);
                        let selector = format!("fold-turn-{turn}");
                        view.child(
                            Button::new(gpui_kit::SharedString::from(selector.clone()))
                                .debug_selector(move || selector.clone())
                                .ghost()
                                .xsmall()
                                .label(if folded { "Expand turn" } else { "Fold turn" })
                                .on_click(cx.listener(move |app, _, _, cx| {
                                    app.conversation_scroll.pause_following_tail();
                                    app.activity_expanded.insert(key.clone(), !folded);
                                    cx.notify();
                                })),
                        )
                    })
                    .child(content)
                    .into_any_element()
            }
            TimelineEntry::TurnStatus {
                label,
                danger: true,
            } => div()
                .text_xs()
                .text_color(danger)
                .child(label.clone())
                .into_any_element(),
            TimelineEntry::TurnStatus { label, .. } | TimelineEntry::Working(label) => div()
                .text_xs()
                .text_color(muted_foreground)
                .child(label.clone())
                .into_any_element(),
            TimelineEntry::Activity {
                turn,
                label,
                items,
                expanded,
                latest,
            } => {
                let selector = if *latest {
                    "activity-toggle".to_owned()
                } else {
                    format!(
                        "activity-toggle-{}",
                        row.key.strip_prefix("activity:").unwrap_or(turn)
                    )
                };
                let expanded = *expanded;
                let key = turn.clone();
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        Button::new(gpui_kit::SharedString::from(selector.clone()))
                            .debug_selector(move || selector.clone())
                            .ghost()
                            .small()
                            .label(label.clone())
                            .on_click(cx.listener(move |app, _, _, cx| {
                                // The disclosure grows in place; do not snap to the tail.
                                app.conversation_scroll.pause_following_tail();
                                app.activity_expanded.insert(key.clone(), !expanded);
                                cx.notify();
                            })),
                    )
                    .when(expanded, |view| {
                        view.child(self.render_activity_log(items, cx))
                    })
                    .into_any_element()
            }
            TimelineEntry::Plan(plan) => self.render_plan(plan, cx).into_any_element(),
            TimelineEntry::Checkpoint {
                turn,
                checkpoint,
                latest,
            } => {
                let selector = if *latest {
                    "revert-checkpoint".to_owned()
                } else {
                    format!("revert-checkpoint-{turn}")
                };
                self.render_checkpoint(checkpoint, selector, cx)
                    .into_any_element()
            }
            TimelineEntry::Approval(approval) => {
                self.render_approval(approval, cx).into_any_element()
            }
            TimelineEntry::UserInput(request) => self
                .render_user_input(request, window, cx)
                .into_any_element(),
            TimelineEntry::Error(error) => div()
                .flex()
                .flex_col()
                .gap_1()
                .p_2()
                .border_1()
                .border_color(danger)
                .text_xs()
                .text_color(danger)
                .child(error.clone())
                .child(
                    Button::new("thread-error-settings")
                        .ghost()
                        .small()
                        .label("Provider / connection settings…")
                        .on_click(cx.listener(|app, _, window, cx| {
                            app.execute_command(
                                t3code_desktop_gpui::palette::Command::Settings,
                                window,
                                cx,
                            );
                        })),
                )
                .when(!self.connection_ready(), |view| {
                    view.child(
                        Button::new("thread-error-reconnect")
                            .ghost()
                            .small()
                            .label("Retry connection")
                            .on_click(cx.listener(|app, _, _, cx| app.retry_connection_now(cx))),
                    )
                })
                .into_any_element(),
        };
        div()
            .id(gpui_kit::SharedString::from(row.key.clone()))
            .w_full()
            .px_3()
            .py_1()
            .when_some(message_id, |row, id| {
                row.debug_selector(move || format!("message-{id}"))
            })
            .child(content)
            .into_any_element()
    }

    fn render_approval(
        &self,
        approval: &PendingApproval,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let muted_foreground = cx.theme().muted_foreground;
        let group_box = cx.theme().group_box;
        let warning = cx.theme().warning;
        let request_id = approval.request_id.clone();
        let options = if approval.options.is_empty() {
            vec![
                ("accept".to_owned(), "Allow once".to_owned()),
                ("decline".to_owned(), "Deny".to_owned()),
            ]
        } else {
            approval.options.clone()
        };
        div()
            .flex()
            .flex_col()
            .gap_2()
            .p_3()
            .rounded_md()
            .bg(group_box)
            .border_1()
            .border_color(warning)
            .child(
                div()
                    .text_xs()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(warning)
                    .child("APPROVAL REQUIRED"),
            )
            .child(div().text_sm().child(approval.summary.clone()))
            .when_some(approval.detail.clone(), |card, detail| {
                card.child(div().text_xs().text_color(muted_foreground).child(detail))
            })
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap_2()
                    .children(options.into_iter().map(|(decision, label)| {
                        let request_id = request_id.clone();
                        let decision_id = decision.clone();
                        let danger = decision == "decline" || decision == "cancel";
                        Button::new(gpui_kit::SharedString::from(format!(
                            "approval-{decision_id}"
                        )))
                        .debug_selector(move || format!("approval-{decision_id}"))
                        .label(label)
                        .ghost()
                        .small()
                        .when(danger, |button| button.danger())
                        .disabled(!self.connection_ready())
                        .on_click(cx.listener(move |app, _, _, cx| {
                            app.respond_approval(&request_id, &decision, cx)
                        }))
                    })),
            )
    }

    fn render_user_input(
        &mut self,
        request: &PendingUserInput,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let muted_foreground = cx.theme().muted_foreground;
        let group_box = cx.theme().group_box;
        let accent = cx.theme().blue;
        let mut card = div()
            .debug_selector(|| "user-input-card".to_owned())
            .flex()
            .flex_col()
            .gap_3()
            .p_3()
            .rounded_md()
            .bg(group_box)
            .border_1()
            .border_color(accent)
            .child(
                div()
                    .text_xs()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(accent)
                    .child("QUESTION"),
            );
        for (question_index, question) in request.questions.iter().enumerate() {
            let custom = question.allow_custom_answer.then(|| {
                self.user_input_custom
                    .entry(question.id.clone())
                    .or_insert_with(|| {
                        let input = cx
                            .new(|cx| single_line_input(window, cx).placeholder("Your own answer"));
                        let question_id = question.id.clone();
                        cx.subscribe_in(&input, window, move |app, _, event, _, cx| {
                            if matches!(event, InputEvent::Change) {
                                app.type_user_input_custom(&question_id, cx);
                            }
                        })
                        .detach();
                        input
                    })
                    .clone()
            });
            // A typed custom answer outranks the chips, so they show unselected.
            let custom_active = custom
                .as_ref()
                .is_some_and(|input| !input.read(cx).value().trim().is_empty());
            let selected = self
                .user_input_selected
                .get(&question.id)
                .filter(|_| !custom_active)
                .cloned()
                .unwrap_or_default();
            let options = question.options.iter().enumerate().map(|(index, option)| {
                let selector = format!("user-input-option-{question_index}-{index}");
                let question_id = question.id.clone();
                let value = option.value.clone();
                let multi_select = question.multi_select;
                Button::new(gpui_kit::SharedString::from(selector.clone()))
                    .debug_selector(move || selector.clone())
                    .ghost()
                    .small()
                    .selected(selected.contains(&option.value))
                    .label(option.label.clone())
                    .when(!option.description.is_empty(), |button| {
                        button.tooltip(option.description.clone())
                    })
                    .on_click(cx.listener(move |app, _, window, cx| {
                        app.select_user_input_option(&question_id, &value, multi_select, window, cx)
                    }))
            });
            card = card.child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .text_xs()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(muted_foreground)
                            .child(question.header.clone()),
                    )
                    .child(div().text_sm().child(question.question.clone()))
                    .when(question.multi_select, |view| {
                        view.child(
                            div()
                                .text_xs()
                                .text_color(muted_foreground)
                                .child("Select one or more options."),
                        )
                    })
                    .child(div().flex().flex_wrap().gap_2().children(options))
                    .when_some(custom, |view, input| {
                        let selector = format!("user-input-custom-{question_index}");
                        view.child(
                            div()
                                .debug_selector(move || selector.clone())
                                .child(Input::new(&input).small()),
                        )
                    }),
            );
        }
        let submitted = request.clone();
        let request_id = request.request_id.clone();
        card.child(
            div()
                .flex()
                .gap_2()
                .child(
                    Button::new("user-input-submit")
                        .disabled(!self.connection_ready())
                        .debug_selector(|| "user-input-submit".to_owned())
                        .small()
                        .label("Submit")
                        .on_click(
                            cx.listener(move |app, _, _, cx| app.submit_user_input(&submitted, cx)),
                        ),
                )
                .when(request.dismissible, |view| {
                    view.child(
                        Button::new("user-input-dismiss")
                            .disabled(!self.connection_ready())
                            .debug_selector(|| "user-input-dismiss".to_owned())
                            .ghost()
                            .small()
                            .label("Dismiss")
                            .on_click(cx.listener(move |app, _, _, cx| {
                                app.dismiss_user_input(&request_id, cx)
                            })),
                    )
                }),
        )
    }

    /// A message row. `show_header` puts the role and time above the first
    /// message of a run; Copy shows while the pointer is over the message.
    fn render_message(
        &mut self,
        message: &Value,
        show_header: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let muted_foreground = cx.theme().muted_foreground;
        let secondary = cx.theme().secondary;
        let message_id = message["id"].as_str().unwrap_or("unknown");
        let id = gpui_kit::SharedString::from(format!("message-{message_id}"));
        let copy_id = format!("copy-{message_id}");
        let text = message["text"].as_str().unwrap_or("").to_owned();
        let copy_text = text.clone();
        let role = message["role"].as_str().unwrap_or("message");
        let label = conversation::role_label(role);
        let created = message["createdAt"]
            .as_str()
            .and_then(conversation::clock_time);
        let streaming = message["streaming"] == true;
        let meta = match (created, streaming) {
            (Some(time), true) => format!("{label} · {time} · streaming"),
            (Some(time), false) => format!("{label} · {time}"),
            (None, true) => format!("{label} · streaming"),
            (None, false) => label.to_owned(),
        };
        let attachments = message["attachments"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        let attachment_row = (!attachments.is_empty()).then(|| {
            div()
                .flex()
                .flex_wrap()
                .gap_1()
                .children(attachments.into_iter().map(|attachment| {
                    let name = attachment["name"].as_str().unwrap_or("attachment");
                    let size = attachment["sizeBytes"].as_u64().unwrap_or(0);
                    let resource = serde_json::json!({"_tag":"attachment", "attachmentId":attachment["id"], "fileName": name, "mimeType": attachment["mimeType"], "disposition":"attachment"});
                    let key = super::message_media::media_key(&self.environment_scope, self.selected_thread.as_deref().unwrap_or(""), attachment["id"].as_str().unwrap_or(""));
                    if attachment["type"] == "image" {
                        self.request_message_image(key.clone(), resource.clone(), attachment["mimeType"].as_str().unwrap_or(""), cx);
                    }
                    let preview = self.conversation_presentation.media.get(&key).cloned();
                    let preview_ready = matches!(preview, Some(super::message_media::MediaPreview::Ready(_)));
                    let expand_key = key.clone();
                    let keyboard_resource = resource.clone();
                    let expand_resource = resource.clone();
                    let mut inline_resource = resource.clone();
                    inline_resource["disposition"] = serde_json::json!("inline");
                    div().flex().flex_col().gap_1()
                        .when_some(preview, |view, preview| match preview {
                            super::message_media::MediaPreview::Ready(image) => view.child(
                                div().id(gpui_kit::SharedString::from(format!("image-{key}")))
                                    .debug_selector({ let selector = format!("image-{key}"); move || selector.clone() })
                                    .child(gpui_kit::img(image).w(px(160.)).h(px(100.)).object_fit(gpui_kit::ObjectFit::Contain))
                                    .on_click(cx.listener(move |app, _, window, cx| app.expand_message_image(key.clone(), expand_resource.clone(), window, cx)))
                            ),
                            super::message_media::MediaPreview::Loading(_) => view.child("Loading preview…"),
                            super::message_media::MediaPreview::Failed => view.child("Preview unavailable · download below"),
                        })
                        .child(Button::new(gpui_kit::SharedString::from(format!("attachment-{}", attachment["id"].as_str().unwrap_or("unknown"))))
                            .debug_selector({ let selector = format!("attachment-{}", attachment["id"].as_str().unwrap_or("unknown")); move || selector.clone() })
                            .ghost().small().label(format!("{name} · {size} B · Download"))
                            .on_click(cx.listener(move |app, _, _, cx| app.open_conversation_asset(resource.clone(), cx))))
                        .when(preview_ready, |view| view.child(Button::new("expand-image").ghost().xsmall().label("Expand image")
                            .on_click(cx.listener(move |app, _, window, cx| app.expand_message_image(expand_key.clone(), keyboard_resource.clone(), window, cx)))))
                        .child(Button::new(gpui_kit::SharedString::from(format!("attachment-open-{}", attachment["id"].as_str().unwrap_or("unknown"))))
                            .ghost().xsmall().label("Open in browser")
                            .on_click(cx.listener(move |app, _, _, cx| app.open_conversation_asset(inline_resource.clone(), cx))))
                }))
        });
        let copy = div()
            .debug_selector({
                let copy_id = copy_id.clone();
                move || copy_id
            })
            .invisible()
            .group_hover("message", |style| style.visible())
            .child(
                Button::new(gpui_kit::SharedString::from(copy_id.clone()))
                    .label(
                        if self.conversation_presentation.copied.as_ref() == Some(&copy_id) {
                            "Copied"
                        } else {
                            "Copy"
                        },
                    )
                    .ghost()
                    .xsmall()
                    .on_click(cx.listener(move |app, _, _, cx| {
                        app.copy_conversation_text(copy_id.clone(), copy_text.clone(), cx);
                    })),
            );
        let user = role == "user";
        let reasoning = role == "reasoning";
        let expandable = reasoning || (user && text.chars().count() > 2000);
        let disclosure_key = format!("message:{message_id}");
        let expanded = self.activity_expanded.get(&disclosure_key) == Some(&true);
        let displayed = if reasoning && !expanded {
            String::new()
        } else if user && expandable && !expanded {
            format!("{}…", text.chars().take(2000).collect::<String>())
        } else {
            text.clone()
        };
        let (displayed, workspace_images) =
            crate::conversation_presentation::workspace_markdown_images(&displayed);
        let workspace_media = div().flex().flex_wrap().gap_2().children(workspace_images.into_iter().map(|(alt, path)| {
            let resource = serde_json::json!({"_tag":"media-file", "threadId": self.selected_thread, "path":path});
            let key = super::message_media::media_key(&self.environment_scope, self.selected_thread.as_deref().unwrap_or(""), &format!("workspace:{path}"));
            self.request_message_image(key.clone(), resource.clone(), crate::attachment_mime(&path), cx);
            let image = self.conversation_presentation.media.get(&key).and_then(|preview| match preview { super::message_media::MediaPreview::Ready(image) => Some(image.clone()), _ => None });
            let open_resource = resource.clone();
            div().flex().flex_col().gap_1()
                .when_some(image, |view, image| view.child(div().id(gpui_kit::SharedString::from(format!("workspace-image-{key}")))
                    .child(gpui_kit::img(image).w(px(160.)).h(px(100.)).object_fit(gpui_kit::ObjectFit::Contain))
                    .on_click(cx.listener(move |app, _, window, cx| app.expand_message_image(key.clone(), resource.clone(), window, cx)))))
                .child(Button::new(gpui_kit::SharedString::from(format!("workspace-image-open-{path}"))).ghost().xsmall()
                    .label(format!("Open image · {}", if alt.is_empty() { path } else { alt }))
                    .on_click(cx.listener(move |app, _, _, cx| app.open_conversation_asset(open_resource.clone(), cx))))
        }));
        let context = message["context"]["records"]
            .as_array()
            .map_or(&[][..], Vec::as_slice);
        let body = div()
            .flex()
            .flex_col()
            .gap_1()
            .min_w_0()
            .when_some(attachment_row, |view, row| view.child(row))
            .when(!context.is_empty(), |view| {
                view.child(
                    div()
                        .flex()
                        .flex_wrap()
                        .gap_1()
                        .children(context.iter().map(|record| {
                            let kind = record["kind"].as_str().unwrap_or("context");
                            let label = record["label"]
                                .as_str()
                                .or_else(|| record["name"].as_str())
                                .or_else(|| record["path"].as_str())
                                .unwrap_or("Unavailable context");
                            let detail = serde_json::to_string_pretty(record).unwrap_or_default();
                            let key = format!(
                                "context:{}",
                                record["contextId"].as_str().unwrap_or(label)
                            );
                            Button::new(gpui_kit::SharedString::from(key.clone()))
                                .ghost()
                                .xsmall()
                                .label(format!("{kind} · {label}"))
                                .tooltip(detail.clone())
                                .on_click(cx.listener(move |app, _, _, cx| {
                                    app.copy_conversation_text(key.clone(), detail.clone(), cx)
                                }))
                        })),
                )
            })
            .when(expandable, |view| {
                view.child(
                    Button::new(gpui_kit::SharedString::from(format!("expand-{message_id}")))
                        .debug_selector({
                            let id = format!("expand-{message_id}");
                            move || id.clone()
                        })
                        .ghost()
                        .xsmall()
                        .label(if reasoning {
                            if expanded {
                                "Hide reasoning"
                            } else {
                                "Show reasoning"
                            }
                        } else if expanded {
                            "Show less"
                        } else {
                            "Show more"
                        })
                        .on_click(cx.listener(move |app, _, _, cx| {
                            app.conversation_scroll.pause_following_tail();
                            app.activity_expanded
                                .insert(disclosure_key.clone(), !expanded);
                            cx.notify();
                        })),
                )
            })
            .when(!reasoning || expanded, |view| {
                let view = view.child(workspace_media);
                let app = cx.weak_entity();
                let link_app = app.clone();
                let wrap_id = message_id.to_owned();
                let wrapped = self
                    .conversation_presentation
                    .code_wrap
                    .get(message_id)
                    .copied()
                    .unwrap_or(true);
                let mut style = gpui_kit::component::text::TextViewStyle::default();
                style.code_block.text.white_space = Some(if wrapped {
                    gpui_kit::WhiteSpace::Normal
                } else {
                    gpui_kit::WhiteSpace::Nowrap
                });
                style.code_block.overflow.x = Some(gpui_kit::Overflow::Scroll);
                view.child(
                    TextView::markdown(id, displayed)
                        .selectable(true)
                        .style(style)
                        .on_link_click(move |href, _, window, cx| {
                            let _ = link_app
                                .update(cx, |app, cx| app.open_message_link(href, window, cx));
                        })
                        .code_block_actions(move |block, _, cx| {
                            let code = block.code().to_string();
                            let key = format!("code:{code}");
                            let copied = app.upgrade().is_some_and(|entity| {
                                entity.read(cx).conversation_presentation.copied.as_ref()
                                    == Some(&key)
                            });
                            let wrap_app = app.clone();
                            let wrap_id = wrap_id.clone();
                            let app = app.clone();
                            div()
                                .flex()
                                .items_center()
                                .gap_2()
                                .child(block.lang().unwrap_or_else(|| "Code".into()))
                                .child(
                                    Button::new("code-wrap")
                                        .debug_selector(|| "code-wrap".into())
                                        .ghost()
                                        .xsmall()
                                        .label(if wrapped { "No wrap" } else { "Wrap lines" })
                                        .on_click(move |_, _, cx| {
                                            let _ = wrap_app.update(cx, |app, cx| {
                                                app.conversation_presentation
                                                    .code_wrap
                                                    .insert(wrap_id.clone(), !wrapped);
                                                if let Some(index) =
                                                    app.conversation_rows.iter().position(|row| {
                                                        row.key == format!("message:{wrap_id}")
                                                    })
                                                {
                                                    app.conversation_scroll
                                                        .remeasure_items(index..index + 1);
                                                }
                                                cx.notify();
                                            });
                                        }),
                                )
                                .child(
                                    Button::new("copy-code")
                                        .ghost()
                                        .xsmall()
                                        .label(if copied { "Copied" } else { "Copy code" })
                                        .on_click(move |_, _, cx| {
                                            let _ = app.update(cx, |app, cx| {
                                                app.copy_conversation_text(
                                                    key.clone(),
                                                    code.clone(),
                                                    cx,
                                                )
                                            });
                                        }),
                                )
                        }),
                )
            })
            .map(|body| match role {
                "user" => body
                    .max_w(px(720.))
                    .px_3()
                    .py_2()
                    .rounded_lg()
                    .bg(secondary),
                "reasoning" | "system" => body.text_color(muted_foreground),
                _ => body,
            });
        div()
            .group("message")
            .flex()
            .flex_col()
            .gap_1()
            .when(user, |view| view.items_end())
            .when(show_header, |view| {
                view.child(div().text_xs().text_color(muted_foreground).child(meta))
            })
            .child(body)
            .child(copy)
    }

    fn render_activity_log(&self, items: &[ActivityItem], cx: &mut Context<Self>) -> gpui_kit::Div {
        let muted_foreground = cx.theme().muted_foreground;
        let warning = cx.theme().warning;
        let blue = cx.theme().blue;
        let danger = cx.theme().danger;
        let mut log = div().flex().flex_col().gap_1();
        for item in items {
            let (activity, text_color) = match item {
                ActivityItem::Tool(call) => {
                    log = log.child(self.render_tool_call(call, cx));
                    continue;
                }
                ActivityItem::Activity(activity) => (activity, muted_foreground),
                ActivityItem::Error(activity) => (activity, danger),
            };
            let color = match activity["tone"].as_str().unwrap_or("info") {
                "error" => danger,
                "approval" => warning,
                "tool" => muted_foreground,
                _ => blue,
            };
            let selector = format!("activity-{}", activity["id"].as_str().unwrap_or("unknown"));
            log =
                log.child(
                    div()
                        .debug_selector(move || selector)
                        .flex()
                        .items_center()
                        .gap_2()
                        .text_xs()
                        .text_color(text_color)
                        .child(div().w(px(6.)).h(px(6.)).rounded_full().bg(color))
                        .child(activity["summary"].as_str().unwrap_or("").to_owned())
                        .when(
                            activity["kind"].as_str().is_some_and(|kind| {
                                kind.contains("agent") || kind.contains("spawn")
                            }),
                            |view| {
                                view.child(
                                    Button::new(gpui_kit::SharedString::from(format!(
                                        "spawn-agents-{}",
                                        activity["id"].as_str().unwrap_or("unknown")
                                    )))
                                    .icon(IconName::User)
                                    .ghost()
                                    .xsmall()
                                    .label("Agents")
                                    .on_click(cx.listener(|app, _, _, cx| {
                                        app.inspector_open = true;
                                        app.right_tab = crate::RightTab::Agents;
                                        cx.notify();
                                    })),
                                )
                            },
                        ),
                );
        }
        log
    }

    /// One tool call: state, name, key argument, and a chevron that opens its body.
    fn render_tool_call(&self, call: &ToolCall, cx: &mut Context<Self>) -> gpui_kit::Div {
        let theme = cx.theme();
        let (marker, marker_color) = match call.state {
            ToolState::Running => (IconName::Circle, theme.blue),
            ToolState::Completed => (IconName::Check, theme.green),
            ToolState::Failed => (IconName::CircleX, theme.danger),
        };
        let (foreground, muted_foreground, green, danger) = (
            theme.foreground,
            theme.muted_foreground,
            theme.green,
            theme.danger,
        );
        let (code_background, mono_font) = (theme.group_box, theme.mono_font_family.clone());
        let id = call.key.clone();
        let row_selector = format!("tool-row-{id}");
        let name_selector = format!("tool-name-{id}");
        let toggle_selector = format!("tool-toggle-{id}");
        let body_selector = format!("tool-body-{id}");
        let key = call.expansion_key();
        let expanded = call.expanded;
        let row = div()
            .debug_selector(move || row_selector)
            .flex()
            .items_center()
            .gap_2()
            .min_w_0()
            .text_xs()
            .child(Icon::new(marker).xsmall().text_color(marker_color))
            .child(
                Icon::new(match conversation::tool_family(&call.name) {
                    conversation::ToolFamily::Command => IconName::SquareTerminal,
                    conversation::ToolFamily::Search => IconName::Search,
                    conversation::ToolFamily::Edit => IconName::File,
                    conversation::ToolFamily::Agent => IconName::User,
                    conversation::ToolFamily::Other => IconName::Circle,
                })
                .xsmall()
                .text_color(muted_foreground),
            )
            .child(
                div()
                    .debug_selector(move || name_selector)
                    .flex_shrink_0()
                    .text_color(foreground)
                    .child(call.name.clone()),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_color(muted_foreground)
                    .child(call.headline.clone().unwrap_or_default()),
            )
            .when_some(call.line_counts, |row, (additions, deletions)| {
                row.child(div().text_color(green).child(format!("+{additions}")))
                    .child(div().text_color(danger).child(format!("-{deletions}")))
            })
            .when(call.body.is_some(), |row| {
                row.child(
                    Button::new(gpui_kit::SharedString::from(toggle_selector.clone()))
                        .debug_selector(move || toggle_selector.clone())
                        .icon(if expanded {
                            IconName::ChevronDown
                        } else {
                            IconName::ChevronRight
                        })
                        .ghost()
                        .xsmall()
                        .on_click(cx.listener(move |app, _, _, cx| {
                            // The row grows in place; do not snap to the tail.
                            app.conversation_scroll.pause_following_tail();
                            app.activity_expanded.insert(key.clone(), !expanded);
                            cx.notify();
                        })),
                )
            });
        div().flex().flex_col().gap_1().child(row).when_some(
            call.body.clone().filter(|_| expanded),
            |view, body| {
                view.child(
                    div()
                        .id(gpui_kit::SharedString::from(body_selector.clone()))
                        .debug_selector(move || body_selector.clone())
                        .max_h(px(240.))
                        .overflow_y_scroll()
                        .p_2()
                        .rounded_md()
                        .bg(code_background)
                        .font_family(mono_font)
                        .text_xs()
                        .text_color(muted_foreground)
                        .child(body),
                )
            },
        )
    }

    pub(crate) fn confirmed_plan_followup_mode(
        &self,
        text: &str,
    ) -> Result<Option<String>, String> {
        let Some(record) = self
            .conversation_presentation
            .plan_followup
            .as_ref()
            .filter(|record| {
                record.generation == self.generation
                    && record.environment == self.environment_scope
                    && self.selected_thread.as_deref() == Some(record.thread.as_str())
                    && (text == record.prompt
                        || record
                            .prompt
                            .split_once("\n\n")
                            .is_some_and(|(heading, _)| {
                                text.starts_with(&format!("{heading}\n\n"))
                            }))
            })
        else {
            return Ok(None);
        };
        match record.accepted {
            Some(true) => Ok(Some(record.mode.clone())),
            Some(false) => Err("Plan mode change was rejected; prepare the follow-up again".into()),
            None => Err("Wait for the plan mode receipt before sending".into()),
        }
    }

    pub(crate) fn plan_followup_source(&self, text: &str) -> Option<Value> {
        let mode = self.confirmed_plan_followup_mode(text).ok().flatten()?;
        let record = self.conversation_presentation.plan_followup.as_ref()?;
        (mode == "default" && text == record.prompt)
            .then(|| serde_json::json!({"threadId":record.thread,"planId":record.plan}))
    }

    fn prepare_plan_followup(
        &mut self,
        plan_id: String,
        markdown: String,
        implement: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.connection_ready() {
            self.report_error("Reconnect before preparing a plan follow-up", cx);
            return;
        }
        if self.checkout_transition_pending()
            || !self.selected_provider(cx).is_some_and(|provider| {
                provider.selectable() && provider.show_interaction_mode_toggle != Some(false)
            })
        {
            self.report_error("Plan follow-up requires an available provider with interaction modes and a reconciled checkout", cx);
            return;
        }
        if !self.composer.read(cx).value().trim().is_empty() || !self.attachments.is_empty() {
            self.report_error("Stash or clear the current prompt before using a plan", cx);
            return;
        }
        if self.pending_actions > 0 {
            self.report_progress("Wait for the pending server receipt", cx);
            return;
        }
        let Some(sender) = self.commands.clone() else {
            return;
        };
        let Some(thread) = self.selected_thread.clone() else {
            return;
        };
        let mode = if implement { "default" } else { "plan" };
        let generation = self.generation;
        let environment = self.environment_scope.clone();
        let prompt = format!(
            "{}\n\n{markdown}",
            if implement {
                "Implement the following plan:"
            } else {
                "Refine the following plan:"
            }
        );
        self.conversation_presentation.plan_followup =
            Some(crate::conversation_presentation::PlanFollowup {
                generation,
                environment: environment.clone(),
                thread: thread.clone(),
                plan: plan_id,
                prompt: prompt.clone(),
                mode: mode.into(),
                accepted: None,
            });
        self.pending_actions += 1;
        let payload = t3code_desktop_gpui::commands::set_thread_mode(
            "thread.interaction-mode.set",
            &thread,
            "interactionMode",
            mode,
        );
        cx.spawn(async move |entity, cx| {
            let result = t3code_desktop_gpui::connection::call(
                &sender,
                "orchestration.dispatchCommand",
                payload,
            )
            .await;
            let _ = entity.update(cx, |app, cx| {
                if app.generation != generation || app.environment_scope != environment {
                    return;
                }
                app.pending_actions = app.pending_actions.saturating_sub(1);
                if let Some(record) = app.conversation_presentation.plan_followup.as_mut()
                    && record.generation == generation
                    && record.thread == thread
                    && record.mode == mode
                {
                    record.accepted = Some(result.is_ok());
                }
                match result {
                    Ok(_) => app.report_success("Plan mode confirmed · review and Send", cx),
                    Err(error) => app.report_error(format!("Plan mode rejected: {error}"), cx),
                }
                cx.notify();
            });
        })
        .detach();
        self.composer.update(cx, |input, cx| {
            input.set_value(prompt, window, cx);
            input.focus(window, cx);
        });
        self.remember_composer_draft(cx);
        self.report_success(
            "Plan follow-up prepared · review and Send after the mode receipt",
            cx,
        );
    }

    fn export_conversation_plan(&mut self, contents: String, cx: &mut Context<Self>) {
        let directory = std::env::var_os("HOME")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| ".".into());
        let picker = cx.prompt_for_new_path(&directory, Some("plan.md"));
        cx.spawn(async move |entity, cx| {
            let path = match picker.await {
                Ok(Ok(Some(path))) => path,
                Ok(Ok(None)) => return,
                _ => {
                    let _ = entity.update(cx, |app, cx| {
                        app.report_error("Could not open the save dialog", cx)
                    });
                    return;
                }
            };
            let result = cx
                .background_executor()
                .spawn(async move { std::fs::write(path, contents) })
                .await;
            let _ = entity.update(cx, |app, cx| match result {
                Ok(()) => app.report_success("Plan exported", cx),
                Err(_) => app.report_error("Could not export plan; choose a writable location", cx),
            });
        })
        .detach();
    }

    fn save_conversation_plan(&mut self, contents: String, cx: &mut Context<Self>) {
        if !self.connection_ready() {
            self.report_error("Reconnect before saving a plan", cx);
            return;
        }
        if self.conversation_presentation.saving {
            return;
        }
        let (Some(sender), Some(cwd)) = (self.commands.clone(), self.selected_workspace()) else {
            self.report_error("Connect and select a workspace to save a plan", cx);
            return;
        };
        // A fresh path avoids overwriting a user's existing workspace file.
        let path = format!("plan-{}.md", uuid::Uuid::new_v4());
        let generation = self.generation;
        let origin = self.environment_scope.clone();
        let thread = self.selected_thread.clone();
        self.conversation_presentation.saving = true;
        cx.notify();
        cx.spawn(async move |entity, cx| {
            let result = t3code_desktop_gpui::connection::call(
                &sender,
                "projects.writeFile",
                serde_json::json!({"cwd":cwd,"relativePath":path,"contents":contents}),
            )
            .await;
            let _ = entity.update(cx, |app, cx| {
                app.conversation_presentation.saving = false;
                if app.generation == generation
                    && app.environment_scope == origin
                    && app.selected_thread == thread
                {
                    match result {
                        Ok(_) => app.report_success(&format!("Plan saved to {path}"), cx),
                        Err(error) => app.report_error(format!("Could not save plan: {error}"), cx),
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn render_plan(&self, plan: &Value, cx: &Context<Self>) -> gpui_kit::Div {
        let markdown = plan["planMarkdown"].as_str().unwrap_or("").to_owned();
        let muted_foreground = cx.theme().muted_foreground;
        let border = cx.theme().border;
        let group_box = cx.theme().group_box;
        let muted = cx.theme().muted;
        let id = gpui_kit::SharedString::from(format!(
            "plan-{}",
            plan["id"].as_str().unwrap_or("unknown")
        ));
        let implemented = !plan["implementedAt"].is_null();
        let can_followup = self.connection_ready()
            && self.pending_actions == 0
            && !self.checkout_transition_pending()
            && self.selected_provider(cx).is_some_and(|provider| {
                provider.selectable() && provider.show_interaction_mode_toggle != Some(false)
            });
        let implement_plan = plan["id"].as_str().unwrap_or("unknown").to_owned();
        let refine_plan = implement_plan.clone();
        let key = format!("plan:{}", plan["id"].as_str().unwrap_or("unknown"));
        let expanded = self.activity_expanded.get(&key) == Some(&true);
        let can_collapse = markdown.chars().count() > 900 || markdown.lines().count() > 20;
        let displayed = if can_collapse && !expanded {
            markdown
                .lines()
                .take(10)
                .collect::<Vec<_>>()
                .join("\n")
                .chars()
                .take(900)
                .collect::<String>()
        } else {
            markdown.clone()
        };
        let copy_key = key.clone();
        let copy_text = markdown.clone();
        let save_text = markdown.clone();
        let export_text = markdown.clone();
        let implement_text = markdown.clone();
        let refine_text = markdown;
        div()
            .flex()
            .flex_col()
            .gap_1()
            .p_3()
            .rounded_md()
            .bg(group_box)
            .border_1()
            .border_color(border)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .text_xs()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(muted_foreground)
                    .child("PROPOSED PLAN")
                    .when(implemented, |view| {
                        view.child(div().px_2().rounded_sm().bg(muted).child("implemented"))
                    }),
            )
            .child(
                TextView::markdown(
                    id,
                    crate::conversation_presentation::workspace_markdown_images(&displayed).0,
                )
                .on_link_click({
                    let app = cx.weak_entity();
                    move |href, _, window, cx| {
                        let _ = app.update(cx, |app, cx| app.open_message_link(href, window, cx));
                    }
                }),
            )
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap_1()
                    .when(can_collapse, |view| {
                        view.child(
                            Button::new("plan-expand")
                                .ghost()
                                .small()
                                .label(if expanded {
                                    "Collapse plan"
                                } else {
                                    "Expand plan"
                                })
                                .on_click(cx.listener(move |app, _, _, cx| {
                                    app.conversation_scroll.pause_following_tail();
                                    app.activity_expanded.insert(key.clone(), !expanded);
                                    cx.notify();
                                })),
                        )
                    })
                    .child(
                        Button::new("plan-copy")
                            .ghost()
                            .small()
                            .label(
                                if self.conversation_presentation.copied.as_ref() == Some(&copy_key)
                                {
                                    "Copied"
                                } else {
                                    "Copy plan"
                                },
                            )
                            .on_click(cx.listener(move |app, _, _, cx| {
                                app.copy_conversation_text(copy_key.clone(), copy_text.clone(), cx)
                            })),
                    )
                    .child(
                        Button::new("plan-export")
                            .ghost()
                            .small()
                            .label("Download markdown…")
                            .on_click(cx.listener(move |app, _, _, cx| {
                                app.export_conversation_plan(export_text.clone(), cx)
                            })),
                    )
                    .child(
                        Button::new("plan-save")
                            .debug_selector(|| "plan-save".into())
                            .ghost()
                            .small()
                            .label(if self.conversation_presentation.saving {
                                "Saving…"
                            } else {
                                "Save new workspace file"
                            })
                            .disabled(
                                self.conversation_presentation.saving
                                    || !self.connection_ready()
                                    || self.selected_workspace().is_none(),
                            )
                            .on_click(cx.listener(move |app, _, _, cx| {
                                app.save_conversation_plan(save_text.clone(), cx)
                            })),
                    )
                    .when(!implemented, |view| {
                        view.child(
                            Button::new("plan-implement")
                                .disabled(!self.connection_ready())
                                .debug_selector(|| "plan-implement".into())
                                .ghost()
                                .small()
                                .label("Implement…")
                                .disabled(!can_followup)
                                .on_click(cx.listener(move |app, _, window, cx| {
                                    app.prepare_plan_followup(
                                        implement_plan.clone(),
                                        implement_text.clone(),
                                        true,
                                        window,
                                        cx,
                                    )
                                })),
                        )
                        .child(
                            Button::new("plan-refine")
                                .disabled(!self.connection_ready())
                                .debug_selector(|| "plan-refine".into())
                                .ghost()
                                .small()
                                .label("Refine…")
                                .disabled(!can_followup)
                                .on_click(cx.listener(move |app, _, window, cx| {
                                    app.prepare_plan_followup(
                                        refine_plan.clone(),
                                        refine_text.clone(),
                                        false,
                                        window,
                                        cx,
                                    )
                                })),
                        )
                    }),
            )
    }

    fn render_checkpoint(
        &self,
        checkpoint: &Value,
        selector: String,
        cx: &mut Context<Self>,
    ) -> gpui_kit::Div {
        let files = checkpoint["files"]
            .as_array()
            .map_or(&[][..], Vec::as_slice);
        let muted_foreground = cx.theme().muted_foreground;
        let border = cx.theme().border;
        let group_box = cx.theme().group_box;
        let green = cx.theme().green;
        let danger = cx.theme().danger;
        let (additions, deletions) = conversation::diff_totals(files);
        let turn_count = checkpoint["checkpointTurnCount"].as_u64().unwrap_or(0);
        let confirm = self.confirm_revert == Some(turn_count);
        let mut card = div()
            .flex()
            .flex_col()
            .gap_1()
            .p_3()
            .rounded_md()
            .bg(group_box)
            .border_1()
            .border_color(border)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .text_xs()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(muted_foreground)
                    .child("CHANGES")
                    .child(format!("{} files", files.len()))
                    .child(div().text_color(green).child(format!("+{additions}")))
                    .child(div().text_color(danger).child(format!("-{deletions}"))),
            );
        for file in files.iter().take(8) {
            card = card.child(
                div()
                    .flex()
                    .justify_between()
                    .gap_2()
                    .text_xs()
                    .text_color(muted_foreground)
                    .child(
                        div()
                            .min_w_0()
                            .truncate()
                            .child(file["path"].as_str().unwrap_or("").to_owned()),
                    )
                    .child(format!(
                        "+{} -{}",
                        file["additions"].as_u64().unwrap_or(0),
                        file["deletions"].as_u64().unwrap_or(0)
                    )),
            );
        }
        card.child(
            div()
                .flex()
                .justify_end()
                .gap_2()
                .pt_1()
                .child(
                    Button::new(gpui_kit::SharedString::from(format!(
                        "view-checkpoint-{turn_count}"
                    )))
                    .debug_selector(move || format!("view-checkpoint-{turn_count}"))
                    .label("View changes")
                    .ghost()
                    .small()
                    .on_click(
                        cx.listener(move |app, _, _, cx| app.view_checkpoint_diff(turn_count, cx)),
                    ),
                )
                .child(
                    Button::new(gpui_kit::SharedString::from(selector.clone()))
                        .debug_selector(move || selector.clone())
                        .label(if confirm {
                            "Confirm revert"
                        } else {
                            "Revert to here"
                        })
                        .ghost()
                        .small()
                        .disabled(!self.connection_ready())
                        .on_click(
                            cx.listener(move |app, _, _, cx| app.revert_checkpoint(turn_count, cx)),
                        ),
                ),
        )
    }
}
