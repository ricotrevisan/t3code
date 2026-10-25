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
    let point = cx.debug_bounds(id).unwrap().center();
    cx.simulate_click(point, Default::default());
    cx.run_until_parked();
}
fn config(settings: &Value) -> Value {
    json!({"providers":settings["providerInstances"].as_object().unwrap().iter().map(|(id, value)| json!({"instanceId":id,"driver":value["driver"],"displayName":value["displayName"],"enabled":true,"installed":true,"status":"ready","models":[]})).collect::<Vec<_>>()})
}
fn setup(app: &Entity<NativeApp>, cx: &mut VisualTestContext, requests: &Requests) -> Value {
    let settings = json!({"providerInstances":{"one":{"driver":"codex","displayName":"Original","enabled":true,"config":{"binaryPath":"codex","homePath":"/keep","launchArgs":"--keep"}},"other":{"driver":"custom","config":{"opaque":7}}}});
    cx.simulate_resize(size(px(1600.), px(1800.)));
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.settings_open = true;
            app.settings_query.update(cx, |input, cx| {
                input.set_value("authentication", window, cx)
            });
            app.refresh_settings_providers(false, cx);
        })
    });
    cx.run_until_parked();
    let (_, reply) = rpc(requests, "server.getConfig");
    reply.send_blocking(Ok(config(&settings))).unwrap();
    cx.run_until_parked();
    let (_, reply) = rpc(requests, "server.getSettings");
    reply.send_blocking(Ok(settings.clone())).unwrap();
    cx.run_until_parked();
    settings
}
fn opened(cx: &mut VisualTestContext, requests: &Requests, settings: &Value, copy: bool) {
    click(
        cx,
        if copy {
            "provider-copy-one"
        } else {
            "provider-edit-one"
        },
    );
    let (payload, reply) = rpc(requests, "server.getSettings");
    assert_eq!(payload, json!({}));
    reply.send_blocking(Ok(settings.clone())).unwrap();
    cx.run_until_parked();
}
fn refresh(cx: &mut VisualTestContext, requests: &Requests, settings: &Value) {
    let (payload, reply) = rpc(requests, "server.refreshProviders");
    assert_eq!(payload, json!({"refreshModels":true}));
    reply.send_blocking(Ok(json!({}))).unwrap();
    cx.run_until_parked();
    let (_, reply) = rpc(requests, "server.getConfig");
    reply.send_blocking(Ok(config(settings))).unwrap();
    cx.run_until_parked();
    let (_, reply) = rpc(requests, "server.getSettings");
    reply.send_blocking(Ok(settings.clone())).unwrap();
    cx.run_until_parked();
}
#[gpui_kit::test]
fn r18_f_edit_instance_receipt_denial_retry_and_canonical_projection(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    let mut settings = setup(&app, cx, &requests);
    opened(cx, &requests, &settings, false);
    click(cx, "provider-instance-name");
    cx.simulate_keystrokes("cmd-a");
    cx.simulate_input("Renamed");
    click(cx, "provider-instance-binary");
    cx.simulate_keystrokes("cmd-a");
    cx.simulate_input("/tools/codex");
    click(cx, "provider-instance-save");
    let (_, reply) = rpc(&requests, "server.getSettings");
    reply.send_blocking(Ok(settings.clone())).unwrap();
    cx.run_until_parked();
    let (payload, reply) = rpc(&requests, "server.updateSettings");
    assert_eq!(
        payload["patch"]["providerInstances"]["one"]["displayName"],
        "Renamed"
    );
    assert_eq!(
        payload["patch"]["providerInstances"]["one"]["config"],
        json!({"binaryPath":"/tools/codex","homePath":"/keep","launchArgs":"--keep"})
    );
    assert_eq!(
        payload["patch"]["providerInstances"]["other"],
        settings["providerInstances"]["other"]
    );
    reply.send_blocking(Err("write denied".into())).unwrap();
    cx.run_until_parked();
    assert_eq!(
        app.read_with(cx, |app, _| app.provider_instance_editor.error.clone())
            .as_deref(),
        Some("write denied")
    );
    assert_eq!(
        app.read_with(cx, |app, cx| app
            .provider_instance_editor
            .name
            .read(cx)
            .value()
            .to_string()),
        "Renamed"
    );
    click(cx, "provider-instance-save");
    let (_, reply) = rpc(&requests, "server.getSettings");
    reply.send_blocking(Ok(settings.clone())).unwrap();
    cx.run_until_parked();
    let (payload, reply) = rpc(&requests, "server.updateSettings");
    settings["providerInstances"] = payload["patch"]["providerInstances"].clone();
    reply.send_blocking(Ok(settings.clone())).unwrap();
    cx.run_until_parked();
    assert_eq!(
        app.read_with(cx, |app, _| {
            app.project_settings.as_ref().unwrap()["providerInstances"]["one"]["displayName"]
                .clone()
        }),
        json!("Renamed")
    );
    refresh(cx, &requests, &settings);
    opened(cx, &requests, &settings, false);
    assert_eq!(
        app.read_with(cx, |app, cx| app
            .provider_instance_editor
            .binary
            .read(cx)
            .value()
            .to_string()),
        "/tools/codex"
    );
    click(cx, "provider-instance-cancel");
    assert!(requests.is_empty());
}
#[gpui_kit::test]
fn r18_f_add_instance_and_confirmed_remove_preserve_other_instances(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    let mut settings = setup(&app, cx, &requests);
    opened(cx, &requests, &settings, true);
    click(cx, "provider-instance-save");
    let (_, reply) = rpc(&requests, "server.getSettings");
    reply.send_blocking(Ok(settings.clone())).unwrap();
    cx.run_until_parked();
    let (payload, reply) = rpc(&requests, "server.updateSettings");
    assert_eq!(
        payload["patch"]["providerInstances"]["one_copy"],
        settings["providerInstances"]["one"]
    );
    settings["providerInstances"] = payload["patch"]["providerInstances"].clone();
    reply.send_blocking(Ok(settings.clone())).unwrap();
    cx.run_until_parked();
    refresh(cx, &requests, &settings);
    opened(cx, &requests, &settings, false);
    click(cx, "provider-instance-remove");
    assert!(requests.is_empty(), "removal requires confirmation");
    click(cx, "provider-instance-confirm-remove");
    let (_, reply) = rpc(&requests, "server.getSettings");
    reply.send_blocking(Ok(settings.clone())).unwrap();
    cx.run_until_parked();
    let (payload, reply) = rpc(&requests, "server.updateSettings");
    assert!(payload["patch"]["providerInstances"].get("one").is_none());
    assert_eq!(
        payload["patch"]["providerInstances"]["one_copy"],
        settings["providerInstances"]["one_copy"]
    );
    settings["providerInstances"] = payload["patch"]["providerInstances"].clone();
    reply.send_blocking(Ok(settings.clone())).unwrap();
    cx.run_until_parked();
    refresh(cx, &requests, &settings);
    assert!(app.read_with(cx, |app, _| {
        app.project_settings.as_ref().unwrap()["providerInstances"]
            .get("one")
            .is_none()
    }));
}
#[gpui_kit::test]
fn r18_f_instance_reconnect_during_preflight_emits_no_write(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    let settings = setup(&app, cx, &requests);
    opened(cx, &requests, &settings, false);
    click(cx, "provider-instance-save");
    let (_, reply) = rpc(&requests, "server.getSettings");
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.generation += 1;
            cx.notify();
        })
    });
    reply.send_blocking(Ok(settings)).unwrap();
    cx.run_until_parked();
    assert!(
        requests.is_empty(),
        "stale preflight must not dispatch a settings mutation"
    );
    click(cx, "provider-instance-cancel");
}
#[gpui_kit::test]
fn r18_f_instance_newer_settings_stream_wins_over_save_receipt(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    let settings = setup(&app, cx, &requests);
    opened(cx, &requests, &settings, false);
    click(cx, "provider-instance-save");
    let (_, reply) = rpc(&requests, "server.getSettings");
    reply.send_blocking(Ok(settings.clone())).unwrap();
    cx.run_until_parked();
    let (_, reply) = rpc(&requests, "server.updateSettings");
    let mut newer = settings.clone();
    newer["defaultRuntimeMode"] = "approval-required".into();
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.apply_server_config_event(
                json!({"type":"settingsUpdated","payload":{"settings":newer}}),
                cx,
            )
        })
    });
    reply.send_blocking(Ok(settings.clone())).unwrap();
    cx.run_until_parked();
    refresh(cx, &requests, &settings);
    assert_eq!(
        app.read_with(cx, |app, _| {
            app.project_settings.as_ref().unwrap()["defaultRuntimeMode"].clone()
        }),
        json!("approval-required")
    );
}
#[gpui_kit::test]
fn r18_f_instance_conflict_and_late_generation_never_mutate(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    let settings = setup(&app, cx, &requests);
    opened(cx, &requests, &settings, false);
    click(cx, "provider-instance-save");
    let (_, reply) = rpc(&requests, "server.getSettings");
    let mut changed = settings.clone();
    changed["providerInstances"]["one"]["enabled"] = false.into();
    reply.send_blocking(Ok(changed)).unwrap();
    cx.run_until_parked();
    assert!(requests.is_empty());
    assert!(app.read_with(cx, |app, _| {
        app.provider_instance_editor
            .error
            .as_ref()
            .unwrap()
            .contains("changed")
    }));
    click(cx, "provider-instance-cancel");
    click(cx, "provider-edit-one");
    let (_, reply) = rpc(&requests, "server.getSettings");
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.generation += 1;
            cx.notify();
        })
    });
    reply.send_blocking(Ok(settings)).unwrap();
    cx.run_until_parked();
    click(cx, "provider-instance-save");
    assert!(requests.is_empty());
    click(cx, "provider-instance-cancel");
    assert!(!app.read_with(cx, |app, _| app.provider_instance_editor.is_open()));
}
