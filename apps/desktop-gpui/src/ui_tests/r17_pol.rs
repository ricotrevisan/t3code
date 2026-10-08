use super::*;

#[gpui_kit::test]
fn r17_pol_rw27_unsaved_current_choice_carries_only_identity(cx: &mut TestAppContext) {
    let (app, cx, _) = harness(cx);
    assert!(app.read_with(cx, |app, cx| app.environment_choices(cx).is_empty()));
    let choices = app.read_with(cx, |app, cx| app.status_environment_choices("Mac mini", cx));
    assert_eq!(choices.len(), 1);
    assert_eq!(choices[0].1, "Mac mini");
    assert!(choices[0].2);
    let bounds = cx.debug_bounds("environment-chip").unwrap();
    cx.simulate_click(bounds.center(), Default::default());
    cx.run_until_parked();
    assert!(cx.debug_bounds("environment-choice-0").is_some());
}

#[gpui_kit::test]
fn r17_pol_rw28_compact_credit_action_uses_owned_receipt_and_can_cancel(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.perform("server.getConfig", json!({}), None, cx)
        })
    });
    cx.run_until_parked();
    let connection::Command::Call {
        tag,
        payload,
        reply,
    } = requests.try_recv().unwrap()
    else {
        panic!("config request")
    };
    assert_eq!(tag, "server.getConfig");
    assert_eq!(payload, json!({}));
    reply.send_blocking(Ok(json!({"providers":[{"instanceId":"codex","driver":"codex","enabled":true,"installed":true,"usageLimits":{"checkedAt":"2026-10-07T20:16:06.587Z","windows":[{"label":"Weekly","usedPercent":20,"resetsAt":"2026-10-14T05:24:43.000Z"}],"resetCredits":{"availableCount":2,"nextExpiresAt":"2026-11-01T00:00:00Z"}}}],"usageLimitSources":[],"settings":{}}))).unwrap();
    cx.run_until_parked();
    let bounds = cx.debug_bounds("usage-limits-entry").unwrap();
    cx.simulate_click(bounds.center(), Default::default());
    cx.run_until_parked();
    let row = cx.debug_bounds("usage-credit-row-0").unwrap();
    let button = cx.debug_bounds("usage-credit-0").unwrap();
    assert!(
        button.size.width < row.size.width * 0.5,
        "credit action must not stretch across the account"
    );
    cx.simulate_click(button.center(), Default::default());
    cx.run_until_parked();
    assert!(cx.debug_bounds("usage-confirm-0").is_some());
    assert!(requests.try_recv().is_err());
    let cancel = cx.debug_bounds("usage-cancel-0").unwrap();
    cx.simulate_click(cancel.center(), Default::default());
    cx.run_until_parked();
    assert!(cx.debug_bounds("usage-confirm-0").is_none());
    assert!(requests.try_recv().is_err());
}

#[gpui_kit::test]
fn r17_pol_rw25_native_menu_and_focused_input_toggle_once_per_event(cx: &mut TestAppContext) {
    let (app, cx, _) = harness(cx);
    cx.update(|_, cx| app.update(cx, |app, _| app.keystroke_guard = true));
    for (event, expected) in [(1, true), (2, false), (3, true)] {
        cx.update(|window, cx| {
            crate::platform_shell::keystrokes::simulate_native_event(cx, event, 40);
            window.dispatch_action(Box::new(crate::TogglePalette), cx);
            window.dispatch_action(Box::new(crate::TogglePalette), cx);
        });
        cx.run_until_parked();
        assert_eq!(app.read_with(cx, |app, _| app.palette_open), expected);
    }
}

#[gpui_kit::test]
fn r17_pol_rw25_cmd_k_toggles_from_palette_input_and_restores_composer(cx: &mut TestAppContext) {
    let (app, cx, _) = harness(cx);
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.composer.update(cx, |input, cx| input.focus(window, cx))
        })
    });
    cx.simulate_keystrokes("cmd-k");
    cx.run_until_parked();
    assert!(app.read_with(cx, |app, _| app.palette_open));
    cx.simulate_input("settings");
    assert_eq!(
        app.read_with(cx, |app, cx| app.palette_query.read(cx).value().to_string()),
        "settings"
    );
    cx.simulate_keystrokes("cmd-k");
    cx.run_until_parked();
    assert!(!app.read_with(cx, |app, _| app.palette_open));
    cx.simulate_input("draft");
    assert_eq!(
        app.read_with(cx, |app, cx| app.composer.read(cx).value().to_string()),
        "draft"
    );
    cx.simulate_keystrokes("cmd-k");
    cx.run_until_parked();
    assert!(app.read_with(cx, |app, _| app.palette_open));
}
