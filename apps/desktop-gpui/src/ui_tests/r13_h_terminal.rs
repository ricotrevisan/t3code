//! Round-13 terminal entry points exercised through PTY requests and receipts.
use super::surfaces_terminal::{call, drain_sizes, open};
use super::*;

fn click(cx: &mut VisualTestContext, id: &'static str) {
    let bounds = cx
        .debug_bounds(id)
        .unwrap_or_else(|| panic!("missing {id}"));
    cx.simulate_click(bounds.center(), Default::default());
    cx.run_until_parked();
}

#[gpui_kit::test]
fn r13_h_terminal_restart_failure_retry_and_clear_preserve_receipt_ownership(
    cx: &mut TestAppContext,
) {
    let (app, cx, requests) = harness(cx);
    let (id, stream) = open(&app, cx, &requests);
    // Attach metadata can describe a changed display cwd, but cannot rewrite
    // the authoritative environment/worktree captured by this client's open.
    cx.update(|_, cx| app.update(cx, |app, cx| app.terminal_update(&stream, Ok(json!({"type":"snapshot","snapshot":{"threadId":"t","terminalId":id,"history":"prompt> ","status":"running","cwd":"/tmp/display-only","worktreePath":null,"sequence":1}})), cx)));
    click(cx, "terminal-restart");
    drain_sizes(cx, &requests);
    assert!(cx.debug_bounds("terminal-confirm-restart").is_some());
    assert!(requests.is_empty());
    click(cx, "terminal-confirm-restart");
    let (payload, reply) = call(&requests, "terminal.restart");
    assert_eq!(payload["threadId"], "t");
    assert_eq!(payload["terminalId"], id);
    assert_eq!(payload["cwd"], "/tmp/selected-checkout");
    assert_eq!(payload["worktreePath"], "/tmp/selected-checkout");
    assert_eq!(payload["env"]["T3CODE_PROJECT_ROOT"], "/tmp/fixture");
    assert_eq!(
        payload["env"]["T3CODE_WORKTREE_PATH"],
        "/tmp/selected-checkout"
    );
    assert!(payload["cols"].as_u64().unwrap() > 2);
    cx.simulate_input("must not send during restart");
    assert!(requests.is_empty());
    reply.send_blocking(Err("restart denied".into())).unwrap();
    cx.run_until_parked();
    app.read_with(cx, |app, _| {
        assert_eq!(
            app.terminal_drawer.tabs[0].error.as_deref(),
            Some("restart denied")
        );
        assert!(!app.terminal_drawer.tabs[0].busy);
        assert!(app.terminal_drawer.tabs[0].screen.selection().is_none());
    });
    click(cx, "terminal-restart");
    drain_sizes(cx, &requests);
    click(cx, "terminal-confirm-restart");
    let (_, reply) = call(&requests, "terminal.restart");
    reply.send_blocking(Ok(json!({"threadId":"t","terminalId":id,"cwd":"/tmp/selected-checkout","history":"fresh","status":"running","sequence":3}))).unwrap();
    cx.run_until_parked();
    app.read_with(cx, |app, _| {
        assert!(app.terminal_drawer.tabs[0].error.is_none())
    });
    drain_sizes(cx, &requests);
    click(cx, "terminal-clear");
    let (payload, reply) = call(&requests, "terminal.clear");
    assert_eq!(payload, json!({"threadId":"t","terminalId":id}));
    // A stream can clear and then receive output before the receipt. Do not erase it twice.
    cx.update(|_, cx| app.update(cx, |app, cx| {
        app.terminal_update(&stream, Ok(json!({"type":"cleared","threadId":"t","terminalId":id,"sequence":4})), cx);
        app.terminal_update(&stream, Ok(json!({"type":"output","threadId":"t","terminalId":id,"sequence":5,"data":"after"})), cx);
    }));
    reply.send_blocking(Ok(Value::Null)).unwrap();
    cx.run_until_parked();
    app.read_with(cx, |app, _| {
        let text = app.terminal_drawer.tabs[0]
            .screen
            .cells()
            .iter()
            .take(5)
            .map(|cell| cell.text.as_str())
            .collect::<String>();
        assert_eq!(text, "after");
    });
}

