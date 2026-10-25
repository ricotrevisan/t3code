use super::*;

const OFFICE: &str = "https://office.example";
const LAB: &str = "https://lab.example";

fn config(id: &str) -> Value {
    json!({"environment":{"environmentId":id,"label":id},"providers":[{"instanceId":"codex","driver":"codex","displayName":format!("{id}@example.com"),"enabled":true,"installed":true,"status":"ready","models":[],"usageLimits":{"checkedAt":"2026-10-10T12:00:00Z","windows":[{"label":"Weekly","usedPercent":99}],"resetCredits":{"availableCount":2}}}],"settings":{}})
}
fn call(
    requests: &Requests,
    expected: &str,
) -> (Value, async_channel::Sender<Result<Value, String>>) {
    let connection::Command::Call {
        tag,
        payload,
        reply,
    } = requests.try_recv().unwrap()
    else {
        panic!("expected {expected}");
    };
    assert_eq!(tag, expected);
    (payload, reply)
}
struct Live {
    app: Entity<NativeApp>,
    requests: Requests,
    updates: async_channel::Sender<connection::Update>,
}
fn connect(app: Entity<NativeApp>, cx: &mut VisualTestContext, id: &str) -> Live {
    let (commands, requests) = async_channel::bounded(64);
    let (updates, receiver) = async_channel::bounded(64);
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.disconnect(cx);
            app.start_owned_session(
                connection::Session {
                    commands,
                    updates: receiver,
                },
                cx,
            );
        })
    });
    cx.run_until_parked();
    let requests = Requests::from(requests);
    let (payload, reply) = call(&requests, "server.getConfig");
    assert_eq!(payload, json!({}));
    reply.try_send(Ok(config(id))).unwrap();
    cx.run_until_parked();
    updates.try_send(connection::Update::Shell(Box::new(serde_json::from_value(json!({"kind":"snapshot","snapshot":{"snapshotSequence":0,"projects":[{"id":"p","title":"Repository","workspaceRoot":"/tmp/fixture","repositoryIdentity":{"canonicalKey":"github:acme/repo"}}],"threads":[{"id":"t","projectId":"p","title":format!("{id} thread"),"modelSelection":{"instanceId":"codex","model":"gpt-5.4"},"runtimeMode":"full-access","interactionMode":"default","archivedAt":null}]}})).unwrap()))).unwrap();
    updates
        .try_send(connection::Update::Shell(Box::new(
            shell::ShellItem::Synchronized,
        )))
        .unwrap();
    cx.run_until_parked();
    let connection::Command::Subscribe { tag, payload, .. } = requests.try_recv().unwrap() else {
        panic!("server config subscription");
    };
    assert_eq!(tag, "subscribeServerConfig");
    assert_eq!(payload, json!({"usageLimitSources":true}));
    let (_, reply) = call(&requests, "server.getConfig");
    reply.try_send(Ok(config(id))).unwrap();
    cx.run_until_parked();
    cx.update(|window, cx| app.update(cx, |app, cx| app.select_thread("t", window, cx)));
    cx.run_until_parked();
    assert!(
        matches!(requests.try_recv().unwrap(), connection::Command::Subscribe { tag, .. } if tag == "subscribeWorktreeSetup")
    );
    assert!(
        matches!(requests.try_recv().unwrap(), connection::Command::SelectThread(Some(id)) if id == "t")
    );
    assert!(requests.is_empty());
    Live {
        app,
        requests,
        updates,
    }
}
fn pair(cx: &mut TestAppContext) -> (Live, Live, &mut VisualTestContext) {
    let (app, cx, _) = harness(cx);
    let lab = cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.environment_scope = OFFICE.into();
            app.server
                .update(cx, |input, cx| input.set_value(OFFICE, window, cx));
            app.save_environment(cx);
            app.server
                .update(cx, |input, cx| input.set_value(LAB, window, cx));
            app.save_environment(cx);
            app.server
                .update(cx, |input, cx| input.set_value(OFFICE, window, cx));
            app.create_environment_page(LAB, window, cx)
        })
    });
    let office = connect(app, cx, "office-id");
    let lab = connect(lab, cx, "lab-id");
    (office, lab, cx)
}
fn switch(from: &Live, origin: &str, cx: &mut VisualTestContext) {
    cx.update(|window, cx| {
        from.app
            .update(cx, |app, cx| app.switch_environment(origin, window, cx))
    });
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
fn r18_w_usage_background_credit_receipt_stays_on_captured_page(cx: &mut TestAppContext) {
    let (office, lab, cx) = pair(cx);
    click(cx, "usage-limits-entry");
    click(cx, "usage-reveal-0");
    click(cx, "usage-credit-0");
    click(cx, "usage-confirm-0");
    let (payload, reply) = call(&office.requests, "provider.consumeResetCredit");
    assert_eq!(payload, json!({"instanceId":"codex"}));
    switch(&office, LAB, cx);
    click(cx, "usage-limits-entry");
    click(cx, "usage-credit-0");
    assert!(cx.debug_bounds("usage-confirm-0").is_some());
    assert!(
        lab.requests.is_empty(),
        "confirmation never spends by itself"
    );
    reply.try_send(Err("Office credit denied".into())).unwrap();
    cx.run_until_parked();
    assert_eq!(
        office
            .app
            .read_with(cx, |app, cx| app.usage_feedback("provider:codex", cx).0),
        Some("Could not use the reset credit: Office credit denied".into())
    );
    assert_eq!(
        lab.app
            .read_with(cx, |app, cx| app.usage_feedback("provider:codex", cx)),
        (None, false)
    );
    assert!(
        cx.debug_bounds("usage-confirm-0").is_some(),
        "other page confirmation survives late receipt"
    );
    click(cx, "usage-confirm-0");
    let (payload, reply) = call(&lab.requests, "provider.consumeResetCredit");
    assert_eq!(payload, json!({"instanceId":"codex"}));
    reply
        .try_send(Ok(json!({"outcome":"nothingToReset"})))
        .unwrap();
    cx.run_until_parked();
    switch(&lab, OFFICE, cx);
    // Switching may dismiss native popovers, but must not erase owned feedback.
    assert_eq!(
        office
            .app
            .read_with(cx, |app, cx| app.usage_feedback("provider:codex", cx).0),
        Some("Could not use the reset credit: Office credit denied".into())
    );
    assert_eq!(
        lab.app
            .read_with(cx, |app, cx| app.usage_feedback("provider:codex", cx).0),
        Some("Nothing to reset right now.".into())
    );
    assert!(
        office.requests.is_empty(),
        "no redemption replay on selection"
    );
}

#[gpui_kit::test]
fn r18_w_usage_confirmation_and_cancel_are_independent_across_pages(cx: &mut TestAppContext) {
    let (office, lab, cx) = pair(cx);
    click(cx, "usage-limits-entry");
    click(cx, "usage-credit-0");
    switch(&office, LAB, cx);
    click(cx, "usage-limits-entry");
    click(cx, "usage-credit-0");
    click(cx, "usage-cancel-0");
    assert!(cx.debug_bounds("usage-confirm-0").is_none());
    switch(&lab, OFFICE, cx);
    assert!(
        cx.debug_bounds("usage-confirm-0").is_some(),
        "Office confirmation retained on its page"
    );
    click(cx, "usage-cancel-0");
    assert!(office.requests.is_empty());
    assert!(lab.requests.is_empty());
}

#[gpui_kit::test]
fn r18_w_usage_old_generation_receipt_cannot_replace_other_page_feedback(cx: &mut TestAppContext) {
    let (office, lab, cx) = pair(cx);
    click(cx, "usage-limits-entry");
    click(cx, "usage-credit-0");
    click(cx, "usage-confirm-0");
    let (_, office_reply) = call(&office.requests, "provider.consumeResetCredit");
    switch(&office, LAB, cx);
    click(cx, "usage-limits-entry");
    click(cx, "usage-credit-0");
    click(cx, "usage-confirm-0");
    let (_, lab_reply) = call(&lab.requests, "provider.consumeResetCredit");
    lab_reply
        .try_send(Ok(json!({"outcome":"noCredit"})))
        .unwrap();
    cx.run_until_parked();
    cx.update(|_, cx| {
        office.app.update(cx, |app, cx| {
            app.generation += 1;
            cx.notify();
        })
    });
    office_reply
        .try_send(Ok(json!({"outcome":"reset"})))
        .unwrap();
    cx.run_until_parked();
    assert_eq!(
        office
            .app
            .read_with(cx, |app, cx| app.usage_feedback("provider:codex", cx).0),
        None
    );
    assert_eq!(
        lab.app
            .read_with(cx, |app, cx| app.usage_feedback("provider:codex", cx).0),
        Some("No reset credit left.".into())
    );
    assert!(cx.debug_bounds("usage-result-0").is_some());
    assert!(lab.requests.is_empty());
}
