//! Opt-in text previews: exact-resource signing, bounded bytes, literal rendering.
use crate::NativeApp;
use gpui_kit::component::{
    ActiveTheme as _, Sizable,
    button::{Button, ButtonVariants as _},
    text::TextView,
};
use gpui_kit::{Context, SharedString, div, prelude::*, px};
use serde_json::{Value, json};
use t3code_desktop_gpui::connection;

#[derive(Clone)]
pub(crate) enum TextPreview {
    Loading(u64),
    Ready(String, bool),
    Failed(String),
}

pub(crate) fn supported(mime: &str) -> bool {
    mime.starts_with("text/")
        || matches!(
            mime,
            "application/json"
                | "application/xml"
                | "application/javascript"
                | "application/x-yaml"
        )
}
fn literal(text: &str) -> String {
    let fence = "`".repeat(
        text.split(|c| c != '`')
            .map(str::len)
            .max()
            .unwrap_or(0)
            .max(2)
            + 1,
    );
    format!("{fence}text\n{text}\n{fence}")
}
fn decode(bytes: &[u8], truncated: bool) -> Result<String, String> {
    if bytes.contains(&0) {
        return Err("Binary content cannot be previewed as text".into());
    }
    let mut end = bytes.len();
    let text = loop {
        match std::str::from_utf8(&bytes[..end]) {
            Ok(text) => break text,
            Err(error) if truncated && error.error_len().is_none() => end = error.valid_up_to(),
            Err(_) => return Err("This file is not UTF-8 text; download to view it".into()),
        }
    };
    Ok(text
        .chars()
        .map(|c| {
            if c.is_control() && !matches!(c, '\n' | '\r' | '\t') {
                '\u{fffd}'
            } else {
                c
            }
        })
        .collect())
}
impl NativeApp {
    pub(crate) fn render_attachment_text_preview(
        &self,
        key: String,
        resource: Value,
        mime: &str,
        cx: &mut Context<Self>,
    ) -> gpui_kit::Div {
        if !supported(mime) {
            return div();
        }
        let state = self.conversation_presentation.text_media.get(&key).cloned();
        let expanded = state.is_some();
        div()
            .flex()
            .flex_col()
            .gap_1()
            .child(
                Button::new(SharedString::from(format!("file-preview-{key}")))
                    .debug_selector(|| "file-text-preview".into())
                    .ghost()
                    .xsmall()
                    .label(if expanded {
                        "Close text preview"
                    } else {
                        "Preview text (up to 64 KiB)"
                    })
                    .on_click(cx.listener({
                        let key = key.clone();
                        let resource = resource.clone();
                        move |app, _, _, cx| {
                            if expanded {
                                app.conversation_presentation.text_media.remove(&key);
                                app.remeasure_text_attachment(&resource);
                                cx.notify();
                            } else {
                                app.request_attachment_text(key.clone(), resource.clone(), cx);
                            }
                        }
                    })),
            )
            .when_some(state, |view, state| match state {
                TextPreview::Loading(_) => view.child("Loading text preview…"),
                TextPreview::Ready(text, truncated) => view.child(
                    div()
                        .id(SharedString::from(format!("file-preview-body-{key}")))
                        .max_h(px(240.))
                        .overflow_y_scroll()
                        .children(
                            truncated
                                .then_some("Preview truncated · download for the complete file"),
                        )
                        .child(
                            TextView::markdown(
                                SharedString::from(format!("file-preview-text-{key}")),
                                literal(&text),
                            )
                            .selectable(true),
                        ),
                ),
                TextPreview::Failed(error) => view.child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().danger)
                        .child(error)
                        .child(
                            Button::new(SharedString::from(format!("file-preview-retry-{key}")))
                                .debug_selector(|| "file-text-preview-retry".into())
                                .ghost()
                                .xsmall()
                                .label("Retry preview")
                                .on_click(cx.listener(move |app, _, _, cx| {
                                    app.request_attachment_text(key.clone(), resource.clone(), cx)
                                })),
                        ),
                ),
            })
    }
    fn remeasure_text_attachment(&self, resource: &Value) {
        if let Some(index) = self
            .conversation_rows
            .iter()
            .position(|row| match &row.entry {
                t3code_desktop_gpui::conversation::TimelineEntry::Message { message, .. } => {
                    message["attachments"].as_array().is_some_and(|items| {
                        items
                            .iter()
                            .any(|item| item["id"] == resource["attachmentId"])
                    })
                }
                _ => false,
            })
        {
            self.conversation_scroll.remeasure_items(index..index + 1);
        }
    }
    pub(super) fn request_attachment_text(
        &mut self,
        key: String,
        mut resource: Value,
        cx: &mut Context<Self>,
    ) {
        let Some(sender) = self.commands.clone().filter(|_| self.connection_ready()) else {
            self.report_error("Connect to preview this file", cx);
            return;
        };
        if self.conversation_presentation.text_media.len() >= 64
            && !self.conversation_presentation.text_media.contains_key(&key)
        {
            self.report_error("Close a text preview before opening more (64 maximum)", cx);
            return;
        }
        let origin = self.environment_scope.clone();
        let generation = self.generation;
        let thread = self.selected_thread.clone();
        self.conversation_presentation.media_epoch =
            self.conversation_presentation.media_epoch.wrapping_add(1);
        let epoch = self.conversation_presentation.media_epoch;
        self.conversation_presentation
            .text_media
            .insert(key.clone(), TextPreview::Loading(epoch));
        resource["disposition"] = json!("inline");
        self.remeasure_text_attachment(&resource);
        cx.notify();
        let preview_resource = resource.clone();
        cx.spawn(async move |entity, cx| {
            let signed = connection::call(&sender, "assets.createUrl", json!({"resource":resource})).await
                .and_then(|value| super::signed_asset_url(&origin, &value));
            let result = match signed {
                Err(error) => Err(error),
                Ok(url) => {
                    let (tx, rx) = async_channel::bounded(1);
                    std::thread::spawn(move || {
                        let result = tokio::runtime::Builder::new_current_thread().enable_all().build().map_err(|_| "Text loader unavailable".to_owned()).and_then(|runtime| runtime.block_on(async move {
                            let client = reqwest::Client::builder().redirect(reqwest::redirect::Policy::none()).timeout(std::time::Duration::from_secs(30)).build().map_err(|_| "Text loader unavailable")?;
                            let mut response = client.get(url).send().await.map_err(|_| "Text request failed")?.error_for_status().map_err(|_| "Text request failed")?;
                            let mut bytes = Vec::new(); let mut truncated = false;
                            while let Some(chunk) = response.chunk().await.map_err(|_| "Text download failed")? {
                                let remaining = (64 * 1024usize).saturating_sub(bytes.len());
                                if chunk.len() > remaining { bytes.extend_from_slice(&chunk[..remaining]); truncated = true; break; }
                                bytes.extend_from_slice(&chunk);
                            }
                            decode(&bytes, truncated).map(|text| (text, truncated))
                        }));
                        let _ = tx.send_blocking(result);
                    });
                    rx.recv().await.unwrap_or_else(|_| Err("Text loader stopped".into()))
                }
            };
            let _ = entity.update(cx, |app, cx| {
                if app.generation != generation || app.environment_scope != origin || app.selected_thread != thread || !matches!(app.conversation_presentation.text_media.get(&key), Some(TextPreview::Loading(owner)) if *owner == epoch) { return; }
                app.conversation_presentation.text_media.insert(key, match result { Ok((text, truncated)) => TextPreview::Ready(text, truncated), Err(error) => TextPreview::Failed(error) });
                app.remeasure_text_attachment(&preview_resource);
                cx.notify();
            });
        }).detach();
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn r13_d2_text_preview_rejects_binary_and_preserves_utf8_truncation() {
        assert!(decode(b"a\0b", false).is_err());
        assert!(decode(&[0xff], false).is_err());
        assert_eq!(
            decode("hi😀".as_bytes().get(..5).unwrap(), true).unwrap(),
            "hi"
        );
        assert!(supported("text/csv"));
        assert!(!supported("application/pdf"));
        assert!(literal("```\nunsafe").starts_with("````text"));
    }
}
