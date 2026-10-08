use super::*;

fn click(cx: &mut VisualTestContext, selector: &'static str) {
    let bounds = cx
        .debug_bounds(selector)
        .unwrap_or_else(|| panic!("missing {selector}"));
    cx.simulate_click(bounds.center(), gpui_kit::Modifiers::none());
    cx.run_until_parked();
}

#[gpui_kit::test]
fn r13_d2_assistant_keyboard_selection_intake_send_failure_retains_owned_quote(
    cx: &mut TestAppContext,
) {
    let (app, cx, requests) = harness(cx);
    cx.update(|_, cx| app.update(cx, |app, cx| {
        app.draft_store = None;
        app.accept_config_snapshot(json!({"environment":{"environmentId":"env-fixture"}}), cx);
        app.provider_catalog = vec![composer_parity::provider_fixture("codex", "Codex")];
        app.apply_thread_update("t", snapshot(vec![json!({"id":"answer","role":"assistant","text":"Hello **😀 world**","streaming":false})], "idle"), cx).unwrap();
        cx.notify();
    }));
    cx.run_until_parked();
    let state = app.read_with(cx, |app, _| {
        app.conversation_presentation.sources["answer"]
            .state
            .clone()
    });
    cx.update(|window, cx| {
        let focus = state.read(cx).focus_handle().clone();
        focus.focus(window, cx);
    });
    cx.simulate_keystrokes("cmd-a cmd-shift-l");
    cx.run_until_parked();
    app.read_with(cx, |app, cx| {
        let owned = app.draft_context_records();
        assert_eq!(owned.len(), 1);
        assert_eq!(owned[0].kind(), "assistant-citation");
        assert_eq!(owned[0].value()["payload"]["messageId"], "answer");
        assert_eq!(owned[0].value()["payload"]["environmentId"], "env-fixture");
        assert_eq!(owned[0].value()["payload"]["text"], "Hello 😀 world");
        assert!(
            app.composer
                .read(cx)
                .value()
                .contains("t3-context://v1/assistant-citation/")
        );
    });
    cx.simulate_input("Explain this quote");
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    let connection::Command::Call {
        tag,
        payload,
        reply,
    } = requests.try_recv().unwrap()
    else {
        panic!("send");
    };
    assert_eq!(tag, "orchestration.dispatchCommand");
    assert_eq!(
        payload["message"]["context"]["records"][0]["kind"],
        "assistant-citation"
    );
    assert_eq!(
        payload["message"]["context"]["records"][0]["payload"]["text"],
        "Hello 😀 world"
    );
    reply.send_blocking(Err("offline".into())).unwrap();
    cx.run_until_parked();
    app.read_with(cx, |app, cx| {
        assert_eq!(app.draft_context_records().len(), 1);
        assert!(app.composer.read(cx).value().contains("t3-context://"));
    });
}

#[gpui_kit::test]
fn r13_d2_navigator_unfolds_arbitrary_message_and_return_to_latest(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    cx.update(|_, cx| app.update(cx, |app, cx| {
        app.apply_thread_update("t", snapshot((0..40).map(|i| json!({"id":format!("u{i}"),"role":"user","text":format!("Prompt {i}"),"createdAt":format!("2026-10-01T00:{i:02}:00Z")})).collect(), "idle"), cx).unwrap();
        cx.notify();
    }));
    cx.run_until_parked();
    click(cx, "timeline-navigator-toggle");
    click(cx, "timeline-jump-u0");
    assert!(cx.debug_bounds("message-u0").is_some());
    assert!(cx.debug_bounds("return-to-latest").is_some());
    click(cx, "return-to-latest");
    assert!(app.read_with(cx, |app, _| app.conversation_scroll.is_following_tail()));
    assert!(
        requests.try_recv().is_err(),
        "full subscribed history navigation is local"
    );
}

#[gpui_kit::test]
fn r13_d2_setup_cancel_waits_for_server_state_and_disconnect_disables_entry(
    cx: &mut TestAppContext,
) {
    let (app, cx, requests) = harness(cx);
    cx.update(|_, cx| app.update(cx, |app, cx| {
        workspaces::apply_setup(&mut app.setup_progress, "t", json!({"threadId":"t","phase":"running","branch":null,"worktreePath":null,"stages":[{"id":"setup-script","status":"running","percent":null,"detail":"Installing","tail":["last output"]}],"error":null,"sequence":1})).unwrap();
        cx.notify();
    }));
    cx.run_until_parked();
    click(cx, "timeline-setup-cancel");
    let connection::Command::Call {
        tag,
        payload,
        reply,
    } = requests.try_recv().unwrap()
    else {
        panic!("cancel setup");
    };
    assert_eq!(tag, "worktreeSetup.cancel");
    assert_eq!(payload, json!({"threadId":"t"}));
    reply.send_blocking(Ok(json!({"cancelled":false}))).unwrap();
    cx.run_until_parked();
    assert_eq!(
        app.read_with(cx, |app, _| app.selected_setup().unwrap().phase),
        "running"
    );
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.apply_connection_phase(connection::Phase::Offline, cx)
        })
    });
    cx.run_until_parked();
    click(cx, "timeline-setup-cancel");
    assert!(requests.try_recv().is_err());
}

