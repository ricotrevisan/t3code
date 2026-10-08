//! Pointer activation follows streamed terminal cells, never executable URL schemes.
use super::surfaces_terminal::{drain_sizes, open};
use super::*;

#[gpui_kit::test]
fn r14_t_modifier_click_opens_streamed_url_and_plain_click_only_selects(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    let (id, stream) = open(&app, cx, &requests);
    cx.update(|_, cx| app.update(cx, |app, cx| app.terminal_update(&stream, Ok(json!({"type":"output","threadId":"t","terminalId":id,"sequence":1,"data":"\r\nhttps://example.com/docs\r\n\x1b]8;;javascript:alert(1)\x1b\\unsafe\x1b]8;;\x1b\\"})), cx)));
    drain_sizes(cx, &requests);
    let grid = cx.debug_bounds("terminal-grid").unwrap();
    let (width, height) = app.read_with(cx, |app, _| {
        (
            app.terminal_drawer.cell_width,
            app.terminal_drawer.line_height,
        )
    });
    let url = point(
        grid.origin.x + px(width * 5.),
        grid.origin.y + px(height + 8.),
    );
    cx.simulate_click(url, Default::default());
    cx.run_until_parked();
    assert_eq!(cx.opened_url(), None);
    cx.simulate_click(
        url,
        gpui_kit::Modifiers {
            platform: true,
            ..Default::default()
        },
    );
    cx.run_until_parked();
    assert_eq!(cx.opened_url().as_deref(), Some("https://example.com/docs"));
    let unsafe_link = point(url.x, grid.origin.y + px(height * 2. + 8.));
    cx.simulate_click(
        unsafe_link,
        gpui_kit::Modifiers {
            platform: true,
            ..Default::default()
        },
    );
    cx.run_until_parked();
    assert_eq!(
        cx.opened_url().as_deref(),
        Some("https://example.com/docs"),
        "unsafe OSC-8 must not launch another URL"
    );
    assert!(
        requests.is_empty(),
        "link activation must never send shell input"
    );
}
