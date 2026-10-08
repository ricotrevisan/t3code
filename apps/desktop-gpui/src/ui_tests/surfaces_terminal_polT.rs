//! Native platform text entry and global actions with actual terminal focus.
use super::*;
use gpui_kit::{EntityInputHandler, MouseMoveEvent};

fn receipt(
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

fn ready(
    cx: &mut TestAppContext,
) -> (
    Entity<NativeApp>,
    &mut VisualTestContext,
    Requests,
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
        panic!("initial discovery")
    };
    assert_eq!(tag, "subscribeTerminalMetadata");
    assert_eq!(payload, json!({}));
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.terminal_update(&key, Ok(json!({"type":"snapshot","terminals":[]})), cx)
        })
    });
    cx.run_until_parked();
    let (payload, reply) = receipt(&requests, "terminal.open");
    let id = payload["terminalId"].as_str().unwrap().to_owned();
    reply.send_blocking(Ok(json!({"threadId":"t","terminalId":id,"history":"prompt> ","status":"running","sequence":0}))).unwrap();
    cx.run_until_parked();
    let connection::Command::Subscribe { key, .. } = requests.try_recv().unwrap() else {
        panic!("attach")
    };
    cx.update(|_, cx| app.update(cx, |app, cx| app.terminal_update(&key, Ok(json!({"type":"snapshot","snapshot":{"threadId":"t","terminalId":id,"history":"prompt> ","status":"running","sequence":0}})), cx)));
    cx.run_until_parked();
    while !requests.is_empty() {
        let (_, reply) = receipt(&requests, "terminal.resize");
        reply.send_blocking(Ok(Value::Null)).unwrap();
        cx.run_until_parked();
    }
    cx.update(|window, cx| {
        app.read_with(cx, |app, _| {
            assert!(app.terminal_drawer.focus.is_focused(window))
        })
    });
    (app, cx, requests)
}

#[gpui_kit::test]
fn terminal_pol_t_platform_insert_writes_exact_utf8_once(cx: &mut TestAppContext) {
    let (app, cx, requests) = ready(cx);
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.replace_text_in_range(None, "ü€😀", window, cx)
        })
    });
    let (payload, reply) = receipt(&requests, "terminal.write");
    assert_eq!(payload["data"], "ü€😀");
    reply.send_blocking(Ok(Value::Null)).unwrap();
    // Exercise the painted ElementInputHandler too, not just the trait implementation.
    cx.simulate_input("ü€😀");
    // The test platform delivers one insert per Unicode scalar, like successive typing.
    for text in ["ü", "€", "😀"] {
        let (payload, reply) = receipt(&requests, "terminal.write");
        assert_eq!(payload["data"], text);
        reply.send_blocking(Ok(Value::Null)).unwrap();
    }
    cx.simulate_keystrokes("a");
    let (payload, reply) = receipt(&requests, "terminal.write");
    assert_eq!(payload["data"], "a");
    reply.send_blocking(Ok(Value::Null)).unwrap();
    assert!(
        requests.is_empty(),
        "no duplicate keyDown + insertText write"
    );
}

#[gpui_kit::test]
fn terminal_pol_t_ime_mark_is_local_until_commit(cx: &mut TestAppContext) {
    let (app, cx, requests) = ready(cx);
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.replace_and_mark_text_in_range(None, "ü😀", Some(1..3), window, cx);
            assert_eq!(app.marked_text_range(window, cx), Some(0..3));
            assert_eq!(
                app.selected_text_range(false, window, cx).unwrap().range,
                1..3
            );
            let mut adjusted = None;
            assert_eq!(
                app.text_for_range(1..3, &mut adjusted, window, cx)
                    .as_deref(),
                Some("😀")
            );
            assert_eq!(adjusted, Some(1..3));
            app.replace_and_mark_text_in_range(Some(1..3), "€😀", None, window, cx);
            assert_eq!(app.marked_text_range(window, cx), Some(0..4));
        })
    });
    cx.run_until_parked();
    assert!(requests.is_empty(), "preedit must not reach the PTY");
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.replace_text_in_range(Some(0..4), "ü€😀", window, cx);
            assert_eq!(app.marked_text_range(window, cx), None);
            app.unmark_text(window, cx);
        })
    });
    let (payload, reply) = receipt(&requests, "terminal.write");
    assert_eq!(payload["data"], "ü€😀");
    reply.send_blocking(Ok(Value::Null)).unwrap();
    assert!(
        requests.is_empty(),
        "unmark after insert must not double commit"
    );
}

