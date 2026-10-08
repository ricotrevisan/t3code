use super::*;
use t3code_desktop_gpui::drafts::{Drafts, PlanIntent};

#[gpui_kit::test]
fn r16_c_interrupted_plan_queue_restores_for_review_without_replaying(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("drafts.json");
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.provider_catalog = vec![composer_parity::provider_fixture("codex", "Codex")];
            app.draft_store = Some(Drafts::open(path.clone()).unwrap());
            app.composer.update(cx, |input, cx| {
                input.set_value("Implement edited plan", window, cx)
            });
            app.capture_plan_intent(Some(PlanIntent::new(
                app.environment_scope.clone(),
                "source".into(),
                "plan".into(),
                "default".into(),
                "Implement edited plan".into(),
            )));
            app.confirm_durable_plan(cx);
        })
    });
    cx.run_until_parked();
    let connection::Command::Call { payload, reply, .. } = requests.try_recv().unwrap() else {
        panic!("mode")
    };
    assert_eq!(payload["type"], "thread.interaction-mode.set");
    reply.try_send(Ok(json!({}))).unwrap();
    cx.run_until_parked();
    cx.update(|_, cx| app.update(cx, |app, cx| {
        app.apply_thread_update("t", json!({"kind":"snapshot","snapshot":{"snapshotSequence":1,"thread":{"id":"t","messages":[],"activities":[],"latestTurn":{"turnId":"old","state":"running"},"session":{"status":"running"}}}}), cx).unwrap();
        cx.notify();
    }));
    cx.run_until_parked();
    cx.update(|window, cx| app.update(cx, |app, cx| app.queue_prompt(false, window, cx)));
    cx.run_until_parked();
    let recovery = app.read_with(cx, |app, _| {
        let message = &app.queues.messages("t")[0];
        assert_eq!(
            app.queues.source_plan("t", message.id),
            Some(json!({"threadId":"source","planId":"plan"}))
        );
        let id = app.queues.recovery_stash("t", message.id).unwrap();
        assert!(
            app.composer_stashes().is_empty(),
            "active queue recovery cannot be restored twice"
        );
        app.draft_store
            .as_ref()
            .unwrap()
            .write_snapshot()
            .unwrap()
            .persist()
            .unwrap();
        id
    });
    while let Ok(command) = requests.try_recv() {
        assert!(
            !matches!(command, connection::Command::Call { .. }),
            "queue waits for boundary"
        );
    }
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            let id = app.queues.messages("t")[0].id;
            app.send_queued("t", id, cx);
        })
    });
    cx.run_until_parked();
    let connection::Command::Call { payload, reply, .. } = requests.try_recv().unwrap() else {
        panic!("queued dispatch")
    };
    assert_eq!(
        payload["sourceProposedPlan"],
        json!({"threadId":"source","planId":"plan"})
    );
    assert_eq!(payload["interactionMode"], "default");
    reply.try_send(Err("denied".into())).unwrap();
    cx.run_until_parked();
    app.read_with(cx, |app, _| {
        assert_eq!(app.queues.messages("t").len(), 1);
        assert!(
            app.draft_store
                .as_ref()
                .unwrap()
                .stashes(&app.environment_scope)
                .iter()
                .any(|entry| entry.id == recovery)
        );
    });
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.queues = Default::default();
            app.composer_state.plan_confirmations.clear();
            app.composer_state.plan_intents.clear();
            app.draft_store = Some(Drafts::open(path.clone()).unwrap());
            let entries = app.composer_stashes();
            assert_eq!(entries.len(), 1);
            assert_eq!(entries[0].queued_for.as_deref(), Some("t"));
            app.restore_stash(&recovery, window, cx);
        })
    });
    cx.run_until_parked();
    app.read_with(cx, |app, cx| {
        assert_eq!(app.durable_plan_intent().unwrap().source_thread, "source");
        assert_eq!(
            app.composer.read(cx).value().as_str(),
            "Implement edited plan"
        );
        assert!(app.durable_plan_mode("Implement edited plan").is_err());
        assert!(app.queues.messages("t").is_empty());
    });
    assert!(
        requests.try_recv().is_err(),
        "restart recovery never automatically resends a possibly accepted message"
    );
}
