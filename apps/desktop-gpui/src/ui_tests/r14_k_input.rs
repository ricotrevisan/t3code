use super::*;

fn click(cx: &mut VisualTestContext, selector: &'static str) {
    let bounds = cx
        .debug_bounds(selector)
        .unwrap_or_else(|| panic!("missing {selector}"));
    cx.simulate_mouse_move(bounds.center(), None, Default::default());
    cx.run_until_parked();
    cx.simulate_mouse_down(bounds.center(), MouseButton::Left, Default::default());
    cx.run_until_parked();
    cx.simulate_mouse_up(bounds.center(), MouseButton::Left, Default::default());
    cx.run_until_parked();
}

#[gpui_kit::test]
fn r14_k_rw22_composer_cmd_k_opens_palette_and_restores_focus(cx: &mut TestAppContext) {
    let (app, cx, _) = harness(cx);
    // Reuse the painted input subtree as the live app does between updates.
    for _ in 0..3 {
        cx.update(|_, cx| app.update(cx, |_, cx| cx.notify()));
        cx.run_until_parked();
    }
    click(cx, "composer-input");
    let mut key = gpui_kit::Keystroke::parse("cmd-k").unwrap();
    key.key_char = Some("k".into());
    cx.update(|window, cx| window.dispatch_keystroke(key, cx));
    cx.run_until_parked();
    assert!(app.read_with(cx, |app, _| app.palette_open));
    cx.simulate_input("settings");
    assert_eq!(
        app.read_with(cx, |app, cx| app.palette_query.read(cx).value().to_string()),
        "settings"
    );
    assert_eq!(
        app.read_with(cx, |app, cx| app.composer.read(cx).value().to_string()),
        ""
    );
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    cx.simulate_input("draft");
    assert_eq!(
        app.read_with(cx, |app, cx| app.composer.read(cx).value().to_string()),
        "draft"
    );
}

#[gpui_kit::test]
fn r14_k_rw21_empty_saved_environments_still_opens_switcher(cx: &mut TestAppContext) {
    let (app, cx, _) = harness(cx);
    for _ in 0..3 {
        cx.update(|_, cx| app.update(cx, |_, cx| cx.notify()));
        cx.run_until_parked();
    }
    click(cx, "environment-chip");
    assert!(cx.debug_bounds("environment-choice-0").is_some());
    click(cx, "environment-connection-settings");
    assert!(app.read_with(cx, |app, _| app.settings_open));
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    assert!(!app.read_with(cx, |app, _| app.settings_open));
}

#[gpui_kit::test]
fn r14_k_rw24_first_click_on_fresh_snapshot_selects_thread(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    cx.update(|_, cx| app.update(cx, |app, cx| {
        app.selected_thread = None;
        app.apply_shell_item(serde_json::from_value(json!({"kind":"snapshot","snapshot":{"snapshotSequence":1,"projects":[{"id":"p","title":"Fixture","workspaceRoot":"/tmp/fixture"}],"threads":[{"id":"fresh","projectId":"p","title":"Fresh thread","archivedAt":null}]}})).unwrap(), cx).unwrap();
        cx.notify();
    }));
    cx.run_until_parked();
    for _ in 0..3 {
        cx.update(|_, cx| app.update(cx, |_, cx| cx.notify()));
        cx.run_until_parked();
    }
    click(cx, "thread-fresh");
    assert_eq!(
        app.read_with(cx, |app, _| app.selected_thread.clone()),
        Some("fresh".into())
    );
    assert!(matches!(
        requests.try_recv().unwrap(),
        connection::Command::Subscribe { .. }
    ));
    assert!(
        matches!(requests.try_recv().unwrap(), connection::Command::SelectThread(Some(id)) if id == "fresh")
    );
}

/// macOS sends a menu key equivalent to both the menu and the focused input, so the
/// same action can arrive twice for one keystroke. Only the first may act.
#[gpui_kit::test]
fn menu_key_equivalent_double_dispatch_acts_once_per_keystroke(cx: &mut TestAppContext) {
    let (app, cx, _) = harness(cx);
    cx.update(|_, cx| app.update(cx, |app, _| app.keystroke_guard = true));
    cx.run_until_parked();
    click(cx, "composer-input");
    cx.update(|window, cx| {
        window.dispatch_action(Box::new(crate::TogglePalette), cx);
        window.dispatch_action(Box::new(crate::TogglePalette), cx);
    });
    cx.run_until_parked();
    assert!(app.read_with(cx, |app, _| app.palette_open));
    // A later, separate keystroke still toggles.
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(100));
    cx.run_until_parked();
    cx.update(|window, cx| window.dispatch_action(Box::new(crate::TogglePalette), cx));
    cx.run_until_parked();
    assert!(!app.read_with(cx, |app, _| app.palette_open));
}
