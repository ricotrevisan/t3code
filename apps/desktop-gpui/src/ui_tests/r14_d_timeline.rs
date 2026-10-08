use super::*;

fn click(cx: &mut VisualTestContext, selector: &'static str) {
    let bounds = cx
        .debug_bounds(selector)
        .unwrap_or_else(|| panic!("missing {selector}"));
    cx.simulate_click(bounds.center(), gpui_kit::Modifiers::none());
    cx.run_until_parked();
}

#[gpui_kit::test]
fn r14_d_quote_affordance_requires_selection_and_clears_after_capture(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    cx.update(|_, cx| app.update(cx, |app, cx| {
        app.draft_store = None;
        app.accept_config_snapshot(json!({"environment":{"environmentId":"env-fixture"}}), cx);
        app.apply_thread_update("t", snapshot(vec![json!({"id":"answer","role":"assistant","text":"A selectable answer","streaming":false})], "idle"), cx).unwrap();
        cx.notify();
    }));
    cx.run_until_parked();
    assert!(cx.debug_bounds("quote-selection-answer").is_none());
    let state = app.read_with(cx, |app, _| {
        app.conversation_presentation.sources["answer"]
            .state
            .clone()
    });
    cx.update(|window, cx| {
        let focus = state.read(cx).focus_handle().clone();
        focus.focus(window, cx);
    });
    cx.simulate_keystrokes("cmd-a");
    cx.run_until_parked();
    click(cx, "quote-selection-answer");
    app.read_with(cx, |app, _| {
        assert_eq!(
            app.draft_context_records().len(),
            1,
            "quote capture status: {}",
            app.status
        );
        assert_eq!(
            app.draft_context_records()[0].value()["payload"]["text"],
            "A selectable answer"
        );
    });
    assert!(cx.debug_bounds("quote-selection-answer").is_none());
    assert!(
        requests.try_recv().is_err(),
        "selection capture is local, not a send"
    );
}

#[gpui_kit::test]
fn r14_d_minimap_pointer_and_keyboard_jump_unsampled_history(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    cx.update(|_, cx| app.update(cx, |app, cx| {
        app.apply_thread_update("t", snapshot((0..100).map(|i| json!({"id":format!("u{i}"),"role":"user","text":format!("Prompt {i}"),"createdAt":format!("2026-10-01T00:{:02}:{:02}Z", i / 60, i % 60)})).collect(), "idle"), cx).unwrap();
        cx.notify();
    }));
    cx.run_until_parked();
    click(cx, "timeline-minimap");
    assert!(cx.debug_bounds("message-u50").is_some());
    cx.simulate_keystrokes("end enter");
    cx.run_until_parked();
    assert!(
        cx.debug_bounds("message-u99").is_some(),
        "active {:?}",
        app.read_with(cx, |app, _| app.conversation_presentation.minimap_active)
    );
    cx.simulate_keystrokes("home enter");
    cx.run_until_parked();
    assert!(cx.debug_bounds("message-u0").is_some());
    click(cx, "return-to-latest");
    assert!(app.read_with(cx, |app, _| app.conversation_scroll.is_following_tail()));
    assert!(requests.try_recv().is_err());
}

#[gpui_kit::test]
fn r14_d_artifact_use_preserves_draft_and_send_failure_retains_prompt(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    cx.update(|window, cx| app.update(cx, |app, cx| {
        app.draft_store = None;
        app.provider_catalog = vec![composer_parity::provider_fixture("codex", "Codex")];
        app.apply_thread_update("t", snapshot(vec![json!({"id":"template","role":"assistant","text":"::artifact-template{artifact_kind=\"document\" display_name=\"Report\" skill_directory=\"/skills/report\" skill_name=\"artifact-template-report\"}","streaming":false})], "idle"), cx).unwrap();
        app.composer.update(cx, |input, cx| input.set_value("Keep this draft.", window, cx));
        cx.notify();
    }));
    cx.run_until_parked();
    click(cx, "artifact-template-use-0");
    let expected = "Keep this draft. Create a document using this $artifact-template-report about…";
    assert_eq!(
        app.read_with(cx, |app, cx| app.composer.read(cx).value().to_string()),
        expected
    );
    click(cx, "artifact-template-use-0");
    assert_eq!(
        app.read_with(cx, |app, cx| app.composer.read(cx).value().to_string()),
        expected
    );
    click(cx, "send");
    let connection::Command::Call {
        tag,
        payload,
        reply,
    } = requests.try_recv().unwrap()
    else {
        panic!("start turn");
    };
    assert_eq!(tag, "orchestration.dispatchCommand");
    assert!(
        payload["message"]["text"]
            .as_str()
            .unwrap()
            .starts_with("Keep this draft. Create a document using this ")
    );
    assert_eq!(payload["message"]["context"]["records"][0]["kind"], "skill");
    assert_eq!(
        payload["message"]["context"]["records"][0]["name"],
        "artifact-template-report"
    );
    reply.send_blocking(Err("offline".into())).unwrap();
    cx.run_until_parked();
    assert_eq!(
        app.read_with(cx, |app, cx| app.composer.read(cx).value().to_string()),
        expected
    );
}

