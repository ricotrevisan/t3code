use super::*;
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
        panic!("expected {expected}")
    };
    assert_eq!(tag, expected);
    (payload, reply)
}
struct Page {
    app: Entity<NativeApp>,
    requests: Requests,
    updates: async_channel::Sender<connection::Update>,
    key: String,
    settings: Value,
    config: Value,
}
fn connect(app: Entity<NativeApp>, cx: &mut VisualTestContext, id: &str) -> Page {
    let settings = json!({"providerInstances":{"same":{"driver":"cursor","enabled":true,"displayName":id,"config":{"apiEndpoint":format!("https://{id}.example"),"binaryPath":"cursor-agent"}}}});
    let config = json!({"environment":{"environmentId":id,"label":id},"settings":settings,"providers":[{"instanceId":"same","driver":"cursor","displayName":id,"enabled":true,"installed":true,"status":"ready","models":[]}]});
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
    let (_, reply) = call(&requests, "server.getConfig");
    reply.try_send(Ok(config.clone())).unwrap();
    cx.run_until_parked();
    updates.try_send(connection::Update::Shell(Box::new(serde_json::from_value(json!({"kind":"snapshot","snapshot":{"snapshotSequence":0,"projects":[],"threads":[]}})).unwrap()))).unwrap();
    updates
        .try_send(connection::Update::Shell(Box::new(
            shell::ShellItem::Synchronized,
        )))
        .unwrap();
    cx.run_until_parked();
    let connection::Command::Subscribe { key, tag, .. } = requests.try_recv().unwrap() else {
        panic!("config subscription")
    };
    assert_eq!(tag, "subscribeServerConfig");
    let (_, reply) = call(&requests, "server.getConfig");
    reply.try_send(Ok(config.clone())).unwrap();
    cx.run_until_parked();
    Page {
        app,
        requests,
        updates,
        key,
        settings,
        config,
    }
}
fn click(cx: &mut VisualTestContext, id: &'static str) {
    let point = cx.debug_bounds(id).unwrap().center();
    cx.simulate_click(point, Default::default());
    cx.run_until_parked();
}
#[gpui_kit::test]
fn r18_f_background_instance_api_receipt_and_stream_stay_on_owner_page(cx: &mut TestAppContext) {
    const OFFICE: &str = "https://office.example";
    const LAB: &str = "https://lab.example";
    let (app, cx, _) = harness(cx);
    let other = cx.update(|window, cx| {
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
    let mut office = connect(app, cx, "office-id");
    let lab = connect(other, cx, "lab-id");
    cx.simulate_resize(size(px(1600.), px(1600.)));
    cx.update(|window, cx| {
        office.app.update(cx, |app, cx| {
            app.settings_open = true;
            app.settings_query.update(cx, |input, cx| {
                input.set_value("authentication", window, cx)
            });
            cx.notify();
        })
    });
    cx.run_until_parked();
    click(cx, "provider-edit-same");
    let (_, reply) = call(&office.requests, "server.getSettings");
    reply.try_send(Ok(office.settings.clone())).unwrap();
    cx.run_until_parked();
    click(cx, "provider-instance-field-apiEndpoint");
    cx.simulate_keystrokes("cmd-a");
    cx.simulate_input("https://new-office.example");
    click(cx, "provider-instance-save");
    let (_, reply) = call(&office.requests, "server.getSettings");
    cx.update(|window, cx| {
        office
            .app
            .update(cx, |app, cx| app.switch_environment(LAB, window, cx))
    });
    cx.run_until_parked();
    reply.try_send(Ok(office.settings.clone())).unwrap();
    cx.run_until_parked();
    let (payload, reply) = call(&office.requests, "server.updateSettings");
    assert_eq!(
        payload["patch"]["providerInstances"]["same"]["config"]["apiEndpoint"],
        "https://new-office.example"
    );
    assert!(lab.requests.is_empty());
    office.settings["providerInstances"] = payload["patch"]["providerInstances"].clone();
    reply.try_send(Ok(office.settings.clone())).unwrap();
    cx.run_until_parked();
    let (_, reply) = call(&office.requests, "server.refreshProviders");
    reply.try_send(Ok(json!({}))).unwrap();
    cx.run_until_parked();
    office.config["settings"] = office.settings.clone();
    office.config["providers"][0]["displayName"] = json!("Refreshed office");
    office.updates.try_send(connection::Update::Subscription {key:office.key.clone(), item:Ok(json!({"type":"providerStatuses","payload":{"providers":office.config["providers"]}}))}).unwrap();
    cx.run_until_parked();
    let (_, reply) = call(&office.requests, "server.getConfig");
    reply.try_send(Ok(office.config.clone())).unwrap();
    cx.run_until_parked();
    let (_, reply) = call(&office.requests, "server.getSettings");
    reply.try_send(Ok(office.settings.clone())).unwrap();
    cx.run_until_parked();
    assert_eq!(lab.app.read_with(cx, |app, _| app.project_settings.as_ref().unwrap()["providerInstances"]["same"]["config"]["apiEndpoint"].clone()), json!("https://lab-id.example"));
    assert_eq!(
        office
            .app
            .read_with(cx, |app, _| app.provider_catalog[0].label().to_owned()),
        "Refreshed office"
    );
    cx.update(|window, cx| {
        lab.app
            .update(cx, |app, cx| app.switch_environment(OFFICE, window, cx))
    });
    cx.run_until_parked();
    click(cx, "provider-edit-same");
    let (_, reply) = call(&office.requests, "server.getSettings");
    reply.try_send(Ok(office.settings.clone())).unwrap();
    cx.run_until_parked();
    click(cx, "provider-instance-field-apiEndpoint");
    cx.simulate_keystrokes("cmd-a");
    cx.simulate_input("");
    click(cx, "provider-instance-save");
    let (_, reply) = call(&office.requests, "server.getSettings");
    reply.try_send(Ok(office.settings.clone())).unwrap();
    cx.run_until_parked();
    let (payload, reply) = call(&office.requests, "server.updateSettings");
    assert!(
        payload["patch"]["providerInstances"]["same"]["config"]
            .get("apiEndpoint")
            .is_none()
    );
    reply.try_send(Err("reverse denied".into())).unwrap();
    cx.run_until_parked();
    assert_eq!(
        office
            .app
            .read_with(cx, |app, _| app.provider_instance_editor.error.clone())
            .as_deref(),
        Some("reverse denied")
    );
    assert!(lab.requests.is_empty());
}
