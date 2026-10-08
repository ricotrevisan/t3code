//! Real right-panel interactions and server-script receipts.
use super::*;
fn roster(app: &Entity<NativeApp>, cx: &mut VisualTestContext, workflow: bool) {
    cx.update(|_,cx|app.update(cx,|app,cx|{
        app.detail.thread=Some(json!({"id":"t","title":"Thread","messages":[{"id":"u","role":"user","text":"Work","turnId":null,"createdAt":"2026-01-01T00:00:00Z","updatedAt":"2026-01-01T00:00:00Z"}],"activities":[{"id":"spawn","kind":"task.completed","tone":"info","summary":"Done","turnId":"turn-1","createdAt":"2026-01-01T00:00:01Z","payload":{"taskId":"child","agentKind":"agent","title":"Research","taskType":if workflow {"local_workflow"} else {"subagent"},"summary":"Found answer","runHandles":{"scriptPath":"/tmp/workflow.js"}}}],"latestTurn":{"turnId":"turn-1","state":"completed","requestedAt":"2026-01-01T00:00:00Z","completedAt":"2026-01-01T00:00:01Z"},"session":null,"proposedPlans":[],"checkpoints":[]}).into());
        cx.notify();
    }));
    cx.run_until_parked();
    let tab = cx.debug_bounds("right-tab-agents").unwrap();
    cx.simulate_click(tab.center(), Default::default());
    cx.run_until_parked();
}
#[gpui_kit::test]
fn agent_panel_timeline_link_expands_the_owning_activity(cx: &mut TestAppContext) {
    let (app, cx, _requests) = harness(cx);
    roster(&app, cx, false);
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.activity_expanded.insert("fold:turn-1".into(), true);
            cx.notify();
        })
    });
    cx.run_until_parked();
    let link = cx
        .debug_bounds("agent-timeline-child")
        .expect("real agent offers a timeline link");
    cx.simulate_click(link.center(), Default::default());
    cx.run_until_parked();
    app.read_with(cx, |app, _| {
        assert_eq!(app.activity_expanded.get("turn-1"), Some(&true));
        assert!(!app.activity_expanded.contains_key("fold:turn-1"));
        assert_eq!(app.selected_thread.as_deref(), Some("t"));
    });
    // A different selected thread must not expose the old roster or its links.
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.selected_thread = Some("other".into());
            cx.notify();
        })
    });
    cx.run_until_parked();
    assert!(cx.debug_bounds("agent-timeline-child").is_none());
}
#[gpui_kit::test]
fn workflow_disclosure_keeps_its_shape_when_a_live_run_settles(cx: &mut TestAppContext) {
    let (app, cx, _requests) = harness(cx);
    roster(&app, cx, true);
    // Reconstruct a live activation using the same persisted workflow identity.
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.agents_surface.open.borrow_mut().clear();
            let mut thread = app.detail.thread.as_ref().unwrap().as_ref().clone();
            thread["session"] = json!({"status":"running"});
            thread["latestTurn"]["state"] = json!("running");
            thread["latestTurn"]["completedAt"] = Value::Null;
            thread["activities"][0]["kind"] = json!("task.started");
            app.apply_thread_update("t", json!({"kind":"snapshot","snapshot":{"snapshotSequence":2,"thread":thread}}), cx).unwrap();
            cx.notify();
        })
    });
    cx.run_until_parked();
    assert!(cx.debug_bounds("agent-timeline-child").is_some());
    // Completion must not automatically collapse a workflow the user is reading.
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            let mut thread = app.detail.thread.as_ref().unwrap().as_ref().clone();
            thread["activities"][0]["kind"] = json!("task.completed");
            thread["session"] = Value::Null;
            thread["latestTurn"]["state"] = json!("completed");
            thread["latestTurn"]["completedAt"] = json!("2026-01-01T00:00:01Z");
            app.apply_thread_update("t", json!({"kind":"snapshot","snapshot":{"snapshotSequence":3,"thread":thread}}), cx).unwrap();
            cx.notify();
        })
    });
    cx.run_until_parked();
    assert!(cx.debug_bounds("agent-timeline-child").is_some());
    let toggle = cx.debug_bounds("workflow-child").unwrap();
    cx.simulate_click(toggle.center(), Default::default());
    cx.run_until_parked();
    assert!(cx.debug_bounds("agent-timeline-child").is_none());
}
#[gpui_kit::test]
fn workflow_script_is_contained_rpc_and_late_receipt_cannot_restore_closed_view(
    cx: &mut TestAppContext,
) {
    let (app, cx, requests) = harness(cx);
    roster(&app, cx, true);
    // Settled workflows default collapsed; open through the actual control.
    let toggle = cx.debug_bounds("workflow-child").unwrap();
    cx.simulate_click(toggle.center(), Default::default());
    cx.run_until_parked();
    let load = cx.debug_bounds("workflow-script-child").unwrap();
    cx.simulate_click(load.center(), Default::default());
    cx.run_until_parked();
    let connection::Command::Call {
        tag,
        payload,
        reply,
    } = requests.try_recv().unwrap()
    else {
        panic!("script RPC")
    };
    assert_eq!(tag, "orchestration.getWorkflowScript");
    assert_eq!(
        payload,
        json!({"threadId":"t","scriptPath":"/tmp/workflow.js"})
    );
    let close = cx.debug_bounds("close-script-child").unwrap();
    cx.simulate_click(close.center(), Default::default());
    cx.run_until_parked();
    let load = cx.debug_bounds("workflow-script-child").unwrap();
    cx.simulate_click(load.center(), Default::default());
    cx.run_until_parked();
    let connection::Command::Call { reply: second, .. } = requests.try_recv().unwrap() else {
        panic!("second script RPC")
    };
    reply
        .send_blocking(Ok(json!({"contents":"OLD","truncated":false})))
        .unwrap();
    cx.run_until_parked();
    app.read_with(cx, |app, _| {
        assert!(
            app.agents_surface
                .scripts
                .values()
                .all(|(_, r)| matches!(r, Ok(None)))
        )
    });
    second
        .send_blocking(Ok(json!({"contents":"CURRENT","truncated":true})))
        .unwrap();
    cx.run_until_parked();
    app.read_with(cx, |app, _| {
        assert!(
            app.agents_surface
                .scripts
                .values()
                .any(|(_, r)| matches!(r,Ok(Some(v)) if v["contents"] == "CURRENT"))
        )
    });
    // Closing does not collapse the workflow or discard its observed lifecycle.
    let close = cx.debug_bounds("close-script-child").unwrap();
    cx.simulate_click(close.center(), Default::default());
    cx.run_until_parked();
    assert!(cx.debug_bounds("agent-timeline-child").is_some());
}
