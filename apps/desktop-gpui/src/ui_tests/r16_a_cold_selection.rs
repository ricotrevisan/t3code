use super::*;

#[gpui_kit::test]
fn r16_a_cold_synchronized_first_mouse_selects_and_paints_owned_session_detail(
    cx: &mut TestAppContext,
) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        theme::init(cx);
        bind_native_keys(cx);
    });
    let mut app = None;
    let (_, cx) = cx.add_window_view(|window, cx| {
        let entity = cx.new(|cx| NativeApp::new(window, cx, false));
        app = Some(entity.clone());
        Root::new(entity, window, cx)
    });
    cx.simulate_resize(size(px(1280.), px(983.)));
    let app = app.unwrap();
    let (commands, receiver) = async_channel::bounded(64);
    let (updates, update_receiver) = async_channel::bounded(64);
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.start_owned_session(
                connection::Session {
                    commands,
                    updates: update_receiver,
                },
                cx,
            );
        })
    });
    cx.run_until_parked();
    let requests = Requests::from(receiver);
    let (payload, reply) = surfaces_terminal::call(&requests, "server.getConfig");
    assert_eq!(payload, json!({}));
    let config = json!({"environment":{"environmentId":"cold-office","label":"Office"},"providers":[],"settings":{}});
    reply.try_send(Ok(config.clone())).unwrap();
    cx.run_until_parked();
    updates.try_send(connection::Update::Shell(Box::new(serde_json::from_value(json!({"kind":"snapshot","snapshot":{"snapshotSequence":0,"projects":[{"id":"p","title":"Fixture","workspaceRoot":"/tmp/fixture"}],"threads":[{"id":"fresh","projectId":"p","title":"Fresh","archivedAt":null}]}})).unwrap()))).unwrap();
    updates
        .try_send(connection::Update::Shell(Box::new(
            shell::ShellItem::Synchronized,
        )))
        .unwrap();
    cx.run_until_parked();
    assert!(
        matches!(requests.try_recv().unwrap(), connection::Command::Subscribe { tag, .. } if tag == "subscribeServerConfig")
    );
    let (_, reply) = surfaces_terminal::call(&requests, "server.getConfig");
    reply.try_send(Ok(config)).unwrap();
    cx.run_until_parked();
    assert!(app.read_with(cx, |app, _| app.connection_ready()
        && app.selected_thread.is_none()));
    let row = cx.debug_bounds("thread-fresh").unwrap();
    cx.simulate_mouse_move(row.center(), None, Default::default());
    cx.simulate_event(gpui_kit::MouseDownEvent {
        button: MouseButton::Left,
        position: row.center(),
        modifiers: Default::default(),
        click_count: 1,
        first_mouse: true,
    });
    cx.simulate_mouse_up(row.center(), MouseButton::Left, Default::default());
    cx.run_until_parked();
    assert_eq!(
        app.read_with(cx, |app, _| app
            .selected_thread
            .as_deref()
            .map(str::to_owned)),
        Some("fresh".into())
    );
    assert!(
        matches!(requests.try_recv().unwrap(), connection::Command::Subscribe { tag, payload, .. } if tag == "subscribeWorktreeSetup" && payload["threadId"] == "fresh")
    );
    assert!(
        matches!(requests.try_recv().unwrap(), connection::Command::SelectThread(Some(id)) if id == "fresh")
    );
    updates.try_send(connection::Update::Thread { thread_id: "fresh".into(), item: json!({"kind":"snapshot","snapshot":{"snapshotSequence":1,"thread":{"id":"fresh","title":"Fresh","messages":[{"id":"first-answer","role":"assistant","text":"Cold first click selected this conversation"}],"session":{"status":"idle"}}}}) }).unwrap();
    cx.run_until_parked();
    assert!(cx.debug_bounds("message-first-answer").is_some());
    assert!(app.read_with(cx, |app, _| app.selected_thread.as_deref() == Some("fresh")));
}