#[gpui_kit::test]
fn r13_h_terminal_discovery_attaches_without_open_or_restart_and_rejects_stale_metadata(
    cx: &mut TestAppContext,
) {
    let (app, cx, requests) = harness(cx);
    let (_, _) = open(&app, cx, &requests);
    click(cx, "terminal-discover");
    let connection::Command::Subscribe { key, tag, payload } = requests.try_recv().unwrap() else {
        panic!("metadata subscription")
    };
    assert_eq!(tag, "subscribeTerminalMetadata");
    assert_eq!(payload, json!({}));
    cx.update(|_, cx| app.update(cx, |app, cx| app.terminal_update(&key, Ok(json!({"type":"snapshot","terminals":[
        {"threadId":"t","terminalId":"term-2","cwd":"/tmp/restored","worktreePath":null,"status":"exited","label":"Build","hasRunningSubprocess":false},
        {"threadId":"other","terminalId":"term-3","cwd":"/tmp/other","status":"running"}
    ]})), cx)));
    cx.run_until_parked();
    assert!(
        matches!(
            requests.try_recv().unwrap(),
            connection::Command::Unsubscribe(_)
        ),
        "removed snapshot sessions must retire output"
    );
    let connection::Command::Subscribe {
        key: attach,
        tag,
        payload,
    } = requests.try_recv().unwrap()
    else {
        panic!("attach discovered PTY")
    };
    assert_eq!(tag, "terminal.attach");
    assert_eq!(
        payload,
        json!({"threadId":"t","terminalId":"term-2","restartIfNotRunning":false})
    );
    app.read_with(cx, |app, _| {
        assert_eq!(app.terminal_drawer.tabs.len(), 1);
        assert_eq!(app.terminal_drawer.tabs[0].title, "Build");
        assert!(!app.terminal_drawer.tabs[0].stream_ready);
    });
    cx.update(|_, cx| app.update(cx, |app, cx| {
        app.terminal_update(&attach, Ok(json!({"type":"snapshot","snapshot":{"threadId":"t","terminalId":"term-2","history":"saved","status":"exited","sequence":8}})), cx);
        app.generation += 1;
        app.terminal_update(&key, Ok(json!({"type":"upsert","terminal":{"threadId":"t","terminalId":"stale","cwd":"/tmp","status":"running"}})), cx);
        app.connection_phase = connection::Phase::Offline;
    }));
    app.read_with(cx, |app, _| assert_eq!(app.terminal_drawer.tabs.len(), 1));
}

#[gpui_kit::test]
fn r13_h_terminal_capture_uses_transport_output_and_mouse_selected_buffer_range(
    cx: &mut TestAppContext,
) {
    let (app, cx, requests) = harness(cx);
    let (id, stream) = open(&app, cx, &requests);
    cx.update(|_, cx| app.update(cx, |app, cx| app.terminal_update(&stream, Ok(json!({"type":"output","threadId":"t","terminalId":id,"sequence":1,"data":"\r\nsecond line"})), cx)));
    cx.run_until_parked();
    let grid = cx.debug_bounds("terminal-grid").unwrap();
    let (width, height) = app.read_with(cx, |app, _| {
        (
            app.terminal_drawer.cell_width,
            app.terminal_drawer.line_height,
        )
    });
    let start = point(
        grid.origin.x + px(width * 0.8),
        grid.origin.y + px(height + 8.),
    );
    let end = point(
        grid.origin.x + px(width * 10. + 1.),
        grid.origin.y + px(height + 8.),
    );
    cx.simulate_mouse_down(start, MouseButton::Left, Default::default());
    cx.simulate_event(gpui_kit::MouseMoveEvent {
        position: end,
        pressed_button: Some(MouseButton::Left),
        modifiers: Default::default(),
    });
    cx.simulate_mouse_up(end, MouseButton::Left, Default::default());
    app.read_with(cx, |app, _| {
        let capture = app.terminal_capture().unwrap();
        assert_eq!(capture.thread, "t");
        assert_eq!(capture.terminal_id, id);
        assert_eq!(capture.scope, app.environment_scope);
        assert_eq!((capture.line_start, capture.line_end), (2, 2));
        assert_eq!(capture.text, "second line");
    });
    cx.simulate_mouse_down(end, MouseButton::Right, Default::default());
    cx.simulate_mouse_up(end, MouseButton::Right, Default::default());
    cx.run_until_parked();
    click(cx, "terminal-menu-copy");
    assert_eq!(
        cx.read_from_clipboard().unwrap().text().as_deref(),
        Some("second line")
    );
    cx.write_to_clipboard(gpui_kit::ClipboardItem::new_string("paste\nnext".into()));
    cx.simulate_mouse_down(end, MouseButton::Right, Default::default());
    cx.simulate_mouse_up(end, MouseButton::Right, Default::default());
    cx.run_until_parked();
    click(cx, "terminal-menu-paste");
    let (payload, reply) = call(&requests, "terminal.write");
    assert_eq!(
        payload,
        json!({"threadId":"t","terminalId":id,"data":"paste\rnext"})
    );
    reply.send_blocking(Ok(Value::Null)).unwrap();
}

