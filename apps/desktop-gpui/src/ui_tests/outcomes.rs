use super::*;

#[gpui_kit::test]
fn outcomes_failure_without_error_words_persists_until_closed(cx: &mut TestAppContext) {
    let (app, cx, _) = harness(cx);
    cx.update(|_, cx| app.update(cx, |app, cx| app.report_error("Permission denied", cx)));
    cx.run_until_parked();
    assert_eq!(toast_messages(&app, cx), ["Permission denied"]);
    assert_eq!(
        app.read_with(cx, |app, _| app.toasts[0].kind),
        notifications::ToastKind::Error
    );
    cx.background_executor
        .advance_clock(notifications::TOAST_LIFETIME * 2);
    cx.run_until_parked();
    assert_eq!(toast_messages(&app, cx), ["Permission denied"]);
    let close = cx.debug_bounds("toast-close-1").unwrap();
    cx.simulate_click(close.center(), Default::default());
    cx.run_until_parked();
    assert!(toast_messages(&app, cx).is_empty());
}

#[gpui_kit::test]
fn outcomes_success_named_error_is_success_and_expires(cx: &mut TestAppContext) {
    let (app, cx, _) = harness(cx);
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.report_success("Branch error-handler created", cx)
        })
    });
    cx.run_until_parked();
    app.read_with(cx, |app, _| {
        assert_eq!(app.status_kind, notifications::Outcome::Success);
        assert_eq!(app.toasts.len(), 1);
        assert_eq!(app.toasts[0].kind, notifications::ToastKind::Success);
    });
    cx.background_executor
        .advance_clock(notifications::TOAST_LIFETIME);
    cx.run_until_parked();
    assert!(toast_messages(&app, cx).is_empty());
}

#[gpui_kit::test]
fn outcomes_plain_status_and_progress_are_neutral_and_never_raise_toasts(cx: &mut TestAppContext) {
    let (app, cx, _) = harness(cx);
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.set_status("error-handling documentation loaded", cx);
            assert_eq!(app.status_kind, notifications::Outcome::Info);
            app.report_progress("Refreshing error-handler", cx);
            assert_eq!(app.status_kind, notifications::Outcome::Progress);
        })
    });
    assert!(app.read_with(cx, |app, _| app.toasts.is_empty()));
}

#[gpui_kit::test]
fn outcomes_transient_stack_limit_never_discards_unclosed_errors(cx: &mut TestAppContext) {
    let (app, cx, _) = harness(cx);
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            for i in 0..notifications::MAX_TOASTS + 1 {
                app.report_error(format!("Denied request {i}"), cx);
            }
            app.report_success("Finished", cx);
        })
    });
    cx.run_until_parked();
    cx.background_executor
        .advance_clock(notifications::TOAST_LIFETIME * 2);
    cx.run_until_parked();
    assert_eq!(
        app.read_with(cx, |app, _| app.toasts.len()),
        notifications::MAX_TOASTS + 1
    );
    let close = cx
        .debug_bounds("toast-close-1")
        .expect("retained errors remain dismissible");
    cx.simulate_click(close.center(), Default::default());
    cx.run_until_parked();
    assert_eq!(
        app.read_with(cx, |app, _| app.toasts.len()),
        notifications::MAX_TOASTS
    );
}
