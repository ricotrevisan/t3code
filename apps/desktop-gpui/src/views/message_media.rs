//! Exact-file capability images. Downloads do not carry OAuth credentials to a browser.
#[path = "r13_d_file_preview.rs"]
mod file_preview;
use crate::NativeApp;
pub(crate) use file_preview::TextPreview;
use gpui_kit::component::{
    ActiveTheme as _, Sizable,
    button::{Button, ButtonVariants as _},
};
use gpui_kit::{Context, Image, ImageFormat, div, img, prelude::*, px};
use serde_json::{Value, json};
use std::sync::Arc;
use t3code_desktop_gpui::connection;

#[derive(Clone)]
pub(crate) enum MediaPreview {
    Loading(u64),
    Ready(Arc<Image>),
    Failed,
}

pub(crate) fn media_key(origin: &str, thread: &str, id: &str) -> String {
    format!("{origin}\0{thread}\0{id}")
}

pub(crate) fn signed_asset_url(origin: &str, response: &Value) -> Result<String, String> {
    let relative = response["relativeUrl"]
        .as_str()
        .ok_or("Invalid asset response")?;
    let base = url::Url::parse(origin).map_err(|_| "Invalid environment origin")?;
    if !matches!(base.scheme(), "http" | "https")
        || base.origin().ascii_serialization() != origin
        || !relative.starts_with("/api/assets/")
        || relative.contains(['\r', '\n', '?', '#'])
    {
        return Err("Invalid asset response".into());
    }
    Ok(format!("{origin}{relative}"))
}

/// Decode a single static frame on the loader thread. GPUI never sees animated bytes.
/// Source decoding is capped at 16M pixels; the retained preview is at most 1200².
pub(crate) fn bounded_preview(bytes: &[u8], format: ImageFormat) -> Result<Arc<Image>, String> {
    use image::ImageDecoder as _;
    use std::io::Cursor;
    if bytes.len() > 10 * 1024 * 1024 {
        return Err("Image exceeds preview limit".into());
    }
    let format = match format {
        ImageFormat::Png => image::ImageFormat::Png,
        ImageFormat::Jpeg => image::ImageFormat::Jpeg,
        ImageFormat::Webp => image::ImageFormat::WebP,
        ImageFormat::Gif => {
            check_gif_frames(bytes)?;
            image::ImageFormat::Gif
        }
        _ => return Err("Unsupported preview format".into()),
    };
    let mut reader = image::ImageReader::with_format(Cursor::new(bytes), format);
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(4096);
    limits.max_image_height = Some(4096);
    limits.max_alloc = Some(128 * 1024 * 1024);
    reader.limits(limits);
    let decoder = reader
        .into_decoder()
        .map_err(|_| "Image exceeds decode limits or is invalid")?;
    let (width, height) = decoder.dimensions();
    if u64::from(width) * u64::from(height) > 4096 * 4096 {
        return Err("Image exceeds decoded pixel budget".into());
    }
    // DynamicImage's GIF decoder reads the first frame, not into_frames().
    let decoded =
        image::DynamicImage::from_decoder(decoder).map_err(|_| "Image preview decode failed")?;
    let thumbnail = decoded.thumbnail(1200, 1200);
    let mut png = Cursor::new(Vec::new());
    thumbnail
        .write_to(&mut png, image::ImageFormat::Png)
        .map_err(|_| "Image preview encode failed")?;
    Ok(Arc::new(Image::from_bytes(
        ImageFormat::Png,
        png.into_inner(),
    )))
}

/// Walk GIF container blocks without decompressing any frame. A frame count limit
/// rejects animation bombs before the image decoder allocates its canvas.
pub(crate) fn check_gif_frames(bytes: &[u8]) -> Result<(), String> {
    let invalid = || "Invalid or excessive GIF animation".to_owned();
    if bytes.len() < 13 || !matches!(&bytes[..6], b"GIF87a" | b"GIF89a") {
        return Err(invalid());
    }
    let table_len = |flags: u8| {
        if flags & 0x80 != 0 {
            3usize << ((flags & 7) + 1)
        } else {
            0
        }
    };
    let mut offset = 13 + table_len(bytes[10]);
    let mut frames = 0;
    loop {
        let tag = *bytes.get(offset).ok_or_else(invalid)?;
        offset += 1;
        match tag {
            0x3b => return if frames > 0 { Ok(()) } else { Err(invalid()) },
            0x21 => {
                offset += 1;
            }
            0x2c => {
                frames += 1;
                if frames > 256 {
                    return Err(invalid());
                }
                let flags = *bytes.get(offset + 8).ok_or_else(invalid)?;
                offset += 9 + table_len(flags) + 1; // descriptor/table/LZW code size
            }
            _ => return Err(invalid()),
        }
        loop {
            let size = usize::from(*bytes.get(offset).ok_or_else(invalid)?);
            offset += 1;
            if size == 0 {
                break;
            }
            offset = offset
                .checked_add(size)
                .filter(|end| *end <= bytes.len())
                .ok_or_else(invalid)?;
        }
    }
}