#[gpui_kit::test]
fn terminal_pol_t_unmark_commits_and_cancel_discards_preedit(cx: &mut TestAppContext) {
    let (app, cx, requests) = ready(cx);
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.replace_and_mark_text_in_range(None, "😀", None, window, cx);
            app.unmark_text(window, cx);
        })
    });
    let (payload, reply) = receipt(&requests, "terminal.write");
    assert_eq!(payload["data"], "😀");
    reply.send_blocking(Ok(Value::Null)).unwrap();
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.replace_and_mark_text_in_range(None, "cancel", None, window, cx);
            app.replace_and_mark_text_in_range(None, "", None, window, cx);
            app.unmark_text(window, cx);
        })
    });
    assert!(requests.is_empty());
}

#[gpui_kit::test]
fn terminal_pol_t_offline_and_stale_composition_never_write(cx: &mut TestAppContext) {
    let (app, cx, requests) = ready(cx);
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.connection_phase = connection::Phase::Offline;
            app.replace_text_in_range(None, "ü€😀", window, cx);
            app.replace_and_mark_text_in_range(None, "preedit", None, window, cx);
            assert_eq!(app.marked_text_range(window, cx), None);
            app.connection_phase = connection::Phase::Ready;
            app.replace_and_mark_text_in_range(None, "old generation", None, window, cx);
            app.generation += 1;
            app.replace_text_in_range(None, "must not replay", window, cx);
            app.unmark_text(window, cx);
            // Fence the callback without starting a reconnect subscription in this input test.
            app.generation -= 1;
        })
    });
    assert!(requests.is_empty());
}

#[gpui_kit::test]
fn terminal_pol_t_cmd_k_bubbles_but_ctrl_c_is_pty_input(cx: &mut TestAppContext) {
    let (app, cx, requests) = ready(cx);
    cx.simulate_keystrokes("ctrl-c");
    let (payload, reply) = receipt(&requests, "terminal.write");
    assert_eq!(payload["data"], "\x03");
    reply.send_blocking(Ok(Value::Null)).unwrap();
    cx.simulate_keystrokes("cmd-k");
    cx.run_until_parked();
    assert!(app.read_with(cx, |app, _| app.palette_open));
    assert!(requests.is_empty(), "Cmd-K must not send terminal bytes");
}

#[gpui_kit::test]
fn terminal_pol_t_drawer_height_drag_is_nonpersistent_in_test_and_viewport_clamped(
    cx: &mut TestAppContext,
) {
    let (app, cx, requests) = ready(cx);
    // Test windows deliberately have no persistent state: never touch live window.json.
    assert!(app.read_with(cx, |app, _| app.saved_window.is_none()));
    let divider = cx.debug_bounds("terminal-divider").unwrap();
    let start = divider.center();
    let end = point(start.x, start.y - px(60.));
    cx.simulate_mouse_down(start, MouseButton::Left, Default::default());
    cx.simulate_event(MouseMoveEvent {
        position: end,
        pressed_button: Some(MouseButton::Left),
        modifiers: Default::default(),
    });
    cx.simulate_mouse_up(end, MouseButton::Left, Default::default());
    cx.run_until_parked();
    assert_eq!(app.read_with(cx, |app, _| app.terminal_drawer.height), 320.);
    assert!(app.read_with(cx, |app, _| app.saved_window.is_none()));
    while !requests.is_empty() {
        let (_, reply) = receipt(&requests, "terminal.resize");
        reply.send_blocking(Ok(Value::Null)).unwrap();
        cx.run_until_parked();
    }
    cx.simulate_resize(size(px(1100.), px(360.)));
    cx.run_until_parked();
    assert!(app.read_with(cx, |app, _| app.terminal_drawer.height <= 270.));
}