#[gpui_kit::test]
fn r13_h_terminal_search_focus_and_scoped_shell_run_do_not_replay(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    let (id, stream) = open(&app, cx, &requests);
    cx.update(|_, cx| app.update(cx, |app, cx| app.terminal_update(&stream, Ok(json!({"type":"output","threadId":"t","terminalId":id,"sequence":1,"data":"\r\nfindme"})), cx)));
    cx.run_until_parked();
    click(cx, "terminal-search");
    drain_sizes(cx, &requests);
    cx.simulate_input("findme");
    assert!(requests.is_empty(), "search must not write to the PTY");
    click(cx, "terminal-find-next");
    app.read_with(cx, |app, _| {
        assert_eq!(
            app.terminal_drawer.tabs[0].screen.selection().as_deref(),
            Some("findme")
        )
    });
    click(cx, "terminal-search");
    drain_sizes(cx, &requests);
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            let scope = app.environment_scope.clone();
            assert!(
                app.terminal_run_scoped("other-environment", "t", "echo nope", cx)
                    .is_err()
            );
            assert!(
                app.terminal_run_scoped(&scope, "other-thread", "echo nope", cx)
                    .is_err()
            );
            assert!(app.terminal_run_scoped(&scope, "t", "", cx).is_err());
            assert!(
                app.terminal_run_scoped(&scope, "t", "echo real", cx)
                    .is_ok()
            );
        })
    });
    let (payload, reply) = call(&requests, "terminal.write");
    assert_eq!(
        payload,
        json!({"threadId":"t","terminalId":id,"data":"echo real\r"})
    );
    reply.send_blocking(Err("PTY write failed".into())).unwrap();
    cx.run_until_parked();
    app.read_with(cx, |app, _| {
        assert_eq!(
            app.terminal_drawer.tabs[0].error.as_deref(),
            Some("PTY write failed")
        )
    });
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.connection_phase = connection::Phase::Offline;
            let scope = app.environment_scope.clone();
            assert!(
                app.terminal_run_scoped(&scope, "t", "echo must-not-replay", cx)
                    .is_err()
            );
            cx.notify();
        })
    });
    cx.run_until_parked();
    while let Ok(command) = requests.try_recv() {
        assert!(
            !matches!(command, connection::Command::Call { tag, .. } if tag == "terminal.write")
        );
    }
}

#[gpui_kit::test]
fn r13_h_terminal_same_id_in_two_threads_keeps_receipts_and_close_scoped(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    let (_, original) = open(&app, cx, &requests);
    click(cx, "terminal-discover");
    let connection::Command::Subscribe { key, .. } = requests.try_recv().unwrap() else {
        panic!("metadata")
    };
    let summary = |thread: &str| json!({"threadId":thread,"terminalId":"term-1","cwd":"/tmp","status":"running","label":thread,"hasRunningSubprocess":false});
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.terminal_update(
                &key,
                Ok(json!({"type":"snapshot","terminals":[summary("t"),summary("u")]})),
                cx,
            )
        })
    });
    cx.run_until_parked();
    assert!(
        matches!(requests.try_recv().unwrap(), connection::Command::Unsubscribe(stream) if stream == original)
    );
    let connection::Command::Subscribe {
        key: first_stream, ..
    } = requests.try_recv().unwrap()
    else {
        panic!("attach t")
    };
    cx.update(|_, cx| app.update(cx, |app, cx| app.terminal_update(&first_stream, Ok(json!({"type":"snapshot","snapshot":{"threadId":"t","terminalId":"term-1","history":"thread-t","status":"running"}})), cx)));
    drain_sizes(cx, &requests);
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.selected_thread = Some("u".into());
            app.terminal_update(
                &key,
                Ok(json!({"type":"upsert","terminal":summary("u")})),
                cx,
            );
        })
    });
    cx.run_until_parked();
    assert!(
        matches!(requests.try_recv().unwrap(), connection::Command::Unsubscribe(stream) if stream == first_stream)
    );
    let connection::Command::Subscribe {
        key: second_stream, ..
    } = requests.try_recv().unwrap()
    else {
        panic!("attach u")
    };
    cx.update(|_, cx| app.update(cx, |app, cx| app.terminal_update(&second_stream, Ok(json!({"type":"snapshot","snapshot":{"threadId":"u","terminalId":"term-1","history":"thread-u","status":"running"}})), cx)));
    drain_sizes(cx, &requests);
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            let scope = app.environment_scope.clone();
            assert!(app.terminal_run_scoped(&scope, "u", "echo u", cx).is_ok());
            app.selected_thread = Some("t".into());
            cx.notify();
        })
    });
    let (payload, reply) = call(&requests, "terminal.write");
    assert_eq!(payload["threadId"], "u");
    reply.send_blocking(Err("u-only-failure".into())).unwrap();
    cx.run_until_parked();
    assert!(
        matches!(requests.try_recv().unwrap(), connection::Command::Unsubscribe(stream) if stream == second_stream)
    );
    let connection::Command::Subscribe {
        key: restored_t, ..
    } = requests.try_recv().unwrap()
    else {
        panic!("reattach t")
    };
    cx.update(|_, cx| app.update(cx, |app, cx| app.terminal_update(&restored_t, Ok(json!({"type":"snapshot","snapshot":{"threadId":"t","terminalId":"term-1","history":"thread-t","status":"running"}})), cx)));
    drain_sizes(cx, &requests);
    app.read_with(cx, |app, _| {
        assert!(
            app.terminal_drawer
                .tabs
                .iter()
                .find(|tab| tab.thread == "t")
                .unwrap()
                .error
                .is_none()
        );
        assert_eq!(
            app.terminal_drawer
                .tabs
                .iter()
                .find(|tab| tab.thread == "u")
                .unwrap()
                .error
                .as_deref(),
            Some("u-only-failure")
        );
    });
    click(cx, "terminal-close");
    drain_sizes(cx, &requests);
    click(cx, "terminal-confirm-close");
    let (payload, reply) = call(&requests, "terminal.close");
    assert_eq!(payload["threadId"], "t");
    reply.send_blocking(Ok(Value::Null)).unwrap();
    cx.run_until_parked();
    assert!(
        matches!(requests.try_recv().unwrap(), connection::Command::Unsubscribe(stream) if stream == restored_t)
    );
    app.read_with(cx, |app, _| {
        assert_eq!(app.terminal_drawer.tabs.len(), 1);
        assert_eq!(app.terminal_drawer.tabs[0].thread, "u");
    });
}

