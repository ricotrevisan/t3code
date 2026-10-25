use super::*;

fn single_review(cx: &mut VisualTestContext) {
    right_click(cx, "thread-u");
    let delete = cx.debug_bounds("delete").unwrap();
    cx.simulate_click(delete.center(), Default::default());
    cx.run_until_parked();
    assert!(cx.debug_bounds("bulk-delete-review").is_some());
}

#[gpui_kit::test]
fn r18_b_single_review_cancel_reopen_and_denial_preserves_selected_thread(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    seed_second_thread(&app, cx, false);
    single_review(cx);
    assert!(requests.try_recv().is_err());
    let cancel = cx.debug_bounds("bulk-delete-cancel").unwrap();
    cx.simulate_click(cancel.center(), Default::default());
    cx.run_until_parked();
    assert!(cx.debug_bounds("bulk-delete-review").is_none());
    assert!(requests.try_recv().is_err());
    single_review(cx);
    let confirm = cx.debug_bounds("bulk-delete").unwrap();
    cx.simulate_click(confirm.center(), Default::default());
    cx.run_until_parked();
    let connection::Command::Call {
        tag,
        payload,
        reply,
    } = requests.try_recv().unwrap()
    else {
        panic!("delete receipt")
    };
    assert_eq!(tag, "orchestration.dispatchCommand");
    assert_eq!(payload["type"], "thread.delete");
    assert_eq!(payload["threadId"], "u");
    reply.send_blocking(Err("denied".into())).unwrap();
    cx.run_until_parked();
    assert_eq!(
        app.read_with(cx, |app, _| app.selected_thread.clone()),
        Some("t".into())
    );
    assert!(app.read_with(cx, |app, _| !app.sidebar_state.bulk_busy));
    assert!(cx.debug_bounds("thread-u").is_some());
    assert!(requests.try_recv().is_err());
}

#[gpui_kit::test]
fn r18_b_single_review_acceptance_does_not_delete_selection_or_replay(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    seed_second_thread(&app, cx, false);
    single_review(cx);
    let confirm = cx.debug_bounds("bulk-delete").unwrap();
    cx.simulate_click(confirm.center(), Default::default());
    cx.run_until_parked();
    let connection::Command::Call { payload, reply, .. } = requests.try_recv().unwrap() else {
        panic!("delete receipt")
    };
    assert_eq!(payload["threadId"], "u");
    reply.send_blocking(Ok(json!({}))).unwrap();
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.apply_shell_item(
                serde_json::from_value(json!({
                    "kind":"thread-removed", "sequence":101, "threadId":"u"
                }))
                .unwrap(),
                cx,
            )
            .unwrap();
            cx.notify();
        })
    });
    cx.run_until_parked();
    assert_eq!(
        app.read_with(cx, |app, _| app.selected_thread.clone()),
        Some("t".into())
    );
    assert!(cx.debug_bounds("thread-u").is_none());
    assert!(app.read_with(cx, |app, _| app.thread_undo.record.is_none()));
    assert!(requests.try_recv().is_err());
}

#[gpui_kit::test]
fn r18_b_single_review_retarget_rejects_old_confirmation(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    seed_second_thread(&app, cx, false);
    single_review(cx);
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.thread_menu_row = Some(("t".into(), Default::default()));
            app.sidebar_menu_action("", "single-delete-confirm", window, cx);
        })
    });
    cx.run_until_parked();
    assert!(requests.try_recv().is_err());
    assert!(app.read_with(cx, |app, _| {
        app.sidebar_state.single_delete_review.is_none()
    }));
    assert!(cx.debug_bounds("thread-u").is_some());
}
