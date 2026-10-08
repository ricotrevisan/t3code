//! Cancellation retries retain the original stream owners, without replaying PTY input.
use super::surfaces_terminal::{call, drain_sizes, open};
use super::*;

#[gpui_kit::test]
fn r14_t_hidden_output_cancellation_survives_full_channel_and_reopen_reattaches(
    cx: &mut TestAppContext,
) {
    let (app, cx, requests) = harness(cx);
    let (id, original) = open(&app, cx, &requests);
    let sender = app.read_with(cx, |app, _| app.commands.clone().unwrap());
    for n in 0..16 {
        sender
            .try_send(connection::Command::Unsubscribe(format!("filler-{n}")))
            .unwrap();
    }
    cx.simulate_keystrokes("cmd-j");
    cx.run_until_parked();
    assert!(cx.debug_bounds("terminal-drawer").is_none());
    for _ in 0..16 {
        assert!(matches!(
            requests.try_recv().unwrap(),
            connection::Command::Unsubscribe(_)
        ));
    }
    cx.simulate_keystrokes("cmd-j");
    cx.run_until_parked();
    let mut retired = Vec::new();
    let key = loop {
        match requests.try_recv().unwrap() {
            connection::Command::Unsubscribe(key) => retired.push(key),
            connection::Command::Subscribe { key, tag, payload } => {
                assert_eq!(tag, "terminal.attach");
                assert_eq!(
                    payload,
                    json!({"threadId":"t","terminalId":id,"restartIfNotRunning":false})
                );
                break key;
            }
            _ => panic!("reopening must only retire and reattach, never create/restart/write"),
        }
    };
    assert!(retired.contains(&original));
    assert!(
        retired
            .iter()
            .any(|key| key.starts_with("drawer-metadata-"))
    );
    assert_ne!(key, original);
    drain_sizes(cx, &requests);
    cx.simulate_input("blocked");
    assert!(
        requests.is_empty(),
        "attach request is not an authoritative snapshot"
    );
    cx.update(|_, cx| app.update(cx, |app, cx| app.terminal_update(&key, Ok(json!({"type":"snapshot","snapshot":{"threadId":"t","terminalId":id,"history":"restored","status":"running"}})), cx)));
    drain_sizes(cx, &requests);
    cx.simulate_input("猫");
    let (payload, reply) = call(&requests, "terminal.write");
    assert_eq!(payload["data"], "猫");
    reply.send_blocking(Ok(Value::Null)).unwrap();
    assert!(requests.is_empty());
}

#[gpui_kit::test]
fn r14_t_failed_open_releases_screen_and_keeps_failure_visible(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    let (id, original) = open(&app, cx, &requests);
    let bounds = cx.debug_bounds("terminal-new").unwrap();
    cx.simulate_click(bounds.center(), Default::default());
    cx.run_until_parked();
    assert!(
        matches!(requests.try_recv().unwrap(), connection::Command::Unsubscribe(key) if key == original)
    );
    let (_, reply) = call(&requests, "terminal.open");
    reply
        .send_blocking(Err("shell executable unavailable".into()))
        .unwrap();
    cx.run_until_parked();
    let connection::Command::Subscribe { key, tag, payload } = requests.try_recv().unwrap() else {
        panic!("restore previous terminal output, never reopen/restart")
    };
    assert_eq!(tag, "terminal.attach");
    assert_eq!(payload["terminalId"], id);
    cx.update(|_, cx| app.update(cx, |app, cx| app.terminal_update(&key, Ok(json!({"type":"snapshot","snapshot":{"threadId":"t","terminalId":id,"history":"original shell","status":"running"}})), cx)));
    drain_sizes(cx, &requests);
    app.read_with(cx, |app, _| {
        assert_eq!(app.terminal_drawer.tabs.len(), 1);
        assert_eq!(
            app.terminal_drawer.local_error.as_deref(),
            Some("Terminal open failed: shell executable unavailable")
        );
    });
    assert!(requests.is_empty());
}
