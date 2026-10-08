//! Real grid focus/key dispatch, PTY receipts, scoped output and reconnect ownership.
use super::*;
use t3code_desktop_gpui::terminal::Size;

fn call(
    requests: &async_channel::Receiver<connection::Command>,
    expected: &str,
) -> (Value, async_channel::Sender<Result<Value, String>>) {
    let connection::Command::Call {
        tag,
        payload,
        reply,
    } = requests
        .try_recv()
        .unwrap_or_else(|error| panic!("expected {expected}: {error}"))
    else {
        panic!("expected {expected}")
    };
    assert_eq!(tag, expected);
    (payload, reply)
}
fn open(
    app: &Entity<NativeApp>,
    cx: &mut VisualTestContext,
    requests: &async_channel::Receiver<connection::Command>,
) -> (String, String) {
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.connection_phase = connection::Phase::Ready;
            app.shell
                .apply(serde_json::from_value(json!({"kind":"synchronized"})).unwrap())
                .unwrap();
            app.shell.threads[0].worktree_path = Some("/tmp/selected-checkout".into());
            cx.notify();
        })
    });
    cx.simulate_keystrokes("cmd-j");
    cx.run_until_parked();
    assert!(cx.debug_bounds("terminal-drawer").is_some());
    let (payload, reply) = call(requests, "terminal.open");
    assert_eq!(payload["cwd"], "/tmp/selected-checkout");
    assert_eq!(payload["worktreePath"], "/tmp/selected-checkout");
    assert_eq!(payload["threadId"], "t");
    let id = payload["terminalId"].as_str().unwrap().to_owned();
    reply.send_blocking(Ok(json!({"threadId":"t","terminalId":id,"history":"prompt> ","status":"running","sequence":0}))).unwrap();
    cx.run_until_parked();
    let connection::Command::Subscribe { key, tag, payload } = requests.try_recv().unwrap() else {
        panic!("attach after open")
    };
    assert_eq!(tag, "terminal.attach");
    assert_eq!(payload["restartIfNotRunning"], false);
    cx.update(|_, cx| app.update(cx, |app, cx| app.terminal_update(&key, Ok(json!({"type":"snapshot","snapshot":{"threadId":"t","terminalId":id,"history":"prompt> ","status":"running","sequence":0}})), cx)));
    // Rendered grid sizing is a real PTY resize, not a guessed window dimension.
    drain_sizes(cx, requests);
    (id, key)
}
fn drain_sizes(
    cx: &mut VisualTestContext,
    requests: &async_channel::Receiver<connection::Command>,
) {
    cx.run_until_parked();
    while !requests.is_empty() {
        let (size, reply) = call(requests, "terminal.resize");
        assert!(size["cols"].as_u64().unwrap() > 2);
        assert!(size["rows"].as_u64().unwrap() > 1);
        reply.send_blocking(Ok(Value::Null)).unwrap();
        cx.run_until_parked();
    }
}
#[gpui_kit::test]
fn terminal_toggle_checkout_keyboard_and_clipboard_payloads(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    let (id, _) = open(&app, cx, &requests);
    cx.update(|window, cx| {
        app.read_with(cx, |app, _| {
            assert!(
                app.terminal_drawer.focus.is_focused(window),
                "terminal owns focus"
            )
        })
    });
    cx.simulate_input("a");
    cx.simulate_keystrokes("enter backspace tab up down left right ctrl-c escape");
    for data in [
        "a", "\r", "\x7f", "\t", "\x1b[A", "\x1b[B", "\x1b[D", "\x1b[C", "\x03", "\x1b",
    ] {
        assert!(!requests.is_empty(), "missing key payload {data:?}");
        let (payload, reply) = call(&requests, "terminal.write");
        assert_eq!(payload, json!({"threadId":"t","terminalId":id,"data":data}));
        reply.send_blocking(Ok(Value::Null)).unwrap();
    }
    cx.write_to_clipboard(gpui_kit::ClipboardItem::new_string("paste\nnext".into()));
    cx.simulate_keystrokes("cmd-v");
    let (payload, reply) = call(&requests, "terminal.write");
    assert_eq!(payload["data"], "paste\rnext");
    reply.send_blocking(Ok(Value::Null)).unwrap();
    let grid = cx.debug_bounds("terminal-grid").unwrap();
    let width = app.read_with(cx, |app, _| app.terminal_drawer.cell_width);
    // Stay inside cell zero, clear of the sidebar divider's 4px hit slop.
    let start = point(grid.origin.x + px(width * 0.8), grid.origin.y + px(8.));
    let end = point(grid.origin.x + px(width * 6. + 1.), grid.origin.y + px(8.));
    cx.simulate_mouse_down(start, MouseButton::Left, Default::default());
    cx.run_until_parked();
    app.read_with(cx, |app, _| {
        assert!(app.terminal_drawer.tabs[0].screen.term.selection.is_some())
    });
    cx.simulate_event(gpui_kit::MouseMoveEvent {
        position: end,
        pressed_button: Some(MouseButton::Left),
        modifiers: Default::default(),
    });
    cx.simulate_mouse_up(end, MouseButton::Left, Default::default());
    app.read_with(cx, |app, _| {
        assert_eq!(
            app.terminal_drawer.tabs[0].screen.selection().as_deref(),
            Some("prompt>"),
            "mouse selection"
        )
    });
    cx.simulate_keystrokes("cmd-c");
    assert_eq!(
        cx.read_from_clipboard().unwrap().text().as_deref(),
        Some("prompt>")
    );
    cx.simulate_keystrokes("cmd-j");
    cx.run_until_parked();
    assert!(cx.debug_bounds("terminal-drawer").is_none());
    cx.simulate_keystrokes("cmd-j");
    cx.run_until_parked();
    assert!(cx.debug_bounds("terminal-grid").is_some());
    assert!(requests.is_empty());
}
#[gpui_kit::test]
fn terminal_output_grid_resize_and_close_running_confirmation(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    let (id, stream) = open(&app, cx, &requests);
    cx.update(|_, cx| app.update(cx, |app, cx| {
        app.terminal_update(&stream, Ok(json!({"type":"output","threadId":"t","terminalId":id,"sequence":1,"data":"\x1b[2J\x1b[H\x1b[32mGREEN"})), cx);
        app.terminal_update(&stream, Ok(json!({"type":"output","threadId":"t","terminalId":id,"sequence":1,"data":"duplicate"})), cx);
        app.terminal_update(&stream, Ok(json!({"type":"activity","threadId":"t","terminalId":id,"sequence":2,"hasRunningSubprocess":true})), cx);
    }));
    cx.run_until_parked();
    assert!(cx.debug_bounds("terminal-grid").unwrap().size.width > px(100.));
    app.read_with(cx, |app, _| {
        let cells = app.terminal_drawer.tabs[0].screen.cells();
        assert_eq!(
            cells
                .iter()
                .take(5)
                .map(|cell| cell.text.as_str())
                .collect::<String>(),
            "GREEN"
        );
    });
    cx.simulate_resize(size(px(1100.), px(900.)));
    cx.run_until_parked();
    let (payload, reply) = call(&requests, "terminal.resize");
    assert_eq!(payload["terminalId"], id);
    reply.send_blocking(Ok(Value::Null)).unwrap();
    cx.run_until_parked();
    let bounds = cx.debug_bounds("terminal-close").unwrap();
    cx.simulate_click(bounds.center(), Default::default());
    cx.run_until_parked();
    drain_sizes(cx, &requests);
    assert!(cx.debug_bounds("terminal-close-confirm").is_some());
    let bounds = cx.debug_bounds("terminal-cancel-close").unwrap();
    cx.simulate_click(bounds.center(), Default::default());
    cx.run_until_parked();
    drain_sizes(cx, &requests);
    let bounds = cx.debug_bounds("terminal-close").unwrap();
    cx.simulate_click(bounds.center(), Default::default());
    cx.run_until_parked();
    drain_sizes(cx, &requests);
    let bounds = cx.debug_bounds("terminal-confirm-close").unwrap();
    cx.simulate_click(bounds.center(), Default::default());
    cx.run_until_parked();
    let (_, reply) = call(&requests, "terminal.close");
    reply.send_blocking(Ok(Value::Null)).unwrap();
    cx.run_until_parked();
    assert!(
        matches!(requests.try_recv().unwrap(), connection::Command::Unsubscribe(key) if key == stream)
    );
    app.read_with(cx, |app, _| assert!(app.terminal_drawer.tabs.is_empty()));
}
#[gpui_kit::test]
fn terminal_reconnect_reattaches_without_reopening_and_rejects_stale_output(
    cx: &mut TestAppContext,
) {
    let (app, cx, requests) = harness(cx);
    let (id, old_stream) = open(&app, cx, &requests);
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.connection_phase = connection::Phase::Offline;
            cx.notify();
        })
    });
    cx.run_until_parked();
    cx.simulate_keystrokes("x");
    assert!(requests.is_empty());
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.generation += 1;
            app.connection_phase = connection::Phase::Ready;
            cx.notify();
        })
    });
    cx.run_until_parked();
    let connection::Command::Subscribe { key, tag, payload } = requests.try_recv().unwrap() else {
        panic!("reattach, never open")
    };
    assert_eq!(tag, "terminal.attach");
    assert_eq!(
        payload,
        json!({"threadId":"t","terminalId":id,"restartIfNotRunning":false})
    );
    assert_ne!(key, old_stream);
    drain_sizes(cx, &requests);
    cx.update(|_, cx| app.update(cx, |app, cx| {
        app.terminal_update(&old_stream, Ok(json!({"type":"output","threadId":"t","terminalId":id,"sequence":99,"data":"STALE"})), cx);
        app.terminal_update(&key, Ok(json!({"type":"snapshot","snapshot":{"threadId":"t","terminalId":id,"sequence":10,"status":"running","history":"restored"}})), cx);
        let cells = app.terminal_drawer.tabs[0].screen.cells(); assert_eq!(cells.iter().take(8).map(|cell| cell.text.as_str()).collect::<String>(), "restored");
        app.terminal_resize(&id, Size::new(40, 8), cx);
    }));
    let (size, reply) = call(&requests, "terminal.resize");
    assert_eq!(size["cols"], 40);
    reply.send_blocking(Ok(Value::Null)).unwrap();
}
#[gpui_kit::test]
fn terminal_tabs_rename_and_old_environment_open_receipt_is_fenced(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    let (_, _) = open(&app, cx, &requests);
    let bounds = cx.debug_bounds("terminal-rename").unwrap();
    cx.simulate_click(bounds.center(), Default::default());
    cx.run_until_parked();
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.terminal_drawer
                .rename
                .update(cx, |input, cx| input.set_value("Build shell", window, cx))
        })
    });
    drain_sizes(cx, &requests);
    let bounds = cx.debug_bounds("terminal-rename-save").unwrap();
    cx.simulate_click(bounds.center(), Default::default());
    cx.run_until_parked();
    app.read_with(cx, |app, _| {
        assert_eq!(app.terminal_drawer.tabs[0].title, "Build shell")
    });
    drain_sizes(cx, &requests);
    let bounds = cx.debug_bounds("terminal-new").unwrap();
    cx.simulate_click(bounds.center(), Default::default());
    cx.run_until_parked();
    let (payload, reply) = call(&requests, "terminal.open");
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.generation += 1;
            app.environment_scope = "different-server".into();
            cx.notify();
        })
    });
    reply.send_blocking(Ok(json!({"threadId":"t","terminalId":payload["terminalId"],"history":"old server","status":"running"}))).unwrap();
    cx.run_until_parked();
    assert!(requests.is_empty());
    app.read_with(cx, |app, _| {
        assert!(app.terminal_drawer.tabs[1].exists);
        assert!(app.terminal_drawer.tabs[1].error.is_some());
    });
}

#[gpui_kit::test]
fn terminal_drawer_drag_resizes_grid_and_idle_render_does_not_repeat_rpc(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    let (_, _) = open(&app, cx, &requests);
    let divider = cx.debug_bounds("terminal-divider").unwrap();
    let start = divider.center();
    let end = point(start.x, start.y - px(80.));
    cx.simulate_mouse_down(start, MouseButton::Left, Default::default());
    cx.simulate_event(gpui_kit::MouseMoveEvent {
        position: end,
        pressed_button: Some(MouseButton::Left),
        modifiers: Default::default(),
    });
    cx.simulate_mouse_up(end, MouseButton::Left, Default::default());
    cx.run_until_parked();
    app.read_with(cx, |app, _| assert_eq!(app.terminal_drawer.height, 340.));
    assert!(!requests.is_empty());
    drain_sizes(cx, &requests);
    cx.update(|_, cx| app.update(cx, |_, cx| cx.notify()));
    cx.run_until_parked();
    assert!(
        requests.is_empty(),
        "idle grid does not produce another resize or write"
    );
}
