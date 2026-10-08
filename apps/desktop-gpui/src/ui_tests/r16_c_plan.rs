use super::*;
use t3code_desktop_gpui::drafts::{Drafts, PlanIntent};

fn click(cx: &mut VisualTestContext, selector: &'static str) {
    let bounds = cx
        .debug_bounds(selector)
        .unwrap_or_else(|| panic!("missing {selector}"));
    cx.simulate_click(bounds.center(), gpui_kit::Modifiers::none());
    cx.run_until_parked();
}

#[gpui_kit::test]
fn r16_c_durable_edited_cross_thread_plan_requires_fresh_receipt_and_retains_rejection(
    cx: &mut TestAppContext,
) {
    let (app, cx, requests) = harness(cx);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("drafts.json");
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.provider_catalog = vec![composer_parity::provider_fixture("codex", "Codex")];
            app.draft_store = Some(Drafts::open(path.clone()).unwrap());
            let prompt = "Implement the following plan:\n\nEdited after stashing 世界";
            app.composer
                .update(cx, |input, cx| input.set_value(prompt, window, cx));
            app.capture_plan_intent(Some(PlanIntent::new(
                app.environment_scope.clone(),
                "other-source-thread".into(),
                "exact-plan-id".into(),
                "default".into(),
                prompt.into(),
            )));
            app.remember_composer_draft(cx);
            // Persist/reopen the real store, with no live plan or mode receipt.
            app.draft_store
                .as_ref()
                .unwrap()
                .write_snapshot()
                .unwrap()
                .persist()
                .unwrap();
            app.draft_store = Some(Drafts::open(path.clone()).unwrap());
            app.composer_state.plan_intents.clear();
            app.conversation_presentation.plan_followup = None;
            cx.notify();
        })
    });
    cx.run_until_parked();
    click(cx, "send");
    assert!(requests.try_recv().is_err());
    click(cx, "durable-plan-confirm");
    let connection::Command::Call {
        tag,
        payload,
        reply,
    } = requests.try_recv().unwrap()
    else {
        panic!("mode")
    };
    assert_eq!(tag, "orchestration.dispatchCommand");
    assert_eq!(payload["type"], "thread.interaction-mode.set");
    assert_eq!(payload["threadId"], "t");
    assert_eq!(payload["interactionMode"], "default");
    reply.try_send(Err("denied".into())).unwrap();
    cx.run_until_parked();
    click(cx, "send");
    assert!(requests.try_recv().is_err());
    click(cx, "durable-plan-confirm");
    let connection::Command::Call { reply, .. } = requests.try_recv().unwrap() else {
        panic!("retry")
    };
    reply.try_send(Ok(json!({}))).unwrap();
    cx.run_until_parked();
    click(cx, "send");
    let connection::Command::Call { payload, reply, .. } = requests.try_recv().unwrap() else {
        panic!("send")
    };
    assert_eq!(payload["type"], "thread.turn.start");
    assert_eq!(
        payload["sourceProposedPlan"],
        json!({"threadId":"other-source-thread","planId":"exact-plan-id"})
    );
    reply.try_send(Err("not accepted".into())).unwrap();
    cx.run_until_parked();
    app.read_with(cx, |app, cx| {
        assert!(app.durable_plan_intent().is_some());
        assert!(
            app.composer
                .read(cx)
                .value()
                .contains("Edited after stashing")
        );
    });
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.generation += 1;
            cx.notify();
        })
    });
    cx.run_until_parked();
    click(cx, "send");
    assert!(
        requests.try_recv().is_err(),
        "old receipt cannot authorize a new epoch"
    );
    click(cx, "durable-plan-discard");
    app.read_with(cx, |app, cx| {
        assert!(app.durable_plan_intent().is_none());
        assert!(
            app.composer
                .read(cx)
                .value()
                .contains("Edited after stashing")
        );
    });
}