#[gpui_kit::test]
fn r14_d_restored_plan_text_requires_new_mode_receipt_and_preserves_source_on_send(
    cx: &mut TestAppContext,
) {
    let (app, cx, requests) = harness(cx);
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.draft_store = None;
            app.provider_catalog = vec![composer_parity::provider_fixture("codex", "Codex")];
            let mut update = snapshot(vec![], "idle");
            update["snapshot"]["thread"]["proposedPlans"] =
                json!([{"id":"plan-source","planMarkdown":"# Plan\n\nShip it"}]);
            app.apply_thread_update("t", update, cx).unwrap();
            app.composer.update(cx, |input, cx| {
                input.set_value(
                    "Implement the following plan:\n\n# Plan\n\nShip it",
                    window,
                    cx,
                )
            });
            cx.notify();
        })
    });
    cx.run_until_parked();
    click(cx, "send");
    assert!(
        requests.try_recv().is_err(),
        "text restoration cannot invent a mode receipt"
    );
    click(cx, "plan-draft-reconfirm");
    let connection::Command::Call { payload, reply, .. } = requests.try_recv().unwrap() else {
        panic!("mode confirmation");
    };
    assert_eq!(payload["type"], "thread.interaction-mode.set");
    assert_eq!(payload["interactionMode"], "default");
    reply.send_blocking(Err("denied".into())).unwrap();
    cx.run_until_parked();
    click(cx, "send");
    assert!(requests.try_recv().is_err());
    click(cx, "plan-draft-reconfirm");
    let connection::Command::Call { reply, .. } = requests.try_recv().unwrap() else {
        panic!("retry mode");
    };
    reply.send_blocking(Ok(json!({}))).unwrap();
    cx.run_until_parked();
    click(cx, "send");
    let connection::Command::Call { payload, reply, .. } = requests.try_recv().unwrap() else {
        panic!("start turn");
    };
    assert_eq!(
        payload["sourceProposedPlan"],
        json!({"threadId":"t","planId":"plan-source"})
    );
    assert_eq!(payload["interactionMode"], "default");
    reply.send_blocking(Err("offline".into())).unwrap();
    cx.run_until_parked();
    assert!(app.read_with(cx, |app, cx| {
        app.composer
            .read(cx)
            .value()
            .starts_with("Implement the following plan:")
    }));
}

#[gpui_kit::test]
fn r14_d_error_dismissal_is_exact_and_new_failure_remains_visible(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    let set_error = |error: &str, cx: &mut VisualTestContext| {
        cx.update(|_, cx| {
            app.update(cx, |app, cx| {
                let mut update = snapshot(vec![], "idle");
                update["snapshot"]["thread"]["session"]["lastError"] = json!(error);
                app.apply_thread_update("t", update, cx).unwrap();
                cx.notify();
            })
        });
        cx.run_until_parked();
    };
    set_error("Provider denied request", cx);
    click(cx, "thread-error-dismiss");
    assert!(cx.debug_bounds("thread-error-banner").is_none());
    set_error("Different failure", cx);
    assert!(cx.debug_bounds("thread-error-banner").is_some());
    set_error("Provider denied request", cx);
    assert!(cx.debug_bounds("thread-error-banner").is_none());
    assert!(
        requests.try_recv().is_err(),
        "dismissal is session-local, not error deletion"
    );
}

#[gpui_kit::test]
fn r14_d_work_disclosure_is_compact_left_aligned_and_reversible(cx: &mut TestAppContext) {
    let (app, cx, _) = harness(cx);
    cx.update(|_, cx| app.update(cx, |app, cx| {
        let mut update = snapshot(vec![], "idle");
        update["snapshot"]["thread"]["activities"] = json!([
            {"id":"command","turnId":"turn","kind":"tool.completed","summary":"shell completed","createdAt":"2026-10-01T00:00:00Z","payload":{"title":"shell","toolCallId":"cmd","detail":"echo ok"}}
        ]);
        app.apply_thread_update("t", update, cx).unwrap();
        cx.notify();
    }));
    cx.run_until_parked();
    let button = cx.debug_bounds("activity-toggle").unwrap();
    let conversation = cx.debug_bounds("conversation").unwrap();
    assert!(
        button.origin.x < conversation.origin.x + px(64.),
        "work row begins on the message spine"
    );
    assert!(
        button.size.width < conversation.size.width / 2.,
        "compact disclosure does not stretch across the column"
    );
    click(cx, "activity-toggle");
    assert!(cx.debug_bounds("tool-row-command").is_some());
    click(cx, "activity-toggle");
    assert!(cx.debug_bounds("tool-row-command").is_none());
}
