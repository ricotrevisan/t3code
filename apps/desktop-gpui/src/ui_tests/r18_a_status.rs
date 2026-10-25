use super::*;

fn config_receipt(requests: &Requests, label: &str) {
    let connection::Command::Call {
        tag,
        payload,
        reply,
    } = requests.try_recv().unwrap()
    else {
        panic!("expected config receipt");
    };
    assert_eq!(tag, "server.getConfig");
    assert_eq!(payload, json!({}));
    reply
        .try_send(Ok(json!({
            "environment":{"environmentId":"office-id","label":label},
            "providers":[],"settings":{}
        })))
        .unwrap();
}

fn click(cx: &mut VisualTestContext, selector: &'static str) {
    let bounds = cx.debug_bounds(selector).unwrap();
    cx.simulate_click(bounds.center(), Default::default());
    cx.run_until_parked();
}

#[gpui_kit::test]
fn r18_a_switcher_unsaved_config_identity_phase_and_focus_reverse(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.environment_name = Some("obsolete endpoint name".into());
            app.perform("server.getConfig", json!({}), None, cx);
        })
    });
    cx.run_until_parked();
    config_receipt(&requests, "Mac mini");
    cx.run_until_parked();
    let rows = app.read_with(cx, |app, cx| app.status_environment_rows(cx));
    assert_eq!(
        rows,
        vec![(
            app.read_with(cx, |app, _| app.environment_scope.clone()),
            "Mac mini · Connected · synchronized · current".into(),
            false,
        )]
    );
    click(cx, "environment-chip");
    assert!(cx.debug_bounds("environment-choice-0").is_some());
    // The active row is not a reconnect action. Clicking it cannot issue commands.
    click(cx, "environment-choice-0");
    assert!(requests.is_empty());
    // Reopen if the kit dismissed the disabled item's menu.
    if cx.debug_bounds("environment-connection-settings").is_none() {
        click(cx, "environment-chip");
    }
    click(cx, "environment-connection-settings");
    assert!(app.read_with(cx, |app, _| app.settings_open));
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    assert!(!app.read_with(cx, |app, _| app.settings_open));
    cx.simulate_input("returned to composer");
    assert_eq!(
        app.read_with(cx, |app, cx| app.composer.read(cx).value().to_string()),
        "returned to composer"
    );
}

#[gpui_kit::test]
fn r18_a_switcher_saved_identity_updates_from_receipt_without_decorating_storage(
    cx: &mut TestAppContext,
) {
    let (app, cx, requests) = harness(cx);
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.save_environment(cx);
            app.perform("server.getConfig", json!({}), None, cx);
        })
    });
    cx.run_until_parked();
    config_receipt(&requests, "Office");
    cx.run_until_parked();
    app.read_with(cx, |app, cx| {
        assert_eq!(app.environment_choices(cx)[0].1, "Office");
        assert_eq!(
            app.status_environment_rows(cx)[0].1,
            "Office · Connected · synchronized · current"
        );
    });
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.apply_connection_phase(connection::Phase::Syncing, cx);
        })
    });
    cx.run_until_parked();
    app.read_with(cx, |app, cx| {
        assert!(!app.connection_ready());
        assert_eq!(app.environment_choices(cx)[0].1, "Office");
        assert_eq!(
            app.status_environment_rows(cx)[0].1,
            format!("Office · {} · current", app.connection_state_text())
        );
    });
    click(cx, "environment-chip");
    assert!(cx.debug_bounds("environment-choice-0").is_some());
    assert!(requests.is_empty());
}

#[gpui_kit::test]
fn r18_a_distinct_keys_same_timestamp_and_mouse_menu_are_not_debounced(cx: &mut TestAppContext) {
    let (app, cx, _) = harness(cx);
    cx.update(|_, cx| app.update(cx, |app, _| app.keystroke_guard = true));
    for (code, expected) in [(40, true), (41, false), (42, true)] {
        cx.update(|window, cx| {
            crate::platform_shell::keystrokes::simulate_native_event(cx, 1000, code);
            window.dispatch_action(Box::new(TogglePalette), cx);
        });
        cx.run_until_parked();
        assert_eq!(app.read_with(cx, |app, _| app.palette_open), expected);
        // Duplicate delivery after painting must still be ignored.
        cx.update(|window, cx| window.dispatch_action(Box::new(TogglePalette), cx));
        cx.run_until_parked();
        assert_eq!(app.read_with(cx, |app, _| app.palette_open), expected);
    }
    for expected in [false, true, false] {
        cx.update(|window, cx| {
            crate::platform_shell::keystrokes::simulate_menu_click(cx);
            window.dispatch_action(Box::new(TogglePalette), cx);
        });
        cx.run_until_parked();
        assert_eq!(app.read_with(cx, |app, _| app.palette_open), expected);
    }
    cx.simulate_input("focus survived");
    assert_eq!(
        app.read_with(cx, |app, cx| app.composer.read(cx).value().to_string()),
        "focus survived"
    );
}
