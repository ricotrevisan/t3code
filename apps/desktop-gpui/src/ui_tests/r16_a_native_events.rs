use super::*;
use crate::platform_shell::keystrokes::simulate_native_event;

#[gpui_kit::test]
fn r16_a_native_identity_menu_and_key_delivery_toggle_once_but_next_event_toggles(
    cx: &mut TestAppContext,
) {
    let (app, cx, _) = harness(cx);
    cx.update(|_, cx| app.update(cx, |app, _| app.keystroke_guard = true));
    cx.update(|window, cx| {
        simulate_native_event(cx, 1000, 40);
        window.dispatch_action(Box::new(TogglePalette), cx);
    });
    cx.run_until_parked();
    assert!(app.read_with(cx, |app, _| app.palette_open));
    // The focused input's duplicate arrives in another GPUI dispatch, with the
    // SAME AppKit event identity. No drain/frame/time boundary releases it.
    cx.simulate_keystrokes("cmd-k");
    cx.run_until_parked();
    assert!(app.read_with(cx, |app, _| app.palette_open));
    cx.simulate_input("settings");
    cx.update(|_, cx| simulate_native_event(cx, 1001, 40));
    cx.simulate_keystrokes("cmd-k");
    cx.run_until_parked();
    assert!(!app.read_with(cx, |app, _| app.palette_open));
    // Repeat delivery of the second event must not reopen the palette.
    cx.update(|window, cx| window.dispatch_action(Box::new(TogglePalette), cx));
    cx.run_until_parked();
    assert!(!app.read_with(cx, |app, _| app.palette_open));
    cx.simulate_input("draft");
    assert_eq!(
        app.read_with(cx, |app, cx| app.composer.read(cx).value().to_string()),
        "draft"
    );
}

#[gpui_kit::test]
fn r16_a_native_identity_fullscreen_menu_and_key_toggle_once(cx: &mut TestAppContext) {
    let (app, cx, _) = harness(cx);
    cx.update(|_, cx| app.update(cx, |app, _| app.keystroke_guard = true));
    cx.update(|window, cx| {
        simulate_native_event(cx, 1000, 3);
        window.dispatch_action(Box::new(crate::platform_shell::Fullscreen), cx);
    });
    cx.run_until_parked();
    assert!(cx.update(|window, _| window.is_fullscreen()));
    cx.simulate_keystrokes("ctrl-cmd-f");
    cx.run_until_parked();
    assert!(cx.update(|window, _| window.is_fullscreen()));
    cx.update(|_, cx| simulate_native_event(cx, 1001, 3));
    cx.simulate_keystrokes("ctrl-cmd-f");
    cx.run_until_parked();
    assert!(!cx.update(|window, _| window.is_fullscreen()));
}

#[gpui_kit::test]
fn r16_a_native_identity_deduplicates_mutation_and_next_repeat_sends_owned_command(
    cx: &mut TestAppContext,
) {
    let (app, cx, requests) = harness(cx);
    cx.update(|_, cx| app.update(cx, |app, _| app.keystroke_guard = true));
    for timestamp in [1000, 1001] {
        cx.update(|window, cx| {
            simulate_native_event(cx, timestamp, 35);
            window.dispatch_action(Box::new(TogglePin), cx);
        });
        cx.run_until_parked();
        cx.simulate_keystrokes("cmd-shift-p");
        cx.run_until_parked();
        let connection::Command::Call {
            tag,
            payload,
            reply,
        } = requests.try_recv().unwrap()
        else {
            panic!("pin receipt")
        };
        assert_eq!(tag, "orchestration.dispatchCommand");
        assert_eq!(payload["type"], "thread.pin");
        assert_eq!(payload["threadId"], "t");
        assert!(
            requests.is_empty(),
            "duplicate native delivery must not send twice"
        );
        reply.try_send(Err("pin rejected".into())).unwrap();
        cx.run_until_parked();
        assert_eq!(app.read_with(cx, |app, _| app.pending_actions), 0);
        assert!(app.read_with(cx, |app, _| app.shell.threads[0].pinned_at.is_none()));
    }
    // A menu mouse click immediately after a keyboard event is a new action,
    // not a duplicate key equivalent; it must not inherit the last key stamp.
    cx.update(|window, cx| {
        crate::platform_shell::keystrokes::simulate_menu_click(cx);
        window.dispatch_action(Box::new(TogglePin), cx);
    });
    cx.run_until_parked();
    let (payload, reply) = surfaces_terminal::call(&requests, "orchestration.dispatchCommand");
    assert_eq!(payload["type"], "thread.pin");
    assert_eq!(payload["threadId"], "t");
    reply
        .try_send(Err("menu click pin rejected".into()))
        .unwrap();
    cx.run_until_parked();
    assert_eq!(app.read_with(cx, |app, _| app.pending_actions), 0);
}
