use super::*;
use t3code_desktop_gpui::draft_context::DraftContextRecord;

#[gpui_kit::test]
fn r13_c_context_clipboard_remints_owned_payload_and_atomic_backspace(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    let record = DraftContextRecord::terminal("Build", "t", "Build", 1, 1, "capture").unwrap();
    let original = record.id().to_owned();
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.provider_catalog = vec![super::composer_parity::provider_fixture(
                "codex", "Personal",
            )];
            let environment = app.environment_scope.clone();
            let thread = app.selected_thread.clone().unwrap();
            app.intake_draft_context(&environment, &thread, record, window, cx)
                .unwrap();
        })
    });
    cx.run_until_parked();
    cx.simulate_keystrokes("cmd-a cmd-c backspace cmd-v");
    cx.run_until_parked();
    app.read_with(cx, |app, cx| {
        let records = app.draft_context_records();
        assert_eq!(records.len(), 2);
        assert_ne!(records[1].id(), original);
        assert_eq!(records[1].value()["text"], "capture");
        assert!(app.composer.read(cx).value().contains(records[1].id()));
    });
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    let connection::Command::Call {
        tag,
        payload,
        reply,
    } = requests.try_recv().unwrap()
    else {
        panic!("pasted context turn");
    };
    assert_eq!(tag, "orchestration.dispatchCommand");
    assert_eq!(
        payload["message"]["context"]["records"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_ne!(
        payload["message"]["context"]["records"][0]["contextId"],
        original
    );
    reply.try_send(Err("Preserve for edit".into())).unwrap();
    cx.run_until_parked();
    cx.simulate_keystrokes("backspace backspace");
    cx.run_until_parked();
    app.read_with(cx, |app, cx| {
        assert!(app.composer.read(cx).value().is_empty())
    });
}

#[gpui_kit::test]
fn r13_c_context_intake_transport_rejection_retains_owned_snapshot(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    let record =
        DraftContextRecord::terminal("Build output", "term-1", "Build", 3, 4, "captured\ntext")
            .unwrap();
    let expected = record.value().clone();
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.provider_catalog = vec![super::composer_parity::provider_fixture(
                "codex", "Personal",
            )];
            let environment = app.environment_scope.clone();
            let thread = app.selected_thread.clone().unwrap();
            assert!(
                app.intake_draft_context("stale", &thread, record.clone(), window, cx)
                    .is_err()
            );
            app.intake_draft_context(&environment, &thread, record, window, cx)
                .unwrap();
        })
    });
    cx.run_until_parked();
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    let connection::Command::Call {
        tag,
        payload,
        reply,
    } = requests.try_recv().unwrap()
    else {
        panic!("turn");
    };
    assert_eq!(tag, "orchestration.dispatchCommand");
    assert_eq!(payload["type"], "thread.turn.start");
    assert_eq!(payload["message"]["context"]["records"][0], expected);
    assert!(
        payload["message"]["text"]
            .as_str()
            .unwrap()
            .contains(expected["contextId"].as_str().unwrap())
    );
    reply.try_send(Err("Rejected context".into())).unwrap();
    cx.run_until_parked();
    app.read_with(cx, |app, cx| {
        assert_eq!(app.draft_context_records()[0].value(), &expected);
        assert!(!app.composer.read(cx).value().is_empty());
    });
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.remove_draft_context(expected["contextId"].as_str().unwrap(), window, cx);
            assert!(app.composer.read(cx).value().trim().is_empty());
            // Editor undo retains the payload, not just a dead URL.
            assert_eq!(app.draft_context_records()[0].value(), &expected);
        })
    });
}

#[gpui_kit::test]
fn r13_c_context_queue_capture_isolated_from_later_draft_and_retry(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    let first = DraftContextRecord::terminal("First", "t", "Terminal", 1, 1, "first").unwrap();
    let expected = first.value().clone();
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.provider_catalog = vec![super::composer_parity::provider_fixture(
                "codex", "Personal",
            )];
            let environment = app.environment_scope.clone();
            let thread = app.selected_thread.clone().unwrap();
            app.intake_draft_context(&environment, &thread, first, window, cx)
                .unwrap();
            app.queue_prompt(false, window, cx);
            let later =
                DraftContextRecord::terminal("Later", "t", "Terminal", 2, 2, "later").unwrap();
            app.intake_draft_context(&environment, &thread, later, window, cx)
                .unwrap();
            let queued = app.queues.messages(&thread)[0].id;
            app.send_queued(&thread, queued, cx);
        })
    });
    cx.run_until_parked();
    let connection::Command::Subscribe { tag, payload, .. } = requests.try_recv().unwrap() else {
        panic!("queue detail subscription");
    };
    assert_eq!(tag, "orchestration.subscribeThread");
    assert_eq!(payload["reasoningMessages"], false);
    let connection::Command::Call {
        tag,
        payload,
        reply,
    } = requests.try_recv().unwrap()
    else {
        panic!("queued turn");
    };
    assert_eq!(tag, "orchestration.dispatchCommand");
    assert_eq!(payload["message"]["context"]["records"], json!([expected]));
    reply.try_send(Err("Retry later".into())).unwrap();
    cx.run_until_parked();
    app.read_with(cx, |app, cx| {
        let thread = app.selected_thread.as_deref().unwrap();
        assert!(app.queues.held(thread));
        assert_eq!(
            app.queues
                .context(thread, app.queues.messages(thread)[0].id)
                .unwrap()["records"][0],
            expected
        );
        assert_eq!(app.draft_context_records().len(), 1);
        assert_eq!(app.draft_context_records()[0].label(), "Later");
        assert!(app.composer.read(cx).value().contains("Later"));
    });
}
