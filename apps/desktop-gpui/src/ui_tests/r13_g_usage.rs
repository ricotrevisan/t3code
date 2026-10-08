use super::*;

fn config() -> serde_json::Value {
    json!({"environment":{"environmentId":"office","label":"Office"},"providers":[{"instanceId":"codex","driver":"codex","displayName":"rico@example.com","enabled":true,"installed":true,"status":"ready","models":[],"usageLimits":{"checkedAt":"2026-10-10T12:00:00Z","windows":[{"label":"Weekly","usedPercent":99}],"resetCredits":{"availableCount":2}}}],"usageLimitSources":[],"settings":{}})
}
fn receive_config(
    app: &Entity<NativeApp>,
    cx: &mut VisualTestContext,
    requests: &Requests,
    config: serde_json::Value,
) {
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.perform("server.getConfig", json!({}), None, cx)
        })
    });
    cx.run_until_parked();
    let connection::Command::Call {
        tag,
        payload,
        reply,
    } = requests.try_recv().unwrap()
    else {
        panic!("config request");
    };
    assert_eq!(tag, "server.getConfig");
    assert_eq!(payload, json!({}));
    reply.send_blocking(Ok(config)).unwrap();
    cx.run_until_parked();
}
fn click(cx: &mut VisualTestContext, id: &'static str) {
    let bounds = cx
        .debug_bounds(id)
        .unwrap_or_else(|| panic!("missing {id}"));
    cx.simulate_click(bounds.center(), Default::default());
    cx.run_until_parked();
}

#[gpui_kit::test]
fn r13_g_usage_confirm_cancel_dispatch_and_failed_receipt_retry(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    receive_config(&app, cx, &requests, config());
    click(cx, "usage-limits-entry");
    click(cx, "usage-reveal-0");
    assert!(app.read_with(cx, |app, cx| app.usage_feedback("provider:codex", cx).1));
    click(cx, "usage-reveal-0");
    assert!(!app.read_with(cx, |app, cx| app.usage_feedback("provider:codex", cx).1));
    click(cx, "usage-credit-0");
    assert!(
        requests.try_recv().is_err(),
        "capture alone must not redeem"
    );
    click(cx, "usage-cancel-0");
    assert!(requests.try_recv().is_err());
    click(cx, "usage-credit-0");
    click(cx, "usage-confirm-0");
    let connection::Command::Call {
        tag,
        payload,
        reply,
    } = requests.try_recv().unwrap()
    else {
        panic!("credit request");
    };
    assert_eq!(tag, "provider.consumeResetCredit");
    assert_eq!(payload, json!({"instanceId":"codex"}));
    click(cx, "usage-credit-0");
    assert!(
        requests.try_recv().is_err(),
        "busy click must not send a second request"
    );
    reply
        .send_blocking(Err("Credit probe denied".into()))
        .unwrap();
    cx.run_until_parked();
    assert!(cx.debug_bounds("usage-result-0").is_some());
    assert_eq!(
        app.read_with(cx, |app, cx| app.usage_feedback("provider:codex", cx).0),
        Some("Could not use the reset credit: Credit probe denied".into())
    );
    click(cx, "usage-credit-0");
    click(cx, "usage-confirm-0");
    let connection::Command::Call {
        tag,
        payload,
        reply,
    } = requests.try_recv().unwrap()
    else {
        panic!("retry request");
    };
    assert_eq!(tag, "provider.consumeResetCredit");
    assert_eq!(payload, json!({"instanceId":"codex"}));
    reply
        .send_blocking(Ok(json!({"outcome":"nothingToReset"})))
        .unwrap();
    cx.run_until_parked();
    assert!(cx.debug_bounds("usage-result-0").is_some());
    assert_eq!(
        app.read_with(cx, |app, cx| app.usage_feedback("provider:codex", cx).0),
        Some("Nothing to reset right now.".into())
    );
    click(cx, "usage-close");
    assert!(cx.debug_bounds("usage-limits-panel").is_none());
    click(cx, "usage-limits-entry");
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    assert!(
        cx.debug_bounds("usage-limits-panel").is_none(),
        "Kit popover dismisses on Escape"
    );
}