impl NativeApp {
    /// Drop GPUI's app-global decoded cache only after both surfaces release the hash.
    pub(crate) fn release_message_image(&mut self, image: Arc<Image>, cx: &mut Context<Self>) {
        let shared = self
            .composer_state
            .previews
            .values()
            .any(|live| live.id() == image.id())
            || self.conversation_presentation.media.values().any(
                |preview| matches!(preview, MediaPreview::Ready(live) if live.id() == image.id()),
            );
        if !shared {
            image.remove_asset(cx);
            self.conversation_presentation.released_images += 1;
        }
    }

    pub(crate) fn expand_message_image(
        &mut self,
        key: String,
        resource: Value,
        window: &mut gpui_kit::Window,
        cx: &mut Context<Self>,
    ) {
        let focus = self
            .conversation_presentation
            .gallery_focus
            .get_or_insert_with(|| cx.focus_handle())
            .clone();
        let thread = self.selected_thread.as_deref().unwrap_or("");
        let mut items = Vec::new();
        for row in self.conversation_rows.iter() {
            let (text, attachments) = match &row.entry {
                t3code_desktop_gpui::conversation::TimelineEntry::Message { message, .. } => (
                    message["text"].as_str().unwrap_or(""),
                    message["attachments"]
                        .as_array()
                        .map_or(&[][..], Vec::as_slice),
                ),
                t3code_desktop_gpui::conversation::TimelineEntry::Plan(plan) => {
                    (plan["planMarkdown"].as_str().unwrap_or(""), &[][..])
                }
                _ => continue,
            };
            let mut group = Vec::new();
            for attachment in attachments {
                let Some(id) = attachment["id"].as_str() else {
                    continue;
                };
                let mime = attachment["mimeType"].as_str().unwrap_or("");
                if !matches!(
                    mime,
                    "image/png" | "image/jpeg" | "image/webp" | "image/gif"
                ) {
                    continue;
                }
                group.push((
                    media_key(&self.environment_scope, thread, id),
                    json!({"_tag":"attachment", "threadId":thread, "attachmentId":id}),
                    mime.to_owned(),
                ));
            }
            for (_, path) in crate::conversation_presentation::workspace_markdown_images(text).1 {
                let mime = match path
                    .rsplit('.')
                    .next()
                    .unwrap_or("")
                    .to_ascii_lowercase()
                    .as_str()
                {
                    "jpg" | "jpeg" => "image/jpeg",
                    "gif" => "image/gif",
                    "webp" => "image/webp",
                    _ => "image/png",
                };
                group.push((
                    media_key(
                        &self.environment_scope,
                        thread,
                        &format!("workspace:{path}"),
                    ),
                    json!({"_tag":"media-file", "threadId":thread, "path":path}),
                    mime.to_owned(),
                ));
            }
            if group.iter().any(|(candidate, _, _)| candidate == &key) {
                items = group;
                break;
            }
        }
        self.conversation_presentation.gallery_items = items;
        self.conversation_presentation.gallery = Some((key, resource));
        self.conversation_presentation.zoom = 1.;
        focus.focus(window, cx);
        cx.notify();
    }

