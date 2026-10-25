//! Platform-callback leases exercised before the deferred blur observer runs.
use super::surfaces_terminal::{call, open};
use super::*;
use gpui_kit::EntityInputHandler;

#[gpui_kit::test]
fn r18_t_blur_immediately_retires_native_read_and_candidate_geometry(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    open(&app, cx, &requests);
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.replace_and_mark_text_in_range(None, "日本😀", Some(2..4), window, cx);
            let bounds = app.terminal_drawer.bounds.unwrap();
            assert!(app.bounds_for_range(2..4, bounds, window, cx).is_some());
            // Do not park: native getters may run synchronously during a responder
            // transition, before on_focus_out's deferred entity update.
            app.composer.update(cx, |input, cx| input.focus(window, cx));
            assert!(!app.accepts_text_input(window, cx));
            let mut adjusted = None;
            assert_eq!(app.text_for_range(0..4, &mut adjusted, window, cx), None);
            assert!(app.selected_text_range(false, window, cx).is_none());
            assert!(app.selected_text_range(true, window, cx).is_none());
            assert_eq!(app.marked_text_range(window, cx), None);
            assert_eq!(app.bounds_for_range(2..4, bounds, window, cx), None);
            assert_eq!(
                app.character_index_for_point(bounds.origin, window, cx),
                None
            );
            app.replace_text_in_range(None, "late commit", window, cx);
            app.unmark_text(window, cx);
        });
    });
    cx.run_until_parked();
    assert!(
        requests.is_empty(),
        "blurred callbacks must never write to the PTY"
    );
}

#[gpui_kit::test]
fn r18_t_utf16_preedit_replacement_cancel_and_commit_are_single_owned_write(
    cx: &mut TestAppContext,
) {
    let (app, cx, requests) = harness(cx);
    open(&app, cx, &requests);
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.replace_and_mark_text_in_range(None, "に😀本", Some(1..3), window, cx);
            let mut adjusted = None;
            assert_eq!(
                app.text_for_range(1..3, &mut adjusted, window, cx)
                    .as_deref(),
                Some("😀")
            );
            assert_eq!(adjusted, Some(1..3));
            app.replace_and_mark_text_in_range(Some(1..3), "語", Some(0..1), window, cx);
            assert_eq!(
                app.selected_text_range(false, window, cx).unwrap().range,
                1..2
            );
            assert_eq!(
                app.text_for_range(0..3, &mut adjusted, window, cx)
                    .as_deref(),
                Some("に語本")
            );
        });
    });
    assert!(requests.is_empty(), "preedit is local, not shell input");
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.replace_and_mark_text_in_range(None, "", None, window, cx);
            app.unmark_text(window, cx);
            app.replace_and_mark_text_in_range(None, "候補", None, window, cx);
            app.replace_text_in_range(Some(0..2), "日本é€😀", window, cx);
            app.unmark_text(window, cx);
        });
    });
    let (payload, reply) = call(&requests, "terminal.write");
    assert_eq!(payload["threadId"], "t");
    assert_eq!(payload["data"], "日本é€😀");
    reply
        .send_blocking(Err("PTY refused input".into()))
        .unwrap();
    cx.run_until_parked();
    app.read_with(cx, |app, _| {
        assert_eq!(
            app.terminal_drawer.tabs[0].error.as_deref(),
            Some("PTY refused input")
        );
    });
    assert!(
        requests.is_empty(),
        "commit/unmark and failure never duplicate or replay input"
    );
}
