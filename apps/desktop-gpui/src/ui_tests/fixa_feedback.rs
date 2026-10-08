use super::*;

#[gpui_kit::test]
fn fixa_error_overflow_stays_visible_and_dismisses_explicitly(cx: &mut TestAppContext) {
    let (app, cx, _) = harness(cx);
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            for i in 0..100 {
                app.report_error(format!("Denied request {i}"), cx);
            }
        })
    });
    cx.run_until_parked();
    assert!(cx.debug_bounds("toast-close-1").is_some());
    assert!(
        cx.debug_bounds("toast-close-5").is_none(),
        "only a bounded subset is materialized"
    );
    cx.background_executor
        .advance_clock(notifications::TOAST_LIFETIME * 2);
    cx.run_until_parked();
    assert_eq!(
        app.read_with(cx, |app, _| app.toasts.iter().map(|t| t.count).sum::<u64>()),
        100
    );
    let more = cx
        .debug_bounds("toast-more-errors")
        .expect("durable overflow entry");
    cx.simulate_click(more.center(), Default::default());
    cx.run_until_parked();
    assert_eq!(
        app.read_with(cx, |app, _| app.toasts.len()),
        notifications::VISIBLE_TOASTS
    );
    // Explicitly repaint after removing a whole stack row before measuring its new position.
    cx.update(|window, cx| {
        let _ = window.draw(cx);
    });
    cx.run_until_parked();
    let close = cx.debug_bounds("toast-close-1").unwrap();
    cx.simulate_mouse_move(close.center(), None, Default::default());
    cx.simulate_click(close.center(), Default::default());
    cx.run_until_parked();
    assert_eq!(
        app.read_with(cx, |app, _| app.toasts.len()),
        notifications::VISIBLE_TOASTS - 1
    );
}

#[gpui_kit::test]
fn fixa_workflow_receipt_settles_captured_cache_after_navigation(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    cx.update(|_, cx| app.update(cx, |app, cx| {
        app.detail.thread = Some(json!({"id":"t","session":null,"activities":[{"id":"spawn","kind":"task.completed","createdAt":"2026-01-01T00:00:01Z","payload":{"taskId":"child","agentKind":"agent","taskType":"local_workflow","title":"Workflow","runHandles":{"scriptPath":"/tmp/workflow.js"}}}]}).into());
        cx.notify();
    }));
    cx.run_until_parked();
    let tab = cx.debug_bounds("right-tab-agents").unwrap();
    cx.simulate_click(tab.center(), Default::default());
    cx.run_until_parked();
    let toggle = cx.debug_bounds("workflow-child").unwrap();
    cx.simulate_click(toggle.center(), Default::default());
    cx.run_until_parked();
    let load = cx.debug_bounds("workflow-script-child").unwrap();
    cx.simulate_click(load.center(), Default::default());
    cx.run_until_parked();
    let connection::Command::Call { tag, reply, .. } = requests.try_recv().unwrap() else {
        panic!("script RPC")
    };
    assert_eq!(tag, "orchestration.getWorkflowScript");
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.selected_thread = Some("other".into());
            cx.notify();
        })
    });
    cx.run_until_parked();
    reply
        .send_blocking(Ok(json!({"contents":"captured result","truncated":false})))
        .unwrap();
    cx.run_until_parked();
    app.read_with(cx, |app, _| {
        assert!(
            app.agents_surface.scripts.values().any(
                |(_, result)| matches!(result, Ok(Some(v)) if v["contents"] == "captured result")
            )
        )
    });
    assert!(cx.debug_bounds("close-script-child").is_none());
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.selected_thread = Some("t".into());
            cx.notify();
        })
    });
    cx.run_until_parked();
    let close = cx
        .debug_bounds("close-script-child")
        .expect("return restores settled script, never orphan Loading");
    cx.simulate_click(close.center(), Default::default());
    cx.run_until_parked();
    assert!(app.read_with(cx, |app, _| app.agents_surface.scripts.is_empty()));
}

#[gpui_kit::test]
fn fixa_config_stream_drives_provider_availability_and_external_usage(cx: &mut TestAppContext) {
    let (app, cx, _) = harness(cx);
    cx.update(|_, cx| app.update(cx, |app, cx| {
        app.accept_config_snapshot(json!({"providers":[],"environment":{"capabilities":{"usageLimitSources":true}},"settings":{}}), cx);
        app.apply_server_config_event(json!({"type":"usageLimitSourcesUpdated","payload":{"sources":[{"label":"External account","error":"probe failed"}]}}), cx);
        app.apply_server_config_event(json!({"type":"providerStatuses","payload":{"providers":[{"instanceId":"codex","driver":"codex","enabled":true,"installed":true,"status":"ready","models":[]}]}}), cx);
        assert!(app.provider_catalog[0].selectable());
        assert_eq!(t3code_desktop_gpui::surfaces::usage::accounts(app.server_config.config(app.generation).unwrap())[0].notice.as_deref(), Some("probe failed"));
        app.apply_server_config_event(json!({"type":"providerStatuses","payload":{"providers":[{"instanceId":"codex","driver":"codex","enabled":true,"installed":true,"status":"error","models":[]}]}}), cx);
        assert!(!app.provider_catalog[0].selectable());
        app.generation += 1;
        assert!(app.server_config.config(app.generation).is_none());
    }));
}