#[gpui_kit::test]
fn r13_g_usage_hub_credit_pins_displayed_id_and_revalidates_confirmation(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    let hub = |credit: &str| json!({"environment":{"environmentId":"office"},"providers":[],"usageLimitSources":[{"id":"hub","label":"Hub","accounts":[{"id":"account","email":"rico@example.com","driver":"codex","usageLimits":{"checkedAt":"now","windows":[],"resetCredits":{"availableCount":1,"nextCreditId":credit}}}]}],"settings":{}});
    receive_config(&app, cx, &requests, hub("credit-1"));
    cx.update(|_, cx| app.update(cx, |app, cx| app.apply_server_config_event(json!({"type":"usageLimitSourcesUpdated","payload":{"sources":hub("credit-1")["usageLimitSources"]}}), cx)));
    cx.run_until_parked();
    click(cx, "usage-limits-entry");
    click(cx, "usage-credit-0");
    cx.update(|_, cx| app.update(cx, |app, cx| app.apply_server_config_event(json!({"type":"usageLimitSourcesUpdated","payload":{"sources":hub("credit-2")["usageLimitSources"]}}), cx)));
    cx.run_until_parked();
    click(cx, "usage-confirm-0");
    assert!(
        requests.try_recv().is_err(),
        "stale confirmed credit cannot be redeemed"
    );
    click(cx, "usage-credit-0");
    click(cx, "usage-confirm-0");
    let connection::Command::Call {
        tag,
        payload,
        reply,
    } = requests.try_recv().unwrap()
    else {
        panic!("hub request");
    };
    assert_eq!(tag, "provider.consumeResetCredit");
    assert_eq!(
        payload,
        json!({"sourceId":"hub","accountId":"account","creditId":"credit-2"})
    );
    reply
        .send_blocking(Ok(
            json!({"outcome":"reset","warning":"Cooldown refresh failed"}),
        ))
        .unwrap();
    cx.run_until_parked();
    assert!(cx.debug_bounds("usage-result-0").is_some());
    assert_eq!(
        app.read_with(cx, |app, cx| app.usage_feedback("source:hub:account", cx).0),
        Some("Cooldown refresh failed".into())
    );
}

#[gpui_kit::test]
fn r13_g_usage_readiness_and_generation_guard_prevent_stale_projection(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    receive_config(&app, cx, &requests, config());
    click(cx, "usage-limits-entry");
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.apply_connection_phase(
                connection::Phase::Reconnecting {
                    attempt: 1,
                    retry_in: std::time::Duration::from_secs(1),
                    reason: "offline".into(),
                },
                cx,
            )
        })
    });
    cx.run_until_parked();
    click(cx, "usage-credit-0");
    assert!(cx.debug_bounds("usage-confirm-0").is_none());
    assert!(requests.try_recv().is_err());
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.apply_connection_phase(connection::Phase::Ready, cx)
        })
    });
    cx.run_until_parked();
    click(cx, "usage-credit-0");
    click(cx, "usage-confirm-0");
    let connection::Command::Call { reply, .. } = requests.try_recv().unwrap() else {
        panic!("credit request");
    };
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.generation += 1;
            cx.notify();
        })
    });
    reply.send_blocking(Ok(json!({"outcome":"reset"}))).unwrap();
    cx.run_until_parked();
    assert!(cx.debug_bounds("usage-limits-panel").is_none());
    receive_config(&app, cx, &requests, config());
    click(cx, "usage-limits-entry");
    assert!(cx.debug_bounds("usage-result-0").is_none());
    assert!(cx.debug_bounds("usage-reveal-0").is_some());
}
