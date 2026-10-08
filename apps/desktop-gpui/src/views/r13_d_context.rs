//! Received context is a source snapshot, not a JSON-copy action.
use crate::NativeApp;
use gpui_kit::base::Disableable;
use gpui_kit::component::{
    ActiveTheme as _, Sizable,
    button::{Button, ButtonVariants as _},
    text::TextView,
};
use gpui_kit::{Context, SharedString, div, prelude::*};
use serde_json::Value;

fn literal_markdown(text: &str) -> String {
    let fence = "`".repeat(
        text.split(|ch| ch != '`')
            .map(str::len)
            .max()
            .unwrap_or(0)
            .max(2)
            + 1,
    );
    format!("{fence}text\n{text}\n{fence}")
}

fn field(record: &Value, name: &str) -> String {
    record[name].as_str().unwrap_or_default().to_owned()
}

pub(super) fn disclosure_key(message: &str, record: &Value) -> String {
    format!(
        "context:{message}:{}",
        record["contextId"].as_str().unwrap_or("unknown")
    )
}

/// Human-readable snapshot and an optional source path. Never infer a live source
/// from a terminal id or browser selector belonging to an earlier session.
fn details(record: &Value) -> (String, Option<String>) {
    match record["kind"].as_str().unwrap_or_default() {
        "assistant-citation" => (format!("Assistant quote · {}\n\n{}\n\n{}", field(&record["payload"], "messageId"), field(&record["payload"], "text"), field(&record["payload"], "comment")), None),
        "mention" => (field(record, "path"), Some(field(record, "path"))),
        "skill" => (format!("Skill · ${}", field(record, "name")), None),
        "terminal" => (format!("{} · lines {}–{}\n\n{}", field(record, "terminalLabel"), record["lineStart"], record["lineEnd"], field(record, "text")), None),
        "review-comment" => {
            let path = field(record, "filePath");
            let pr = &record["pullRequest"];
            let title = if pr.is_object() { format!("PR #{} · {} · {}\n", pr["number"], field(pr, "title"), field(pr, "state")) } else { String::new() };
            (format!("{title}{} · {}\n{}\n\n{}\n\n{}", field(record, "sectionTitle"), path, field(record, "rangeLabel"), field(record, "text"), field(record, "diff")), Some(path))
        }
        "element" => {
            let source = &record["source"];
            let path = source["fileName"].as_str().filter(|s| !s.is_empty()).map(|path| {
                source["lineNumber"].as_u64().map_or_else(|| path.to_owned(), |line| format!("{path}:{}", line.max(1)))
            });
            (format!("{}\n{} · {}\n{}\n\n{}\n\n{}", field(record, "pageUrl"), field(record, "tagName"), field(record, "componentName"), field(record, "selector"), field(record, "htmlPreview"), field(record, "styles")), path)
        }
        "preview-annotation" => {
            let changes = record["styleChanges"].as_array().map(|items| items.iter().filter_map(Value::as_str).collect::<Vec<_>>().join("\n")).unwrap_or_default();
            (format!("{}\n{}\n\n{}\n\n{}", field(record, "pageUrl"), field(record, "targetSummary"), field(record, "comment"), changes), None)
        }
        "image" | "file" => (format!("{} · {} · {} bytes", field(record, "name"), field(record, "mimeType"), record["sizeBytes"]), None),
        _ => ("This context kind is not supported by this client. Its original payload is preserved in the message.".into(), None),
    }
}

fn reusable_snapshot(
    record: &Value,
) -> Result<t3code_desktop_gpui::draft_context::DraftContextRecord, &'static str> {
    use t3code_desktop_gpui::draft_context::DraftContextRecord;
    let kind = record["kind"]
        .as_str()
        .ok_or("Context kind is unavailable")?;
    let label = record["label"].as_str().unwrap_or("Saved context");
    if kind == "assistant-citation" {
        return DraftContextRecord::citation(label, record["payload"].clone());
    }
    if !matches!(
        kind,
        "terminal" | "review-comment" | "element" | "preview-annotation" | "mention" | "skill"
    ) {
        return Err("This context cannot be reused without its original resource");
    }
    DraftContextRecord::capture(kind, label, record.clone())
}

