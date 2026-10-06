use gpui_kit::component::{
    ActiveTheme as _, Sizable,
    button::{Button, ButtonVariants as _},
    text::TextView,
};
use gpui_kit::{Context, FontWeight, div, prelude::*, px};
use serde_json::Value;
use t3code_desktop_gpui::conversation;

use crate::NativeApp;

impl NativeApp {
    pub(crate) fn render_conversation(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let foreground = cx.theme().foreground;
        let muted_foreground = cx.theme().muted_foreground;
        let border = cx.theme().border;
        let group_box = cx.theme().group_box;
        let muted = cx.theme().muted;
        let secondary = cx.theme().secondary;
        let warning = cx.theme().warning;
        let blue = cx.theme().blue;
        let green = cx.theme().green;
        let danger = cx.theme().danger;
        let mut conversation = div()
            .id("conversation")
            .min_h(px(140.))
            .flex_shrink_0()
            .flex()
            .flex_col()
            .flex_1()
            .overflow_y_scroll()
            .track_scroll(&self.conversation_scroll)
            .gap_2()
            .p_3();
        if let Some(detail) = &self.detail.thread {
            if let Some((label, state)) = conversation::turn_summary(&detail["latestTurn"]) {
                let color = if state.is_danger() {
                    danger
                } else {
                    muted_foreground
                };
                conversation = conversation.child(div().text_xs().text_color(color).child(label));
            }

            if let Some(messages) = detail["messages"].as_array() {
                for message in messages {
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
                    let row = match role {
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
                    };
                    conversation = conversation.child(row);
                }
            }

            if let Some(activities) = detail["activities"].as_array() {
                let visible: Vec<&Value> = activities
                    .iter()
                    .filter(|activity| activity["kind"] != "worktree-setup")
                    .collect();
                if !visible.is_empty() {
                    conversation = conversation.child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(
                                Button::new("activity-toggle")
                                    .debug_selector(|| "activity-toggle".into())
                                    .ghost()
                                    .small()
                                    .label(format!("{} activity steps", visible.len()))
                                    .on_click(cx.listener(|app, _, _, cx| {
                                        app.activity_expanded = !app.activity_expanded;
                                        cx.notify();
                                    })),
                            )
                            .when(self.activity_expanded, |view| {
                                let mut log = div().flex().flex_col().gap_1();
                                for activity in visible {
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
                                            .child(
                                                div().w(px(6.)).h(px(6.)).rounded_full().bg(color),
                                            )
                                            .child(
                                                activity["summary"]
                                                    .as_str()
                                                    .unwrap_or("")
                                                    .to_owned(),
                                            ),
                                    );
                                }
                                view.child(log)
                            }),
                    );
                }
            }

            if let Some(approval) = detail["activities"]
                .as_array()
                .and_then(|activities| conversation::pending_approval(activities))
            {
                let request_id = approval.request_id.clone();
                let options = if approval.options.is_empty() {
                    vec![
                        ("accept".to_owned(), "Allow once".to_owned()),
                        ("decline".to_owned(), "Deny".to_owned()),
                    ]
                } else {
                    approval.options.clone()
                };
                let mut card = div()
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
                    .child(div().text_sm().child(approval.summary.clone()));
                if let Some(detail) = approval.detail.clone() {
                    card = card.child(div().text_xs().text_color(muted_foreground).child(detail));
                }
                card = card.child(div().flex().flex_wrap().gap_2().children(
                    options.into_iter().map(|(decision, label)| {
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
                    }),
                ));
                conversation = conversation.child(card);
            }

            if let Some(plans) = detail["proposedPlans"].as_array() {
                for plan in plans {
                    let markdown = plan["planMarkdown"].as_str().unwrap_or("").to_owned();
                    if markdown.is_empty() {
                        continue;
                    }
                    let id = gpui_kit::SharedString::from(format!(
                        "plan-{}",
                        plan["id"].as_str().unwrap_or("unknown")
                    ));
                    let implemented = !plan["implementedAt"].is_null();
                    conversation = conversation.child(
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
                                        view.child(
                                            div()
                                                .px_2()
                                                .rounded_sm()
                                                .bg(muted)
                                                .child("implemented"),
                                        )
                                    }),
                            )
                            .child(TextView::markdown(id, markdown)),
                    );
                }
            }

            if let Some(checkpoint) = detail["checkpoints"]
                .as_array()
                .and_then(|list| list.last())
            {
                let files = checkpoint["files"].as_array().cloned().unwrap_or_default();
                if !files.is_empty() {
                    let (additions, deletions) = conversation::diff_totals(&files);
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
                    card = card.child(
                        div().flex().justify_end().pt_1().child(
                            Button::new("revert-checkpoint")
                                .debug_selector(|| "revert-checkpoint".into())
                                .label(if confirm {
                                    "Confirm revert"
                                } else {
                                    "Revert to here"
                                })
                                .ghost()
                                .small()
                                .on_click(cx.listener(move |app, _, _, cx| {
                                    app.revert_checkpoint(turn_count, cx)
                                })),
                        ),
                    );
                    conversation = conversation.child(card);
                }
            }

            if let Some(error) = detail["session"]["lastError"].as_str() {
                conversation =
                    conversation.child(div().text_xs().text_color(danger).child(error.to_owned()));
            }
        } else if self
            .pending_threads
            .iter()
            .any(|draft| Some(&draft.thread.id) == self.selected_thread.as_ref())
        {
            let phase = self.selected_setup().map(|setup| setup.phase.clone());
            conversation = conversation.child(match phase.as_deref() {
                Some("running") => "Preparing worktree · your first message is waiting for setup",
                Some("cancelled") => {
                    "Setup cancelled · your draft and message are retained for retry"
                }
                Some("failed") => "Setup failed · your draft and message are retained for retry",
                _ => "New thread draft · send a message to start the conversation",
            });
            if phase.is_some() {
                conversation = conversation.child(self.composer.read(cx).value().to_string());
            }
        } else if self.selected_thread.is_some() {
            conversation = conversation.child(
                div()
                    .text_color(muted_foreground)
                    .child("Loading thread history…"),
            );
        } else {
            conversation = conversation.child(
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
            );
        }
        conversation
    }
}
