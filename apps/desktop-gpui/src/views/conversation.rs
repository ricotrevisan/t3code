use std::rc::Rc;

use gpui_kit::component::{
    ActiveTheme as _, Sizable,
    button::{Button, ButtonVariants as _},
    text::TextView,
};
use gpui_kit::{AnyElement, Context, FontWeight, div, list, prelude::*, px};
use serde_json::Value;
use t3code_desktop_gpui::conversation::{self, PendingApproval, TimelineEntry, TimelineRow};

use crate::NativeApp;

impl NativeApp {
    pub(crate) fn render_conversation(
        &mut self,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let foreground = cx.theme().foreground;
        let muted_foreground = cx.theme().muted_foreground;
        let conversation = div()
            .id("conversation")
            .min_h(px(140.))
            .flex_shrink_0()
            .flex()
            .flex_col()
            .flex_1();
        if let Some(detail) = &self.detail.thread {
            let rows = conversation::timeline_rows(detail, &self.activity_expanded);
            let diff = conversation::diff_rows(&self.conversation_rows, &rows);
            if !diff.replaced.is_empty() || diff.inserted > 0 {
                self.conversation_scroll
                    .splice(diff.replaced, diff.inserted);
            }
            for index in diff.changed {
                self.conversation_scroll.remeasure_items(index..index + 1);
            }
            self.conversation_rows = Rc::new(rows);
            // Only rows near the viewport are built; each frame renders them afresh.
            let rows = self.conversation_rows.clone();
            let app = cx.weak_entity();
            return conversation.py_2().child(
                list(self.conversation_scroll.clone(), move |index, _, cx| {
                    let Some(row) = rows.get(index) else {
                        return div().into_any_element();
                    };
                    app.update(cx, |app, cx| app.render_row(row, cx))
                        .unwrap_or_else(|_| div().into_any_element())
                })
                .flex_1()
                .min_h_0(),
            );
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

    fn render_row(&self, row: &TimelineRow, cx: &mut Context<Self>) -> AnyElement {
        let muted_foreground = cx.theme().muted_foreground;
        let danger = cx.theme().danger;
        let message_id = match &row.entry {
            TimelineEntry::Message(message) => message["id"].as_str().map(str::to_owned),
            _ => None,
        };
        let content = match &row.entry {
            TimelineEntry::Message(message) => self.render_message(message, cx).into_any_element(),
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
                activities,
                expanded,
                latest,
            } => {
                let selector = if *latest {
                    "activity-toggle".to_owned()
                } else {
                    format!("activity-toggle-{turn}")
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
                        view.child(self.render_activity_log(activities, cx))
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
            TimelineEntry::Error(error) => div()
                .text_xs()
                .text_color(danger)
                .child(error.clone())
                .into_any_element(),
        };
        div()
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
                        .on_click(cx.listener(move |app, _, _, cx| {
                            app.respond_approval(&request_id, &decision, cx)
                        }))
                    })),
            )
    }

    fn render_message(&self, message: &Value, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let muted_foreground = cx.theme().muted_foreground;
        let group_box = cx.theme().group_box;
        let muted = cx.theme().muted;
        let secondary = cx.theme().secondary;
        let id = gpui_kit::SharedString::from(format!(
            "message-{}",
            message["id"].as_str().unwrap_or("unknown")
        ));
        let text = message["text"].as_str().unwrap_or("").to_owned();
        let copy_text = text.clone();
        let role = message["role"].as_str().unwrap_or("message");
        let label = conversation::role_label(role);
        let created = message["createdAt"]
            .as_str()
            .and_then(conversation::clock_time);
        let meta = match (created, message["streaming"] == true) {
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
                    div()
                        .px_2()
                        .rounded_sm()
                        .bg(muted)
                        .text_xs()
                        .text_color(muted_foreground)
                        .child(format!("{name} · {size} B"))
                }))
        });
        let body = div()
            .flex()
            .flex_col()
            .gap_1()
            .p_2()
            .rounded_md()
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .child(div().text_xs().text_color(muted_foreground).child(meta))
                    .child(
                        Button::new(gpui_kit::SharedString::from(format!(
                            "copy-{}",
                            message["id"].as_str().unwrap_or("unknown")
                        )))
                        .label("Copy")
                        .ghost()
                        .small()
                        .on_click(cx.listener(move |_, _, _, cx| {
                            cx.write_to_clipboard(gpui_kit::ClipboardItem::new_string(
                                copy_text.clone(),
                            ))
                        })),
                    ),
            )
            .when_some(attachment_row, |view, row| view.child(row))
            .child(TextView::markdown(id, text));
        match role {
            "user" => div()
                .flex()
                .justify_end()
                .child(body.max_w(px(720.)).bg(secondary)),
            "reasoning" => div()
                .flex()
                .justify_center()
                .child(body.max_w(px(720.)).bg(muted)),
            "system" => div()
                .flex()
                .justify_center()
                .child(body.text_color(muted_foreground)),
            _ => body.bg(group_box),
        }
    }

    fn render_activity_log(&self, activities: &[Value], cx: &Context<Self>) -> gpui_kit::Div {
        let muted_foreground = cx.theme().muted_foreground;
        let warning = cx.theme().warning;
        let blue = cx.theme().blue;
        let danger = cx.theme().danger;
        let mut log = div().flex().flex_col().gap_1();
        for activity in activities {
            let color = match activity["tone"].as_str().unwrap_or("info") {
                "error" => danger,
                "approval" => warning,
                "tool" => muted_foreground,
                _ => blue,
            };
            log = log.child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .text_xs()
                    .text_color(muted_foreground)
                    .child(div().w(px(6.)).h(px(6.)).rounded_full().bg(color))
                    .child(activity["summary"].as_str().unwrap_or("").to_owned()),
            );
        }
        log
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
            .child(TextView::markdown(id, markdown))
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
            div().flex().justify_end().pt_1().child(
                Button::new(gpui_kit::SharedString::from(selector.clone()))
                    .debug_selector(move || selector.clone())
                    .label(if confirm {
                        "Confirm revert"
                    } else {
                        "Revert to here"
                    })
                    .ghost()
                    .small()
                    .on_click(
                        cx.listener(move |app, _, _, cx| app.revert_checkpoint(turn_count, cx)),
                    ),
            ),
        )
    }
}
