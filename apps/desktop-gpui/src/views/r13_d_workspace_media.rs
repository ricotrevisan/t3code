//! Workspace media in messages and plans shares exact-resource signing and bounded decode.
use super::super::message_media::{self, MediaPreview};
use crate::NativeApp;
use gpui_kit::component::{
    ActiveTheme as _, Sizable,
    button::{Button, ButtonVariants as _},
};
use gpui_kit::{Context, SharedString, div, prelude::*, px};

impl NativeApp {
    pub(super) fn render_workspace_media(
        &mut self,
        images: Vec<(String, String)>,
        cx: &mut Context<Self>,
    ) -> gpui_kit::Div {
        div().flex().flex_wrap().gap_2().children(images.into_iter().map(|(alt, path)| {
            let resource = serde_json::json!({"_tag":"media-file", "threadId":self.selected_thread, "path":path});
            let key = message_media::media_key(&self.environment_scope, self.selected_thread.as_deref().unwrap_or(""), &format!("workspace:{path}"));
            self.request_message_image(key.clone(), resource.clone(), crate::attachment_mime(&path), cx);
            let image = self.conversation_presentation.media.get(&key).and_then(|preview| match preview { MediaPreview::Ready(image) => Some(image.clone()), _ => None });
            let failed = matches!(self.conversation_presentation.media.get(&key), Some(MediaPreview::Failed));
            let retry_key = key.clone();
            let retry_resource = resource.clone();
            let open_resource = resource.clone();
            let selector = format!("workspace-image-open-{path}");
            let mime = crate::attachment_mime(&path).to_owned();
            div().flex().flex_col().gap_1()
                .when_some(image, |view, image| view.child(div().id(SharedString::from(format!("workspace-image-{key}")))
                    .child(gpui_kit::img(image).w(px(160.)).h(px(100.)).object_fit(gpui_kit::ObjectFit::Contain))
                    .on_click(cx.listener(move |app, _, window, cx| app.expand_message_image(key.clone(), resource.clone(), window, cx)))))
                .when(failed, |view| view.child(div().text_xs().text_color(cx.theme().muted_foreground).child("Image preview unavailable"))
                    .child(Button::new(SharedString::from(format!("workspace-image-retry-{path}"))).debug_selector(|| "workspace-image-retry".into()).ghost().xsmall().label("Retry preview")
                        .on_click(cx.listener(move |app, _, _, cx| {
                            app.conversation_presentation.media.remove(&retry_key);
                            app.request_message_image(retry_key.clone(), retry_resource.clone(), &mime, cx);
                            cx.notify();
                        }))))
                .child(Button::new(SharedString::from(selector.clone())).debug_selector(move || selector.clone()).ghost().xsmall()
                    .label(format!("Open image · {}", if alt.is_empty() { path } else { alt }))
                    .on_click(cx.listener(move |app, _, _, cx| app.open_conversation_asset(open_resource.clone(), cx))))
        }))
    }
}
