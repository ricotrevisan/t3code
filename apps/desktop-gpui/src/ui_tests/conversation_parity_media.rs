use super::*;
use base64::Engine as _;
use gpui_kit::Modifiers;

#[gpui_kit::test]
fn conversation_image_expands_zooms_and_escape_restores_the_timeline(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    cx.update(|_, cx| app.update(cx, |app, cx| {
        app.apply_thread_update("t", json!({"kind":"snapshot","snapshot":{"snapshotSequence":1,"thread":{"id":"t","messages":[{"id":"u","role":"user","text":"Screenshot","attachments":[{"id":"pic","type":"image","mimeType":"image/png","name":"screenshot.png","sizeBytes":68}]}]}}}), cx).unwrap();
        cx.notify();
    }));
    cx.run_until_parked();
    let connection::Command::Call {
        tag,
        payload,
        reply,
    } = requests.try_recv().unwrap()
    else {
        panic!("image asset RPC")
    };
    assert_eq!(tag, "assets.createUrl");
    assert_eq!(payload["resource"]["attachmentId"], "pic");
    // No HTTP or OS browser call: inject the byte result at the loader boundary.
    reply
        .send_blocking(Err("fixture has no HTTP service".into()))
        .unwrap();
    cx.run_until_parked();
    let key = app.read_with(cx, |app, _| {
        crate::views::message_media::media_key(&app.environment_scope, "t", "pic")
    });
    cx.update(|_, cx| app.update(cx, |app, cx| {
        let bytes = base64::engine::general_purpose::STANDARD.decode("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+jRuoAAAAASUVORK5CYII=").unwrap();
        app.conversation_presentation.media.insert(key.clone(), crate::views::message_media::MediaPreview::Ready(std::sync::Arc::new(gpui_kit::Image::from_bytes(gpui_kit::ImageFormat::Png, bytes))));
        app.conversation_scroll.remeasure(); cx.notify();
    }));
    cx.run_until_parked();
    let selector: &'static str = format!("image-{key}").leak();
    let bounds = cx.debug_bounds(selector).unwrap();
    cx.simulate_click(bounds.center(), Modifiers::none());
    cx.run_until_parked();
    assert!(cx.debug_bounds("message-image-gallery").is_some());
    let bounds = cx.debug_bounds("image-zoom-in").unwrap();
    cx.simulate_click(bounds.center(), Modifiers::none());
    cx.run_until_parked();
    assert_eq!(
        app.read_with(cx, |app, _| app.conversation_presentation.zoom),
        1.5
    );
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    assert!(cx.debug_bounds("message-image-gallery").is_none());
    assert!(cx.debug_bounds("message-u").is_some());
}