#[gpui_kit::test]
fn r13_d2_text_preview_signing_failure_retry_and_close_keep_exact_resource(
    cx: &mut TestAppContext,
) {
    let (app, cx, requests) = harness(cx);
    cx.update(|_, cx| app.update(cx, |app, cx| {
        app.apply_thread_update("t", snapshot(vec![json!({"id":"file","role":"user","text":"Inspect file","attachments":[{"type":"file","id":"report","name":"report.csv","mimeType":"text/csv","sizeBytes":42}]})], "idle"), cx).unwrap();
        cx.notify();
    }));
    cx.run_until_parked();
    click(cx, "file-text-preview");
    let connection::Command::Call {
        tag,
        payload,
        reply,
    } = requests.try_recv().unwrap()
    else {
        panic!("signing");
    };
    assert_eq!(tag, "assets.createUrl");
    assert_eq!(
        payload,
        json!({"resource":{"_tag":"attachment","attachmentId":"report","fileName":"report.csv","mimeType":"text/csv","disposition":"inline"}})
    );
    reply.send_blocking(Err("missing file".into())).unwrap();
    cx.run_until_parked();
    click(cx, "file-text-preview-retry");
    let connection::Command::Call {
        payload: retry,
        reply,
        ..
    } = requests.try_recv().unwrap()
    else {
        panic!("retry signing");
    };
    assert_eq!(retry, payload);
    click(cx, "file-text-preview");
    reply.send_blocking(Err("late failure".into())).unwrap();
    cx.run_until_parked();
    assert!(app.read_with(cx, |app, _| {
        app.conversation_presentation.text_media.is_empty()
    }));
    assert!(cx.debug_bounds("file-text-preview-retry").is_none());
}

#[gpui_kit::test]
fn r13_d2_agent_identity_progress_is_projected_and_disclosed_independently(
    cx: &mut TestAppContext,
) {
    let (app, cx, _) = harness(cx);
    cx.update(|_, cx| app.update(cx, |app, cx| {
        let mut initial = snapshot(vec![], "running");
        initial["snapshot"]["thread"]["backgroundLiveness"] = json!({"active":true});
        initial["snapshot"]["thread"]["activities"] = json!([
            {"id":"start","turnId":"turn","kind":"task.started","createdAt":"2026-10-01T00:00:00Z","summary":"Started reviewer","payload":{"taskId":"reviewer","agentKind":"agent","title":"Reviewer","model":"gpt-5.4","role":"review"}},
            {"id":"progress","turnId":"turn","kind":"task.progress","createdAt":"2026-10-01T00:00:01Z","summary":"Reviewing files","payload":{"taskId":"reviewer","agentKind":"agent","summary":"Reviewing two files","status":"running"}}
        ]);
        app.apply_thread_update("t", initial, cx).unwrap();
        cx.notify();
    }));
    cx.run_until_parked();
    click(cx, "activity-toggle");
    click(cx, "timeline-agent-reviewer");
    assert!(app.read_with(
        cx,
        |app, _| app.activity_expanded.get("agent-detail:reviewer") == Some(&true)
    ));
    app.read_with(cx, |app, _| {
        let agent = &app.conversation_presentation.agents[0];
        assert_eq!(agent.title, "Reviewer");
        assert_eq!(agent.progress.as_deref(), Some("Reviewing two files"));
        assert_eq!(agent.model.as_deref(), Some("gpt-5.4"));
    });
    click(cx, "timeline-agent-reviewer");
    assert!(app.read_with(
        cx,
        |app, _| app.activity_expanded.get("agent-detail:reviewer") == Some(&false)
    ));
}

#[gpui_kit::test]
fn r13_d2_received_citation_source_jump_and_unavailable_scope_are_explicit(
    cx: &mut TestAppContext,
) {
    let (app, cx, _) = harness(cx);
    cx.update(|_, cx| app.update(cx, |app, cx| {
        app.accept_config_snapshot(json!({"environment":{"environmentId":"env-fixture"}}), cx);
        app.apply_thread_update("t", snapshot(vec![
            json!({"id":"answer","role":"assistant","text":"the original quote","streaming":false}),
            json!({"id":"quoted","role":"user","text":"Explain","context":{"version":1,"records":[{"version":1,"kind":"assistant-citation","contextId":"quote","label":"Quote","payload":{"version":1,"environmentId":"env-fixture","threadId":"t","messageId":"answer","text":"original","start":4,"end":12,"prefix":"the ","suffix":" quote"}}]}})
        ], "idle"), cx).unwrap();
        cx.notify();
    }));
    cx.run_until_parked();
    click(cx, "context-toggle-quoted-quote");
    click(cx, "citation-open-source");
    assert!(app.read_with(cx, |app, _| {
        app.conversation_presentation.citation_target.is_none()
    }));
    assert!(cx.debug_bounds("message-answer").is_some());
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.jump_to_citation(
                &json!({"environmentId":"other","threadId":"t","messageId":"answer"}),
                cx,
            );
        })
    });
    assert!(app.read_with(cx, |app, _| {
        app.status.contains("another environment or thread")
    }));
}
