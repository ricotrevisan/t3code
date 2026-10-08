//! A bounded, virtualized jump index of user-message anchors, never a second history loader.
use super::*;

use crate::conversation_presentation::NavigationItem;

pub(crate) fn items(detail: &Value) -> Vec<NavigationItem> {
    let mut messages = detail["messages"]
        .as_array()
        .map_or(Vec::new(), |messages| {
            messages
                .iter()
                .filter(|message| message["role"] == "user" && message["id"].as_str().is_some())
                .collect::<Vec<_>>()
        });
    messages.sort_by_key(|message| message["createdAt"].as_str().unwrap_or(""));
    messages
        .into_iter()
        .map(|message| NavigationItem {
            message: message["id"].as_str().unwrap().to_owned(),
            preview: preview(message["text"].as_str().unwrap_or("")),
        })
        .collect()
}

fn preview(text: &str) -> String {
    let mut result = String::new();
    let mut count = 0;
    let mut pending_space = false;
    for c in text.chars() {
        if c.is_whitespace() {
            pending_space = !result.is_empty();
            continue;
        }
        if pending_space && count < 120 {
            result.push(' ');
            count += 1;
            pending_space = false;
        }
        if count >= 120 {
            break;
        }
        result.push(c);
        count += 1;
        if count == 120 {
            break;
        }
    }
    result
}

impl NativeApp {
    pub(super) fn render_timeline_navigator(&self, cx: &mut Context<Self>) -> gpui_kit::Div {
        let count = self.conversation_presentation.navigation.len();
        let open = self.conversation_presentation.navigator_open;
        let items = self.conversation_presentation.navigation.clone();
        let entity = cx.weak_entity();
        div()
            .flex()
            .flex_col()
            .gap_1()
            .when(count > 1, |view| {
                view.child(
                    Button::new("timeline-navigator-toggle")
                        .debug_selector(|| "timeline-navigator-toggle".into())
                        .ghost()
                        .xsmall()
                        .label(if open {
                            "Close message navigator".to_owned()
                        } else {
                            format!("Jump to message · {count}")
                        })
                        .on_click(cx.listener(|app, _, _, cx| {
                            app.conversation_presentation.navigator_open =
                                !app.conversation_presentation.navigator_open;
                            cx.notify();
                        })),
                )
            })
            .when(open && count > 1, |view| {
                view.child(
                    gpui_kit::uniform_list(
                        "timeline-navigator-list",
                        count,
                        move |range, _, _cx| {
                            range
                                .map(|index| {
                                    let item = &items[index];
                                    let message = item.message.clone();
                                    let entity = entity.clone();
                                    let selector = format!("timeline-jump-{message}");
                                    Button::new(gpui_kit::SharedString::from(selector.clone()))
                                        .debug_selector(move || selector.clone())
                                        .ghost()
                                        .small()
                                        .label(format!(
                                            "{} · {}",
                                            index + 1,
                                            if item.preview.is_empty() {
                                                "User message"
                                            } else {
                                                &item.preview
                                            }
                                        ))
                                        .on_click(move |_, _, cx| {
                                            let _ = entity.update(cx, |app, cx| {
                                                app.scroll_to_message(&message);
                                                app.conversation_presentation.navigator_open =
                                                    false;
                                                cx.notify();
                                            });
                                        })
                                        .into_any_element()
                                })
                                .collect()
                        },
                    )
                    .h(px(180.))
                    .w_full(),
                )
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn r13_d2_navigator_anchors_user_messages_in_order_with_bounded_previews() {
        let detail = serde_json::json!({"messages":[
            {"id":"b","role":"user","text":"x".repeat(500),"createdAt":"2026-10-02"},
            {"id":"a","role":"user","text":"first\n prompt","createdAt":"2026-10-01"},
            {"id":"c","role":"assistant","text":"not an anchor"}]});
        let index = items(&detail);
        assert_eq!(index.len(), 2);
        assert_eq!(index[0].message, "a");
        assert_eq!(index[0].preview, "first prompt");
        assert_eq!(index[1].preview.len(), 120);
    }
}
