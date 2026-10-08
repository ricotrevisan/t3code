use super::*;
use gpui_kit::Modifiers;

#[gpui_kit::test]
fn conversation_preparation_is_reused_for_unrelated_notifications(cx: &mut TestAppContext) {
    let (app, cx, _) = harness(cx);
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.apply_thread_update("t", snapshot((0..60).map(message).collect(), "idle"), cx)
                .unwrap();
            cx.notify();
        })
    });
    cx.run_until_parked();
    let before = app.read_with(cx, |app, _| app.conversation_presentation.preparations);
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.set_status("Unrelated notification", cx);
        })
    });
    cx.run_until_parked();
    assert_eq!(
        app.read_with(cx, |app, _| app.conversation_presentation.preparations),
        before
    );
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            // A replacement snapshot may reuse the wire sequence; local revision must invalidate it.
            app.apply_thread_update("t", snapshot((0..61).map(message).collect(), "idle"), cx)
                .unwrap();
            cx.notify();
        })
    });
    cx.run_until_parked();
    assert_eq!(
        app.read_with(cx, |app, _| app.conversation_presentation.preparations),
        before + 1
    );
    assert!(cx.debug_bounds("message-m60").is_some());
}

#[gpui_kit::test]
fn conversation_reasoning_disclosure_and_message_jump_preserve_navigation(cx: &mut TestAppContext) {
    let (app, cx, _) = harness(cx);
    let mut messages: Vec<Value> = (0..60).map(message).collect();
    messages[59]["role"] = json!("reasoning");
    messages[59]["text"] = json!("Private reasoning\n\n".repeat(40));
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.apply_thread_update("t", snapshot(messages, "idle"), cx)
                .unwrap();
            cx.notify();
        })
    });
    cx.run_until_parked();
    assert!(cx.debug_bounds("expand-m59").is_some());
    let before = app.read_with(cx, |app, _| {
        app.conversation_scroll.max_offset_for_scrollbar().y
    });
    let bounds = cx.debug_bounds("expand-m59").unwrap();
    cx.simulate_click(bounds.center(), Modifiers::none());
    cx.run_until_parked();
    assert!(
        app.read_with(cx, |app, _| app
            .conversation_scroll
            .max_offset_for_scrollbar()
            .y)
            > before
    );
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.scroll_to_message("m0");
            cx.notify();
        })
    });
    cx.run_until_parked();
    assert!(cx.debug_bounds("message-m0").is_some());
    assert!(cx.debug_bounds("return-to-latest").is_some());
    let bounds = cx.debug_bounds("return-to-latest").unwrap();
    cx.simulate_click(bounds.center(), Modifiers::none());
    cx.run_until_parked();
    assert!(app.read_with(cx, |app, _| app.conversation_scroll.is_following_tail()));
}

#[gpui_kit::test]
fn conversation_plan_save_uses_selected_workspace_and_waits_for_receipt(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            let mut item = snapshot(vec![], "idle");
            item["snapshot"]["thread"]["proposedPlans"] =
                json!([{"id":"p1","planMarkdown":"# Plan\n\nBuild the feature"}]);
            app.apply_thread_update("t", item, cx).unwrap();
            app.connection_phase = connection::Phase::Ready;
            app.shell.synchronized = true;
            cx.notify();
        })
    });
    cx.run_until_parked();
    let bounds = cx.debug_bounds("plan-save").unwrap();
    cx.simulate_click(bounds.center(), Modifiers::none());
    cx.run_until_parked();
    let connection::Command::Call {
        tag,
        payload,
        reply,
    } = requests.try_recv().unwrap()
    else {
        panic!("save request")
    };
    assert_eq!(tag, "projects.writeFile");
    assert_eq!(payload["cwd"], "/tmp/fixture");
    assert_eq!(payload["contents"], "# Plan\n\nBuild the feature");
    assert!(
        payload["relativePath"]
            .as_str()
            .unwrap()
            .starts_with("plan-")
    );
    assert!(app.read_with(cx, |app, _| app.conversation_presentation.saving));
    reply
        .send_blocking(Err("read-only workspace".into()))
        .unwrap();
    cx.run_until_parked();
    assert!(!app.read_with(cx, |app, _| app.conversation_presentation.saving));
    assert!(app.read_with(cx, |app, _| app.status.contains("read-only workspace")));
}

#[gpui_kit::test]
fn conversation_attachment_download_requests_an_exact_resource(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    cx.update(|_, cx| app.update(cx, |app, cx| {
        let item = json!({"kind":"snapshot","snapshot":{"snapshotSequence":1,"thread":{"id":"t","messages":[{"id":"u","role":"user","text":"File","attachments":[{"id":"a1","type":"file","name":"notes.txt","sizeBytes":12,"mimeType":"text/plain"}]}]}}});
        app.apply_thread_update("t", item, cx).unwrap(); cx.notify();
    }));
    cx.run_until_parked();
    let bounds = cx.debug_bounds("attachment-a1").unwrap();
    cx.simulate_click(bounds.center(), Modifiers::none());
    cx.run_until_parked();
    let connection::Command::Call {
        tag,
        payload,
        reply,
    } = requests.try_recv().unwrap()
    else {
        panic!("asset request")
    };
    assert_eq!(tag, "assets.createUrl");
    assert_eq!(
        payload["resource"],
        json!({"_tag":"attachment","attachmentId":"a1","fileName":"notes.txt","mimeType":"text/plain","disposition":"attachment"})
    );
    // Invalid returned origins cannot be opened; no external application is launched by this test.
    reply
        .send_blocking(Ok(json!({"relativeUrl":"https://wrong.invalid/file"})))
        .unwrap();
    cx.run_until_parked();
    assert!(app.read_with(cx, |app, _| app.status.contains("Invalid attachment")));
}
