use super::surfaces_terminal::{call, drain_sizes, open};
use super::*;
use t3code_desktop_gpui::draft_context::DraftContextRecord;

#[gpui_kit::test]
fn r13_int_terminal_menu_capture_send_failure_and_exact_accepted_consumption(
    cx: &mut TestAppContext,
) {
    let (app, cx, requests) = harness(cx);
    let (id, stream) = open(&app, cx, &requests);
    cx.update(|_, cx| app.update(cx, |app, cx| {
        app.provider_catalog = vec![composer_parity::provider_fixture("codex", "Personal")];
        app.terminal_update(&stream, Ok(json!({"type":"output","threadId":"t","terminalId":id,"sequence":1,"data":"\r\nsecond line"})), cx);
    }));
    cx.run_until_parked();
    let grid = cx.debug_bounds("terminal-grid").unwrap();
    let (width, height) = app.read_with(cx, |app, _| {
        (
            app.terminal_drawer.cell_width,
            app.terminal_drawer.line_height,
        )
    });
    let start = point(grid.left() + px(width * 0.8), grid.top() + px(height + 8.));
    let end = point(
        grid.left() + px(width * 10. + 1.),
        grid.top() + px(height + 8.),
    );
    cx.simulate_mouse_down(start, MouseButton::Left, Default::default());
    cx.simulate_event(gpui_kit::MouseMoveEvent {
        position: end,
        pressed_button: Some(MouseButton::Left),
        modifiers: Default::default(),
    });
    cx.simulate_mouse_up(end, MouseButton::Left, Default::default());
    cx.update(|window, cx| window.dispatch_action(Box::new(gpui_kit::component::input::Copy), cx));
    cx.run_until_parked();
    assert_eq!(
        cx.read_from_clipboard().unwrap().text().as_deref(),
        Some("second line")
    );
    cx.simulate_mouse_down(end, MouseButton::Right, Default::default());
    cx.simulate_mouse_up(end, MouseButton::Right, Default::default());
    cx.run_until_parked();
    let menu = cx.debug_bounds("terminal-menu-chat").unwrap();
    cx.simulate_click(menu.center(), Default::default());
    cx.run_until_parked();
    let expected = app.read_with(cx, |app, _| app.draft_context_records()[0].value().clone());
    assert_eq!(expected["kind"], "terminal");
    assert_eq!(expected["terminalId"], id);
    assert_eq!(expected["lineStart"], 2);
    assert_eq!(expected["lineEnd"], 2);
    assert_eq!(expected["text"], "second line");
    drain_sizes(cx, &requests);
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    let (payload, reply) = call(&requests, "orchestration.dispatchCommand");
    assert_eq!(payload["type"], "thread.turn.start");
    assert_eq!(payload["message"]["context"]["records"][0], expected);
    reply.send_blocking(Err("Rejected quote".into())).unwrap();
    cx.run_until_parked();
    app.read_with(cx, |app, _| {
        assert_eq!(app.draft_context_records()[0].value(), &expected)
    });
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    let (_, reply) = call(&requests, "orchestration.dispatchCommand");
    let later =
        DraftContextRecord::terminal("Later", "term-later", "Later", 1, 1, "later").unwrap();
    let later_id = later.id().to_owned();
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            let scope = app.environment_scope.clone();
            app.intake_draft_context(&scope, "t", later, window, cx)
                .unwrap();
        })
    });
    reply.send_blocking(Ok(json!({"sequence":4}))).unwrap();
    cx.run_until_parked();
    app.read_with(cx, |app, cx| {
        assert_eq!(app.draft_context_records().len(), 1);
        assert_eq!(app.draft_context_records()[0].id(), later_id);
        assert!(app.composer.read(cx).value().contains(&later_id));
    });
}

#[gpui_kit::test]
fn r13_int_palette_usage_opens_and_closes_without_consuming_draft(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    cx.simulate_input("keep this draft");
    cx.simulate_keystrokes("cmd-k");
    cx.simulate_input("usage limits");
    cx.run_until_parked();
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    assert!(cx.debug_bounds("usage-limits-panel").is_some());
    let close = cx.debug_bounds("usage-close").unwrap();
    cx.simulate_click(close.center(), Default::default());
    cx.run_until_parked();
    assert!(cx.debug_bounds("usage-limits-panel").is_none());
    app.read_with(cx, |app, cx| {
        assert_eq!(app.composer.read(cx).value().as_str(), "keep this draft")
    });
    assert!(requests.is_empty());
}
