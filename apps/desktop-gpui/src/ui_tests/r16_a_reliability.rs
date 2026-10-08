use super::*;

fn click(cx: &mut VisualTestContext, selector: &'static str) {
    let bounds = cx
        .debug_bounds(selector)
        .unwrap_or_else(|| panic!("missing {selector}"));
    cx.simulate_mouse_move(bounds.center(), None, Default::default());
    cx.run_until_parked();
    cx.simulate_click(bounds.center(), Default::default());
    cx.run_until_parked();
}

#[gpui_kit::test]
fn r16_a_full_layout_status_hit_test_after_terminal_hide_with_toast_and_browser(
    cx: &mut TestAppContext,
) {
    let (app, cx, requests) = harness(cx);
    cx.update(|window, cx| app.update(cx, |app, cx| {
        app.environment_name = Some("Mac mini · Office".into());
        app.shell.threads[0].branch = Some("feature/native-shell".into());
        app.apply_thread_update("t", snapshot((0..100).map(|i| json!({"id":format!("u{i}"),"role":"user","text":format!("Prompt {i}")})).collect(), "idle"), cx).unwrap();
        app.composer.update(cx, |input, cx| input.set_value("draft\n".repeat(10), window, cx));
        cx.notify();
    }));
    cx.run_until_parked();
    assert!(cx.debug_bounds("timeline-minimap").is_some());
    surfaces_terminal::open(&app, cx, &requests);
    click(cx, "terminal-hide");
    assert!(cx.debug_bounds("terminal-drawer").is_none());
    click(cx, "browser-add");
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            for i in 0..4 {
                app.report_error(format!("Hit-test fixture failure {i}"), cx);
            }
        })
    });
    cx.run_until_parked();
    assert!(cx.debug_bounds("toast-stack").is_some());
    assert!(cx.debug_bounds("browser-surface-body").is_some());
    for (width, height, zoom) in [
        (1280., 983., 0),
        (1000., 700., 0),
        (960., 600., 0),
        (1000., 700., 5),
        (848., 620., 5),
    ] {
        cx.update(|window, cx| {
            app.update(cx, |_, cx| {
                crate::theme::update(window, cx, |preferences| preferences.zoom = zoom).unwrap();
                cx.notify();
            })
        });
        cx.simulate_resize(size(px(width), px(height)));
        cx.run_until_parked();
        let bar = cx.debug_bounds("status-bar").unwrap();
        let chip = cx.debug_bounds("environment-chip").unwrap();
        let viewport = gpui_kit::Bounds::new(point(px(0.), px(0.)), size(px(width), px(height)));
        assert!(
            viewport.contains(&chip.center()),
            "chip must be inside physical window: {chip:?}"
        );
        assert!(
            bar.bottom() <= viewport.bottom(),
            "footer overflow: {bar:?}"
        );
        assert!(
            bar.contains(&chip.center()),
            "chip outside status bar: {chip:?} {bar:?}"
        );
        let message = cx.debug_bounds("status-message").unwrap();
        assert!(
            message.size.width >= px(96.),
            "feedback must retain readable width: {message:?}"
        );
        assert!(
            viewport.contains(&message.center()),
            "feedback must stay inside the physical window: {message:?}"
        );
        let toast = cx.debug_bounds("toast-stack").unwrap();
        assert!(
            toast.bottom() <= bar.top(),
            "toast must not occupy footer: {toast:?} {bar:?}"
        );
        click(cx, "environment-chip");
        assert!(
            cx.debug_bounds("environment-choice-0").is_some(),
            "current endpoint must open at {width}x{height}"
        );
        click(cx, "environment-connection-settings");
        assert!(app.read_with(cx, |app, _| app.settings_open));
        cx.simulate_keystrokes("escape");
        cx.run_until_parked();
        let usage = cx.debug_bounds("usage-limits-entry").unwrap();
        assert!(
            viewport.contains(&usage.center()),
            "usage must be inside physical window: {usage:?}"
        );
        click(cx, "usage-limits-entry");
        assert!(
            cx.debug_bounds("usage-limits-panel").is_some(),
            "usage must open at {width}x{height}"
        );
        click(cx, "usage-close");
        assert!(cx.debug_bounds("usage-limits-panel").is_none());
    }
}

#[gpui_kit::test]
fn r16_a_distinct_rapid_cmd_k_toggles_from_palette_input(cx: &mut TestAppContext) {
    let (app, cx, _) = harness(cx);
    cx.update(|_, cx| app.update(cx, |app, _| app.keystroke_guard = true));
    click(cx, "composer-input");
    cx.simulate_keystrokes("cmd-k");
    cx.run_until_parked();
    assert!(app.read_with(cx, |app, _| app.palette_open));
    cx.simulate_input("settings");
    cx.simulate_keystrokes("cmd-k");
    cx.run_until_parked();
    assert!(
        !app.read_with(cx, |app, _| app.palette_open),
        "a separate keypress is not a duplicate, even inside 80ms"
    );
    cx.simulate_input("draft");
    assert_eq!(
        app.read_with(cx, |app, cx| app.composer.read(cx).value().to_string()),
        "draft"
    );
}
