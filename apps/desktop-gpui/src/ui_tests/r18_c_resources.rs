use super::*;

#[gpui_kit::test]
fn r18_c_folder_picker_intake_uploads_each_body_and_sends_owned_references(
    cx: &mut TestAppContext,
) {
    let (app, cx, requests) = harness(cx);
    let folder = tempfile::tempdir().unwrap();
    std::fs::create_dir(folder.path().join("nested")).unwrap();
    std::fs::write(folder.path().join("a.txt"), b"first").unwrap();
    std::fs::write(folder.path().join("nested/b.txt"), b"second").unwrap();
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.environment_scope = "https://attachments.example".into();
            app.provider_catalog = vec![super::composer_parity::provider_fixture(
                "codex", "Personal",
            )];
            app.attach_paths(
                app.attachment_scope().unwrap(),
                Some(vec![folder.path().to_path_buf()]),
                cx,
            );
        })
    });
    cx.run_until_parked();
    for (index, (name, body)) in [
        ("a.txt", b"first".as_slice()),
        ("b.txt", b"second".as_slice()),
    ]
    .into_iter()
    .enumerate()
    {
        let connection::Command::Call {
            tag,
            payload,
            reply,
        } = requests.try_recv().unwrap()
        else {
            panic!("sign")
        };
        assert_eq!(tag, "attachments.createUploadUrl");
        assert_eq!(payload["name"], name);
        assert_eq!(payload["sizeBytes"], body.len());
        let id = format!("pending-00000000-0000-4000-8000-{:012}-txt", index + 1);
        reply
            .try_send(Ok(
                json!({"attachmentId":id,"relativeUrl":"/api/attachments/upload/claims.signature"}),
            ))
            .unwrap();
        cx.run_until_parked();
        let connection::Command::Upload { bytes, reply, .. } = requests.try_recv().unwrap() else {
            panic!("body")
        };
        assert_eq!(bytes, body);
        reply.try_send(Ok(())).unwrap();
        cx.run_until_parked();
    }
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    let connection::Command::Call { payload, reply, .. } = requests.try_recv().unwrap() else {
        panic!("attachment-only turn")
    };
    assert_eq!(payload["type"], "thread.turn.start");
    assert_eq!(
        payload["message"]["attachments"].as_array().unwrap().len(),
        2
    );
    reply
        .try_send(Err("Retain folder attachments".into()))
        .unwrap();
    cx.run_until_parked();
    app.read_with(cx, |app, _| assert_eq!(app.attachments.len(), 2));
}

#[gpui_kit::test]
fn r18_c_cancel_while_signing_cleans_allocated_resource_and_reports_cleanup_denial(
    cx: &mut TestAppContext,
) {
    let (app, cx, requests) = harness(cx);
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.environment_scope = "https://attachments.example".into();
            app.attach_inputs(
                app.attachment_scope().unwrap(),
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
        reply: signing,
        ..
    } = requests.try_recv().unwrap()
    else {
        panic!("sign")
    };
    assert_eq!(tag, "attachments.createUploadUrl");
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            let id = app.attachment_drafts.transfers[0].id.clone();
            app.cancel_transfer(&id, cx);
        })
    });
    signing.try_send(Ok(json!({"attachmentId":"pending-00000000-0000-4000-8000-000000000001-txt","relativeUrl":"/api/attachments/upload/claims.signature"}))).unwrap();
    cx.run_until_parked();
    let connection::Command::Call {
        tag,
        payload,
        reply,
    } = requests.try_recv().unwrap()
    else {
        panic!("delete, not POST")
    };
    assert_eq!(tag, "attachments.delete");
    assert_eq!(
        payload["attachmentId"],
        "pending-00000000-0000-4000-8000-000000000001-txt"
    );
    reply.try_send(Err("Cleanup denied".into())).unwrap();
    cx.run_until_parked();
    app.read_with(cx, |app, _| {
        assert!(app.attachments.is_empty());
        assert_eq!(
            app.attachment_drafts.transfers[0].phase,
            "Cancelled; cleanup failed"
        );
        assert!(!app.attachment_drafts.uploading);
        assert_eq!(app.pending_actions, 0);
    });
    assert!(requests.try_recv().is_err());
}

#[gpui_kit::test]
fn r18_c_readiness_loss_after_signing_releases_resource_instead_of_orphaning_it(
    cx: &mut TestAppContext,
) {
    let (app, cx, requests) = harness(cx);
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.environment_scope = "https://attachments.example".into();
            app.attach_inputs(
                app.attachment_scope().unwrap(),
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
    let connection::Command::Call { reply: signing, .. } = requests.try_recv().unwrap() else {
        panic!("sign")
    };
    cx.update(|_, cx| app.update(cx, |app, _| app.generation += 1));
    signing.try_send(Ok(json!({"attachmentId":"pending-00000000-0000-4000-8000-000000000001-txt","relativeUrl":"/api/attachments/upload/claims.signature"}))).unwrap();
    cx.run_until_parked();
    let connection::Command::Call { tag, reply, .. } = requests.try_recv().unwrap() else {
        panic!("delete, not POST")
    };
    assert_eq!(tag, "attachments.delete");
    reply.try_send(Ok(json!({}))).unwrap();
    cx.run_until_parked();
    app.read_with(cx, |app, _| {
        assert!(app.attachments.is_empty());
        assert_eq!(app.attachment_drafts.transfers[0].phase, "Failed");
        assert!(app.attachment_drafts.transfers[0].retry.is_some());
    });
    assert!(requests.try_recv().is_err());
}

#[gpui_kit::test]
fn r18_c_paste_as_text_bypasses_large_paste_and_is_undoable(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    let text = "large text\n".repeat(4000);
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.composer
                .update(cx, |input, cx| input.replace("prefix ", window, cx));
            cx.write_to_clipboard(gpui_kit::ClipboardItem::new_string(text.clone()));
        })
    });
    cx.simulate_keystrokes("cmd-shift-v");
    cx.run_until_parked();
    app.read_with(cx, |app, cx| {
        assert_eq!(
            app.composer.read(cx).value().as_str(),
            format!("prefix {text}")
        );
        assert!(app.composer_state.pending_paste.is_none());
        assert!(app.attachments.is_empty());
    });
    assert!(requests.try_recv().is_err());
    cx.simulate_keystrokes("cmd-z");
    app.read_with(cx, |app, cx| {
        assert_eq!(app.composer.read(cx).value().as_str(), "prefix ")
    });
}