    pub(crate) fn request_message_image(
        &mut self,
        key: String,
        resource: Value,
        mime: &str,
        cx: &mut Context<Self>,
    ) {
        if self.conversation_presentation.media.contains_key(&key) {
            return;
        }
        let Some(sender) = self.commands.clone() else {
            return;
        };
        let format = match mime {
            "image/jpeg" => ImageFormat::Jpeg,
            "image/png" => ImageFormat::Png,
            "image/webp" => ImageFormat::Webp,
            "image/gif" => ImageFormat::Gif,
            _ => return,
        };
        // Viewport-lazy, bounded compressed-image cache. Never keep signed URLs in state.
        if self.conversation_presentation.media.len() >= 8 {
            // Protect a bounded viewport neighborhood, not the whole history.
            let first = self
                .conversation_scroll
                .logical_scroll_top()
                .item_ix
                .saturating_sub(8);
            let obsolete =
                self.conversation_presentation
                    .media
                    .iter()
                    .find_map(|(cached, preview)| {
                        if matches!(preview, MediaPreview::Loading(_))
                            || self
                                .conversation_presentation
                                .gallery
                                .as_ref()
                                .is_some_and(|(open, _)| open == cached)
                        {
                            return None;
                        }
                        let resource_id = cached.rsplit('\0').next().unwrap_or("");
                        let nearby =
                            self.conversation_rows
                                .iter()
                                .skip(first)
                                .take(64)
                                .any(|row| {
                                    let t3code_desktop_gpui::conversation::TimelineEntry::Message {
                                        message,
                                        ..
                                    } = &row.entry
                                    else {
                                        return false;
                                    };
                                    if let Some(path) = resource_id.strip_prefix("workspace:") {
                                        message["text"]
                                            .as_str()
                                            .is_some_and(|text| text.contains(path))
                                    } else {
                                        message["attachments"].as_array().is_some_and(|items| {
                                            items.iter().any(|item| item["id"] == resource_id)
                                        })
                                    }
                                });
                        let gallery_request = self
                            .conversation_presentation
                            .gallery
                            .as_ref()
                            .is_some_and(|(open, _)| open == &key);
                        (!nearby || gallery_request).then(|| cached.clone())
                    });
            let Some(obsolete) = obsolete else {
                return;
            };
            if let Some(MediaPreview::Ready(image)) =
                self.conversation_presentation.media.remove(&obsolete)
            {
                self.release_message_image(image, cx);
            }
        }
        self.conversation_presentation.media_epoch =
            self.conversation_presentation.media_epoch.wrapping_add(1);
        let epoch = self.conversation_presentation.media_epoch;
        self.conversation_presentation
            .media
            .insert(key.clone(), MediaPreview::Loading(epoch));
        let origin = self.environment_scope.clone();
        let generation = self.generation;
        let thread = self.selected_thread.clone();
        let row_resource = resource.clone();
        cx.spawn(async move |entity, cx| {
            let response =
                connection::call(&sender, "assets.createUrl", json!({"resource":resource})).await;
            let image = if let Ok(url) =
                response.and_then(|response| signed_asset_url(&origin, &response))
            {
                let (tx, rx) = async_channel::bounded(1);
                std::thread::spawn(move || {
                    let result = tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                        .map_err(|_| "Image loader unavailable".to_owned())
                        .and_then(|runtime| {
                            runtime.block_on(async {
                                let client = reqwest::Client::builder()
                                    .redirect(reqwest::redirect::Policy::none())
                                    .timeout(std::time::Duration::from_secs(30))
                                    .build()
                                    .map_err(|_| "Image loader unavailable")?;
                                let mut response = client
                                    .get(url)
                                    .send()
                                    .await
                                    .map_err(|_| "Image request failed")?
                                    .error_for_status()
                                    .map_err(|_| "Image request failed")?;
                                let mut bytes = Vec::new();
                                while let Some(chunk) = response
                                    .chunk()
                                    .await
                                    .map_err(|_| "Image download failed")?
                                {
                                    if bytes.len() + chunk.len() > 10 * 1024 * 1024 {
                                        return Err("Image exceeds preview limit".to_owned());
                                    }
                                    bytes.extend_from_slice(&chunk);
                                }
                                bounded_preview(&bytes, format)
                            })
                        });
                    let _ = tx.send_blocking(result);
                });
                rx.recv().await.ok().and_then(Result::ok)
            } else {
                None
            };
            let _ = entity.update(cx, |app, cx| {
                if app.generation != generation
                    || app.environment_scope != origin
                    || app.selected_thread != thread
                    || !matches!(app.conversation_presentation.media.get(&key),
                        Some(MediaPreview::Loading(owner)) if *owner == epoch)
                {
                    return;
                }
                app.conversation_presentation
                    .media
                    .insert(key, image.map_or(MediaPreview::Failed, MediaPreview::Ready));
                for (index, row) in app.conversation_rows.iter().enumerate() {
                    if let t3code_desktop_gpui::conversation::TimelineEntry::Message {
                        message,
                        ..
                    } = &row.entry
                    {
                        let attachment = row_resource["attachmentId"].as_str().is_some_and(|id| {
                            message["attachments"]
                                .as_array()
                                .is_some_and(|items| items.iter().any(|item| item["id"] == id))
                        });
                        let workspace = row_resource["path"].as_str().is_some_and(|path| {
                            message["text"]
                                .as_str()
                                .is_some_and(|text| text.contains(path))
                        });
                        if attachment || workspace {
                            app.conversation_scroll.remeasure_items(index..index + 1);
                        }
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn navigate_message_gallery(&mut self, direction: isize, cx: &mut Context<Self>) {
        let items = &self.conversation_presentation.gallery_items;
        let Some(index) = self
            .conversation_presentation
            .gallery
            .as_ref()
            .and_then(|(key, _)| items.iter().position(|(candidate, _, _)| candidate == key))
        else {
            return;
        };
        let next = (index as isize + direction).rem_euclid(items.len() as isize) as usize;
        let (key, resource, mime) = items[next].clone();
        self.conversation_presentation.gallery = Some((key.clone(), resource.clone()));
        self.conversation_presentation.zoom = 1.;
        self.request_message_image(key, resource, &mime, cx);
        cx.notify();
    }

    pub(crate) fn render_message_gallery(
        &self,
        cx: &mut Context<Self>,
    ) -> Option<impl IntoElement + use<>> {
        let (key, resource) = self.conversation_presentation.gallery.as_ref()?;
        let image = match self.conversation_presentation.media.get(key) {
            Some(MediaPreview::Ready(image)) => Some(image.clone()),
            _ => None,
        };
        let failed = matches!(
            self.conversation_presentation.media.get(key),
            Some(MediaPreview::Failed)
        );
        let multiple = self.conversation_presentation.gallery_items.len() > 1;
        let resource = resource.clone();
        let zoom = self.conversation_presentation.zoom.max(1.);
        Some(
            div()
                .id("message-image-gallery")
                .debug_selector(|| "message-image-gallery".into())
                .key_context("NativeApp")
                .when_some(
                    self.conversation_presentation.gallery_focus.clone(),
                    |view, focus| view.track_focus(&focus),
                )
                .on_action(cx.listener(|app, _: &crate::DismissOverlay, window, cx| {
                    app.conversation_presentation.gallery = None;
                    app.composer.update(cx, |input, cx| input.focus(window, cx));
                    cx.stop_propagation();
                    cx.notify();
                }))
                .on_key_down(cx.listener(|app, event: &gpui_kit::KeyDownEvent, _, cx| {
                    match event.keystroke.key.as_str() {
                        "left" => app.navigate_message_gallery(-1, cx),
                        "right" => app.navigate_message_gallery(1, cx),
                        _ => return,
                    }
                    cx.stop_propagation();
                }))
                .absolute()
                .inset_0()
                .bg(cx.theme().background)
                .p_3()
                .flex()
                .flex_col()
                .gap_2()
                .occlude()
                .child(
                    div()
                        .flex()
                        .gap_1()
                        .child(
                            Button::new("image-close")
                                .debug_selector(|| "image-close".into())
                                .ghost()
                                .small()
                                .label("Close image")
                                .on_click(cx.listener(|app, _, window, cx| {
                                    app.conversation_presentation.gallery = None;
                                    app.composer.update(cx, |input, cx| input.focus(window, cx));
                                    cx.notify();
                                })),
                        )
                        .when(multiple, |view| {
                            view.child(
                                Button::new("image-previous")
                                    .debug_selector(|| "image-previous".into())
                                    .ghost()
                                    .small()
                                    .label("← Previous")
                                    .on_click(cx.listener(|app, _, _, cx| {
                                        app.navigate_message_gallery(-1, cx)
                                    })),
                            )
                            .child(
                                Button::new("image-next")
                                    .debug_selector(|| "image-next".into())
                                    .ghost()
                                    .small()
                                    .label("Next →")
                                    .on_click(cx.listener(|app, _, _, cx| {
                                        app.navigate_message_gallery(1, cx)
                                    })),
                            )
                        })
                        .child(
                            Button::new("image-zoom-in")
                                .debug_selector(|| "image-zoom-in".into())
                                .ghost()
                                .small()
                                .label("Zoom in")
                                .on_click(cx.listener(|app, _, _, cx| {
                                    app.conversation_presentation.zoom =
                                        (app.conversation_presentation.zoom.max(1.) * 1.5).min(8.);
                                    cx.notify();
                                })),
                        )
                        .child(
                            Button::new("image-zoom-out")
                                .ghost()
                                .small()
                                .label("Zoom out")
                                .on_click(cx.listener(|app, _, _, cx| {
                                    app.conversation_presentation.zoom =
                                        (app.conversation_presentation.zoom / 1.5).max(1.);
                                    cx.notify();
                                })),
                        )
                        .child(
                            Button::new("image-download")
                                .ghost()
                                .small()
                                .label("Open / download")
                                .on_click(cx.listener(move |app, _, _, cx| {
                                    app.open_conversation_asset(resource.clone(), cx)
                                })),
                        ),
                )
                .child(
                    div()
                        .id("image-pan")
                        .overflow_scroll()
                        .flex_1()
                        .when_some(image, |view, image| {
                            view.child(
                                img(image)
                                    .w(px(600. * zoom))
                                    .object_fit(gpui_kit::ObjectFit::Contain),
                            )
                        })
                        .when(failed, |view| {
                            view.child("Preview unavailable · open / download above")
                        })
                        .when(
                            matches!(
                                self.conversation_presentation.media.get(key),
                                Some(MediaPreview::Loading(_))
                            ),
                            |view| view.child("Loading preview…"),
                        )
                        .when(
                            !self.conversation_presentation.media.contains_key(key),
                            |view| view.child("Preview not cached · open / download above"),
                        ),
                ),
        )
    }
}
