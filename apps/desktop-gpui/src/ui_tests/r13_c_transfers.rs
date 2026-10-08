use super::*;

fn start(app: &Entity<NativeApp>, cx: &mut VisualTestContext) {
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.environment_scope = "https://attachments.example".into();
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
}
fn prepare(
    requests: &Requests,
    cx: &mut VisualTestContext,
) -> async_channel::Sender<Result<(), String>> {
    let connection::Command::Call {
        tag,
        payload,
        reply,
    } = requests.try_recv().unwrap()
    else {
        panic!("prepare");
    };
    assert_eq!(tag, "attachments.createUploadUrl");
    assert_eq!(payload["sizeBytes"], 4);
    reply.try_send(Ok(json!({"attachmentId":"pending-00000000-0000-4000-8000-000000000001-txt","relativeUrl":"/api/attachments/upload/claims.signature"}))).unwrap();
    cx.run_until_parked();
    let connection::Command::Upload { bytes, reply, .. } = requests.try_recv().unwrap() else {
        panic!("POST");
    };
    assert_eq!(bytes, b"body");
    reply
}
#[gpui_kit::test]
fn r13_c_transfer_failed_body_can_retry_without_reselecting_source(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    start(&app, cx);
    prepare(&requests, cx)
        .try_send(Err("Upload interrupted".into()))
        .unwrap();
    cx.run_until_parked();
    let id = app.read_with(cx, |app, _| {
        assert_eq!(app.attachment_drafts.transfers[0].phase, "Failed");
        assert!(app.attachments.is_empty());
        app.attachment_drafts.transfers[0].id.clone()
    });
    cx.update(|_, cx| app.update(cx, |app, cx| app.retry_transfer(&id, cx)));
    cx.run_until_parked();
    prepare(&requests, cx).try_send(Ok(())).unwrap();
    cx.run_until_parked();
    app.read_with(cx, |app, _| {
        assert_eq!(app.attachments.len(), 1);
        assert_eq!(app.attachment_drafts.transfers[0].phase, "Attached");
        assert_eq!(app.pending_actions, 0);
    });
}
#[gpui_kit::test]
fn r13_c_transfer_cancel_inflight_deletes_resource_without_installing_ref(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    start(&app, cx);
    let receipt = prepare(&requests, cx);
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            let id = app.attachment_drafts.transfers[0].id.clone();
            app.cancel_transfer(&id, cx);
        })
    });
    receipt.try_send(Ok(())).unwrap();
    cx.run_until_parked();
    let connection::Command::Call {
        tag,
        payload,
        reply,
    } = requests.try_recv().unwrap()
    else {
        panic!("delete cancelled");
    };
    assert_eq!(tag, "attachments.delete");
    assert_eq!(
        payload["attachmentId"],
        "pending-00000000-0000-4000-8000-000000000001-txt"
    );
    reply.try_send(Ok(json!({}))).unwrap();
    cx.run_until_parked();
    app.read_with(cx, |app, _| {
        assert!(app.attachments.is_empty());
        assert_eq!(app.attachment_drafts.transfers[0].phase, "Cancelled");
        assert_eq!(app.pending_actions, 0);
    });
}
#[test]
fn r13_c_folder_drop_expands_sorted_files_and_rejects_over_limit() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("nested")).unwrap();
    std::fs::write(dir.path().join("b.txt"), "b").unwrap();
    std::fs::write(dir.path().join("nested/a.txt"), "a").unwrap();
    let files = crate::app::attachments::expand_paths(vec![dir.path().to_owned()], 2).unwrap();
    assert_eq!(files.len(), 2);
    assert!(crate::app::attachments::expand_paths(vec![dir.path().to_owned()], 1).is_err());
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(dir.path(), dir.path().join("nested/loop")).unwrap();
        assert!(crate::app::attachments::expand_paths(vec![dir.path().to_owned()], 100).is_err());
    }
}
