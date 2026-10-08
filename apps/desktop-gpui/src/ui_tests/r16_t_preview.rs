//! Streamed link selection reaches the owning native browser, with truthful relay denial.
use super::surfaces_terminal::{call, drain_sizes, open};
use super::*;

#[gpui_kit::test]
fn r16_t_selected_streamed_file_uses_authoritative_editor_rpc_and_denial(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    cx.update(|_, cx| {
        app.update(cx, |app, _| {
            app.available_editors = vec!["zed".into()];
            assert!(!app.is_remote_editor_checkout());
        })
    });
    let (id, stream) = open(&app, cx, &requests);
    let path = "src/main.rs:2:3";
    cx.update(|_, cx| app.update(cx, |app, cx| app.terminal_update(&stream, Ok(json!({"type":"snapshot","snapshot":{"threadId":"t","terminalId":id,"history":format!("{path}\r\n"),"status":"running","sequence":1}})), cx)));
    drain_sizes(cx, &requests);
    let grid = cx.debug_bounds("terminal-grid").unwrap();
    let (width, height) = app.read_with(cx, |app, _| {
        (
            app.terminal_drawer.cell_width,
            app.terminal_drawer.line_height,
        )
    });
    let start = point(
        grid.origin.x + px(width * 0.5),
        grid.origin.y + px(height * 0.5),
    );
    let end = point(
        grid.origin.x + px(width * (path.len() as f32 - 0.5)),
        start.y,
    );
    cx.simulate_mouse_down(start, MouseButton::Left, Default::default());
    cx.simulate_event(gpui_kit::MouseMoveEvent {
        position: end,
        pressed_button: Some(MouseButton::Left),
        modifiers: Default::default(),
    });
    cx.simulate_mouse_up(end, MouseButton::Left, Default::default());
    cx.simulate_mouse_down(end, MouseButton::Right, Default::default());
    cx.simulate_mouse_up(end, MouseButton::Right, Default::default());
    let item = cx.debug_bounds("terminal-menu-editor").unwrap();
    cx.simulate_click(item.center(), Default::default());
    cx.run_until_parked();
    let (payload, reply) = call(&requests, "shell.openInEditor");
    assert_eq!(
        payload,
        json!({"cwd":"/tmp/selected-checkout/src/main.rs","editor":"zed"})
    );
    reply.send_blocking(Err("editor denied".into())).unwrap();
    cx.run_until_parked();
    app.read_with(cx, |app, _| assert!(app.status.contains("editor denied")));
    assert_eq!(cx.opened_url(), None);
    assert!(
        requests.is_empty(),
        "an editor route must never inject a shell command"
    );
}

#[gpui_kit::test]
fn r16_t_selected_streamed_url_opens_preview_and_relay_denial_allocates_no_tab(
    cx: &mut TestAppContext,
) {
    let (app, cx, requests) = harness(cx);
    let (id, stream) = open(&app, cx, &requests);
    let url = "http://localhost:5173/app";
    cx.update(|window, cx| app.update(cx, |app, cx| {
        app.server.update(cx, |input, cx| input.set_value("http://100.124.111.25:8789", window, cx));
        app.terminal_update(&stream, Ok(json!({"type":"snapshot","snapshot":{"threadId":"t","terminalId":id,"history":format!("{url}\r\n"),"status":"running","sequence":1}})), cx);
    }));
    drain_sizes(cx, &requests);
    let select_and_menu = |cx: &mut VisualTestContext| {
        let grid = cx.debug_bounds("terminal-grid").unwrap();
        let (width, height) = app.read_with(cx, |app, _| {
            (
                app.terminal_drawer.cell_width,
                app.terminal_drawer.line_height,
            )
        });
        let start = point(
            grid.origin.x + px(width * 0.5),
            grid.origin.y + px(height * 0.5),
        );
        let end = point(
            grid.origin.x + px(width * (url.len() as f32 - 0.5)),
            start.y,
        );
        cx.simulate_mouse_down(start, MouseButton::Left, Default::default());
        cx.simulate_event(gpui_kit::MouseMoveEvent {
            position: end,
            pressed_button: Some(MouseButton::Left),
            modifiers: Default::default(),
        });
        cx.simulate_mouse_up(end, MouseButton::Left, Default::default());
        app.read_with(cx, |app, _| {
            assert_eq!(app.terminal_capture().unwrap().text, url)
        });
        cx.simulate_mouse_down(end, MouseButton::Right, Default::default());
        cx.simulate_mouse_up(end, MouseButton::Right, Default::default());
        cx.run_until_parked();
        let item = cx.debug_bounds("terminal-menu-preview").unwrap();
        cx.simulate_click(item.center(), Default::default());
        cx.run_until_parked();
    };
    select_and_menu(cx);
    let panel = app.read_with(cx, |app, _| app.browser_panel.clone());
    panel.read_with(cx, |panel, _| {
        assert!(panel.browser_active());
        assert_eq!(panel.tabs.entries.len(), 1);
    });
    app.read_with(cx, |app, _| assert!(app.inspector_open));
    assert_eq!(
        cx.opened_url(),
        None,
        "explicit preview does not launch the system browser"
    );
    drain_sizes(cx, &requests);
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.server.update(cx, |input, cx| {
                input.set_value("https://relay.example.com", window, cx)
            })
        })
    });
    select_and_menu(cx);
    panel.read_with(cx, |panel, _| {
        assert_eq!(panel.tabs.entries.len(), 1);
        assert!(panel.error.as_ref().unwrap().contains("gateway"));
    });
    assert!(
        requests.is_empty(),
        "opening links must not synthesize terminal writes"
    );
}
