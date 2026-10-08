//! Start receipt → authoritative interaction → real Settings controls → scoped respond receipt.
use super::*;
fn rpc(
    requests: &Requests,
    expected: &str,
) -> (Value, async_channel::Sender<Result<Value, String>>) {
    let connection::Command::Call {
        tag,
        payload,
        reply,
    } = requests.try_recv().unwrap()
    else {
        panic!("expected {expected}")
    };
    assert_eq!(tag, expected);
    (payload, reply)
}
fn click(cx: &mut VisualTestContext, id: &'static str) {
    let target = cx.debug_bounds(id).unwrap().center();
    cx.simulate_click(target, Default::default());
    cx.run_until_parked();
}
fn start(
    app: &Entity<NativeApp>,
    cx: &mut VisualTestContext,
    requests: &Requests,
    interaction: Value,
) -> Value {
    cx.simulate_resize(size(px(1600.), px(1600.)));
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.settings_open = true;
            app.settings_query
                .update(cx, |input, cx| input.set_value("providers", window, cx));
            app.start_provider_auth("login", cx);
        })
    });
    cx.run_until_parked();
    let (payload, reply) = rpc(requests, "provider.auth.start");
    assert_eq!(payload["instanceId"], "login");
    let state =
        json!({"instanceId":"login","flowId":"flow","phase":"waiting","interaction":interaction});
    reply.send_blocking(Ok(state.clone())).unwrap();
    cx.run_until_parked();
    assert!(
        matches!(requests.try_recv().unwrap(), connection::Command::Subscribe { tag, .. } if tag == "provider.auth.subscribe")
    );
    state
}
#[gpui_kit::test]
fn r13_f_provider_consent_receipt_and_failure_retry(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    let mut state = start(
        &app,
        cx,
        &requests,
        json!({"type":"browser","id":"consent","url":"https://example.test/authorize","requiresConsent":true}),
    );
    assert!(
        cx.opened_url().is_none(),
        "consent must precede browser launch"
    );
    click(cx, "provider-consent-accept");
    let (payload, reply) = rpc(&requests, "provider.auth.respond");
    assert_eq!(
        payload,
        json!({"instanceId":"login","flowId":"flow","interactionId":"consent","response":{"type":"browser","action":"accept"}})
    );
    reply.send_blocking(Err("failed".into())).unwrap();
    cx.run_until_parked();
    assert_eq!(
        app.read_with(cx, |app, _| app.auth_state.as_ref().unwrap()["phase"]
            .clone()),
        "waiting"
    );
    click(cx, "provider-consent-decline");
    let (payload, reply) = rpc(&requests, "provider.auth.respond");
    assert_eq!(payload["response"]["action"], "decline");
    state["phase"] = json!("cancelled");
    state["interaction"] = Value::Null;
    reply.send_blocking(Ok(state)).unwrap();
    cx.run_until_parked();
    assert_eq!(
        app.read_with(cx, |app, _| app.auth_state.as_ref().unwrap()["phase"]
            .clone()),
        "cancelled"
    );
}
#[gpui_kit::test]
fn r13_f_provider_credentials_and_terminal_capture_real_wire(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    let mut state = start(
        &app,
        cx,
        &requests,
        json!({"type":"credentials","id":"credentials","fields":[{"name":"token","label":"API token","secret":true}]}),
    );
    click(cx, "auth-prepare-inputs");
    cx.simulate_input("credential-fixture");
    click(cx, "auth-submit-inputs");
    let (payload, reply) = rpc(&requests, "provider.auth.respond");
    assert_eq!(
        payload["response"],
        json!({"type":"credentials","values":{"token":"credential-fixture"}})
    );
    state["interaction"] = json!({"type":"terminal","id":"terminal","output":"Enter account:"});
    reply.send_blocking(Ok(state.clone())).unwrap();
    cx.run_until_parked();
    click(cx, "auth-prepare-inputs");
    cx.simulate_input("account-fixture");
    click(cx, "auth-submit-inputs");
    let (payload, reply) = rpc(&requests, "provider.auth.respond");
    assert_eq!(payload["interactionId"], "terminal");
    assert_eq!(
        payload["response"],
        json!({"type":"terminal","data":"account-fixture\n"})
    );
    // A late old-environment receipt must not install a successful sign-in.
    cx.update(|_, cx| app.update(cx, |app, _| app.generation += 1));
    state["phase"] = json!("succeeded");
    reply.send_blocking(Ok(state)).unwrap();
    cx.run_until_parked();
    assert_ne!(
        app.read_with(cx, |app, _| app.auth_state.as_ref().unwrap()["phase"]
            .clone()),
        "succeeded"
    );
}