impl NativeApp {
    pub(super) fn render_received_context(
        &self,
        message_value: &Value,
        record: &Value,
        cx: &Context<Self>,
    ) -> gpui_kit::Div {
        let message = message_value["id"].as_str().unwrap_or("unknown");
        let key = disclosure_key(message, record);
        let expanded = self.activity_expanded.get(&key) == Some(&true);
        let kind = record["kind"].as_str().unwrap_or("context");
        let label = record["label"]
            .as_str()
            .filter(|s| !s.is_empty())
            .or_else(|| record["name"].as_str())
            .or_else(|| record["path"].as_str())
            .unwrap_or("Unavailable context");
        let (mut text, source) = details(record);
        if matches!(kind, "image" | "file") {
            let available = message_value["attachments"]
                .as_array()
                .is_some_and(|attachments| {
                    attachments.iter().any(|attachment| {
                        attachment["id"] == record["attachmentId"]
                            && attachment["downloadable"] != false
                    })
                });
            text.push_str(if available {
                "\nOpen the attachment above to view or download it."
            } else {
                "\nThis attachment is no longer available in the message."
            });
        }
        let selector = format!(
            "context-toggle-{message}-{}",
            record["contextId"].as_str().unwrap_or("unknown")
        );
        let detail_selector = format!(
            "context-detail-{message}-{}",
            record["contextId"].as_str().unwrap_or("unknown")
        );
        let copy_key = key.clone();
        let reusable = reusable_snapshot(record).ok();
        let scope = self.environment_scope.clone();
        let thread = self.selected_thread.clone().unwrap_or_default();
        let generation = self.generation;
        div()
            .flex()
            .flex_col()
            .gap_1()
            .min_w_0()
            .child(
                Button::new(SharedString::from(selector.clone()))
                    .debug_selector(move || selector.clone())
                    .ghost()
                    .xsmall()
                    .label(format!(
                        "{} {kind} · {label}",
                        if expanded { "▾" } else { "▸" }
                    ))
                    .on_click(cx.listener(move |app, _, _, cx| {
                        app.conversation_scroll.pause_following_tail();
                        app.activity_expanded.insert(key.clone(), !expanded);
                        cx.notify();
                    })),
            )
            .when(expanded, |view| {
                view.child(
                    div()
                        .debug_selector(move || detail_selector.clone())
                        .flex()
                        .flex_col()
                        .gap_1()
                        .p_2()
                        .border_1()
                        .border_color(cx.theme().border)
                        .rounded_md()
                        .child(
                            TextView::markdown(
                                SharedString::from(format!("context-text-{copy_key}")),
                                literal_markdown(&text),
                            )
                            .selectable(true),
                        )
                        .child(
                            div()
                                .flex()
                                .gap_1()
                                .when(kind == "assistant-citation", |view| {
                                    let citation = record["payload"].clone();
                                    view.child(
                                        Button::new(SharedString::from(format!(
                                            "citation-source-{copy_key}"
                                        )))
                                        .debug_selector(|| "citation-open-source".into())
                                        .ghost()
                                        .xsmall()
                                        .label("Jump to quoted source")
                                        .on_click(
                                            cx.listener(move |app, _, window, cx| {
                                                app.navigate_to_citation(&citation, window, cx)
                                            }),
                                        ),
                                    )
                                })
                                .when_some(reusable, |view, snapshot| {
                                    view.child(
                                        Button::new(SharedString::from(format!(
                                            "context-reuse-{copy_key}"
                                        )))
                                        .debug_selector(|| "context-reuse-snapshot".into())
                                        .ghost()
                                        .xsmall()
                                        .label("Reuse saved context")
                                        .tooltip("Add the saved snapshot, not a new live capture")
                                        .on_click(
                                            cx.listener(move |app, _, window, cx| {
                                                if app.generation != generation {
                                                    return;
                                                }
                                                if let Err(error) = app.intake_draft_context(
                                                    &scope,
                                                    &thread,
                                                    snapshot.clone(),
                                                    window,
                                                    cx,
                                                ) {
                                                    app.report_error(error, cx);
                                                }
                                            }),
                                        ),
                                    )
                                })
                                .when_some(source, |view, source| {
                                    view.child(
                                        Button::new(SharedString::from(format!(
                                            "context-source-{copy_key}"
                                        )))
                                        .debug_selector(|| "context-open-source".into())
                                        .ghost()
                                        .xsmall()
                                        .label("Open source file")
                                        .disabled(!self.connection_ready())
                                        .on_click(
                                            cx.listener(move |app, _, window, cx| {
                                                app.open_message_link(&source, window, cx)
                                            }),
                                        ),
                                    )
                                })
                                .child(
                                    Button::new(SharedString::from(format!(
                                        "context-copy-{copy_key}"
                                    )))
                                    .ghost()
                                    .xsmall()
                                    .label("Copy context text")
                                    .on_click(cx.listener(move |app, _, _, cx| {
                                        app.copy_conversation_text(
                                            copy_key.clone(),
                                            text.clone(),
                                            cx,
                                        )
                                    })),
                                ),
                        ),
                )
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn r13_d_received_context_preserves_terminal_range_and_snapshot() {
        let (text, source) = details(
            &json!({"kind":"terminal","terminalLabel":"Build","lineStart":4,"lineEnd":7,"text":"failed\ntry again"}),
        );
        assert_eq!(text, "Build · lines 4–7\n\nfailed\ntry again");
        assert!(source.is_none());
    }
    #[test]
    fn r13_d_review_context_retains_comment_diff_and_pr_identity() {
        let (text, source) = details(
            &json!({"kind":"review-comment","filePath":"src/main.rs","sectionTitle":"Review","rangeLabel":"lines 4–5","text":"Please fix","diff":"-bad\n+good","pullRequest":{"number":17,"title":"Fix","state":"open"}}),
        );
        assert!(text.contains("PR #17 · Fix · open"));
        assert!(text.contains("Please fix\n\n-bad\n+good"));
        assert_eq!(source.as_deref(), Some("src/main.rs"));
    }
}
