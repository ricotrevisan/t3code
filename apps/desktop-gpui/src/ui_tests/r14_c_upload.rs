use super::*;

#[gpui_kit::test]
fn r14_c_cancel_drops_http_receipt_before_delete_and_keeps_draft(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.environment_scope = "https://attachments.example".into();
            app.composer
                .update(cx, |input, cx| input.set_value("keep draft", window, cx));
            let scope = app.attachment_scope().unwrap();
            app.attach_inputs(
                scope,
                vec![crate::app::attachments::AttachmentInput::Bytes {
                    name: "notes.txt".into(),
                    bytes: b"body".to_vec(),
                    source: None,
                }],
                cx,
            );
        })
    });
    cx.run_until_parked();
    let connection::Command::Call {
        tag,
        payload,
        reply,
    } = requests.try_recv().unwrap()
    else {
        panic!("sign");
    };
    assert_eq!(tag, "attachments.createUploadUrl");
    assert_eq!(payload["sizeBytes"], 4);
    reply.try_send(Ok(json!({"attachmentId":"pending-00000000-0000-4000-8000-000000000001-txt","relativeUrl":"/api/attachments/upload/claims.signature"}))).unwrap();
    cx.run_until_parked();
    let connection::Command::Upload {
        bytes,
        reply: upload,
        ..
    } = requests.try_recv().unwrap()
    else {
        panic!("upload");
    };
    assert_eq!(bytes, b"body");
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            let id = app.attachment_drafts.transfers[0].id.clone();
            app.cancel_transfer(&id, cx);
        })
    });
    cx.run_until_parked();
    assert!(
        upload.is_closed(),
        "HTTP worker must receive cancellation without an upload receipt"
    );
    let connection::Command::Call {
        tag,
        payload,
        reply,
    } = requests.try_recv().unwrap()
    else {
        panic!("cleanup");
    };
    assert_eq!(tag, "attachments.delete");
    assert_eq!(
        payload["attachmentId"],
        "pending-00000000-0000-4000-8000-000000000001-txt"
    );
    app.read_with(cx, |app, cx| {
        assert!(app.attachments.is_empty());
        assert_eq!(app.composer.read(cx).value().as_str(), "keep draft");
        assert_eq!(app.attachment_drafts.transfers[0].phase, "Cancelling");
    });
    reply.try_send(Ok(json!({}))).unwrap();
    cx.run_until_parked();
    app.read_with(cx, |app, _| {
        assert_eq!(app.attachment_drafts.transfers[0].phase, "Cancelled");
        assert_eq!(app.pending_actions, 0);
        assert!(!app.attachment_drafts.uploading);
    });
}

#[gpui_kit::test]
fn r14_c_oversize_image_signs_compressed_metadata_and_installs_receipted_ref(
    cx: &mut TestAppContext,
) {
    let (app, cx, requests) = harness(cx);
    let mut state = 42_u32;
    let image = image::RgbImage::from_fn(2100, 2100, |_, _| {
        image::Rgb(std::array::from_fn(|_| {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            state as u8
        }))
    });
    let mut encoded = std::io::Cursor::new(Vec::new());
    image::DynamicImage::ImageRgb8(image)
        .write_to(&mut encoded, image::ImageFormat::Png)
        .unwrap();
    let bytes = encoded.into_inner();
    assert!(bytes.len() > t3code_desktop_gpui::composer_image::IMAGE_LIMIT);
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.environment_scope = "https://attachments.example".into();
            let scope = app.attachment_scope().unwrap();
            app.attach_inputs(
                scope,
                vec![crate::app::attachments::AttachmentInput::Bytes {
                    name: "large.png".into(),
                    bytes,
                    source: None,
                }],
                cx,
            );
        })
    });
    cx.run_until_parked();
    let connection::Command::Call {
        tag,
        payload,
        reply,
    } = requests.try_recv().unwrap()
    else {
        panic!("sign compressed");
    };
    assert_eq!(tag, "attachments.createUploadUrl");
    assert_eq!(payload["name"], "large.jpg");
    assert_eq!(payload["mimeType"], "image/jpeg");
    let size = payload["sizeBytes"].as_u64().unwrap();
    assert!(size <= t3code_desktop_gpui::composer_image::IMAGE_LIMIT as u64);
    reply.try_send(Ok(json!({"attachmentId":"pending-00000000-0000-4000-8000-000000000002-jpg","relativeUrl":"/api/attachments/upload/claims.signature"}))).unwrap();
    cx.run_until_parked();
    let connection::Command::Upload {
        bytes,
        content_type,
        reply,
        ..
    } = requests.try_recv().unwrap()
    else {
        panic!("compressed body");
    };
    assert_eq!(content_type, "image/jpeg");
    assert_eq!(bytes.len() as u64, size);
    let decoded = image::load_from_memory(&bytes).unwrap();
    assert!(decoded.width().max(decoded.height()) <= 2048);
    reply.try_send(Ok(())).unwrap();
    cx.run_until_parked();
    app.read_with(cx, |app, _| {
        assert_eq!(app.attachments.len(), 1);
        assert_eq!(app.attachments[0]["name"], "large.jpg");
        assert_eq!(app.attachments[0]["sizeBytes"], size);
        assert_eq!(app.attachment_drafts.transfers[0].phase, "Attached");
    });
}