#[gpui_kit::test]
fn r13_h_terminal_first_open_discovers_existing_session_and_keeps_custom_name(
    cx: &mut TestAppContext,
) {
    let (app, cx, requests) = harness(cx);
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.connection_phase = connection::Phase::Ready;
            app.shell
                .apply(serde_json::from_value(json!({"kind":"synchronized"})).unwrap())
                .unwrap();
            cx.notify();
        })
    });
    cx.simulate_keystrokes("cmd-j");
    cx.run_until_parked();
    let connection::Command::Subscribe { key, tag, payload } = requests.try_recv().unwrap() else {
        panic!("discover before creating")
    };
    assert_eq!(tag, "subscribeTerminalMetadata");
    assert_eq!(payload, json!({}));
    cx.update(|_, cx| app.update(cx, |app, cx| app.terminal_update(&key, Ok(json!({"type":"snapshot","terminals":[{"threadId":"t","terminalId":"term-1","cwd":"/tmp/restored","status":"running","label":"server-title","hasRunningSubprocess":true}]})), cx)));
    cx.run_until_parked();
    let connection::Command::Subscribe {
        key: stream,
        tag,
        payload,
    } = requests.try_recv().unwrap()
    else {
        panic!("restore attaches only")
    };
    assert_eq!(tag, "terminal.attach");
    assert_eq!(
        payload,
        json!({"threadId":"t","terminalId":"term-1","restartIfNotRunning":false})
    );
    cx.update(|_, cx| app.update(cx, |app, cx| app.terminal_update(&stream, Ok(json!({"type":"snapshot","snapshot":{"threadId":"t","terminalId":"term-1","history":"existing shell","status":"running","label":"server-title"}})), cx)));
    drain_sizes(cx, &requests);
    assert!(
        requests.is_empty(),
        "never create a duplicate PTY on client restart"
    );
    click(cx, "terminal-rename");
    drain_sizes(cx, &requests);
    cx.simulate_keystrokes("cmd-a");
    cx.simulate_input("My shell");
    click(cx, "terminal-rename-save");
    drain_sizes(cx, &requests);
    cx.update(|_, cx| app.update(cx, |app, cx| app.terminal_update(&stream, Ok(json!({"type":"restarted","snapshot":{"threadId":"t","terminalId":"term-1","history":"new process","status":"running","label":"different server title"}})), cx)));
    app.read_with(cx, |app, _| {
        assert_eq!(app.terminal_drawer.tabs.len(), 1);
        assert_eq!(app.terminal_drawer.tabs[0].title, "My shell");
    });
}
