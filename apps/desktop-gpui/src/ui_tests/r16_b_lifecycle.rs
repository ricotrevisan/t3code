use super::*;

#[gpui_kit::test]
fn r16_b_keyless_physical_drop_preserves_accepted_repair_when_neighbor_denied(
    cx: &mut TestAppContext,
) {
    let (app, cx, requests) = harness(cx);
    seed_second_thread(&app, cx, false);
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.sidebar_state.priority = true;
            app.sidebar_state.capabilities = json!({"threadActiveReorder":true});
            for row in &mut app.shell.threads {
                row.active_order_key = None;
            }
            app.shell.revision += 1;
            cx.notify();
        })
    });
    cx.run_until_parked();
    let from = cx.debug_bounds("thread-u").unwrap().center();
    let to = cx.debug_bounds("thread-t").unwrap().center();
    drag(cx, from, to);
    let connection::Command::Call {
        tag,
        payload,
        reply,
    } = requests.try_recv().unwrap()
    else {
        panic!("drop receipt")
    };
    assert_eq!(tag, "orchestration.dispatchCommand");
    assert_eq!(payload["type"], "thread.active.reorder");
    assert_eq!(payload["threadId"], "u");
    let key = payload["orderKey"].as_str().unwrap().to_owned();
    assert!(requests.try_recv().is_err());
    reply.send_blocking(Ok(json!({}))).unwrap();
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            let mut row =
                serde_json::to_value(app.shell.threads.iter().find(|row| row.id == "u").unwrap())
                    .unwrap();
            row["activeOrderKey"] = json!(key);
            app.apply_shell_item(
                serde_json::from_value(
                    json!({"kind":"thread-upserted", "sequence":100, "thread":row}),
                )
                .unwrap(),
                cx,
            )
            .unwrap();
            cx.notify();
        })
    });
    cx.run_until_parked();
    let connection::Command::Call { payload, reply, .. } = requests.try_recv().unwrap() else {
        panic!("neighbor repair")
    };
    assert_eq!(payload["threadId"], "t");
    assert!(payload["orderKey"].as_str().unwrap() > key.as_str());
    reply.send_blocking(Err("repair denied".into())).unwrap();
    cx.run_until_parked();
    assert!(app.read_with(cx, |app, _| !app.sidebar_state.order_busy));
    assert_eq!(
        app.read_with(cx, |app, _| app
            .shell
            .threads
            .iter()
            .find(|row| row.id == "u")
            .unwrap()
            .active_order_key
            .clone()),
        Some(key)
    );
    assert!(app.read_with(cx, |app, _| {
        app.shell
            .threads
            .iter()
            .find(|row| row.id == "t")
            .unwrap()
            .active_order_key
            .is_none()
    }));
    assert!(requests.try_recv().is_err());
}

#[gpui_kit::test]
fn r16_b_delete_all_accepted_clears_detail_and_transport_selection(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    review(&app, cx);
    let confirm = cx.debug_bounds("bulk-delete").unwrap();
    cx.simulate_click(confirm.center(), Default::default());
    cx.run_until_parked();
    for id in ["t", "u"] {
        let connection::Command::Call { payload, reply, .. } = requests.try_recv().unwrap() else {
            panic!("delete")
        };
        assert_eq!(payload["threadId"], id);
        reply.send_blocking(Ok(json!({}))).unwrap();
        cx.run_until_parked();
    }
    assert!(app.read_with(cx, |app, _| app.selected_thread.is_none()
        && app.detail.thread.is_none()));
    assert!(app.read_with(cx, |app, _| app.sidebar_state.selection.is_empty()));
    assert!(app.read_with(cx, |app, _| !app.pending_history_attached));
    assert!(app.read_with(cx, |app, _| app.thread_undo.record.is_none()));
}

#[gpui_kit::test]
fn r16_b_bulk_settle_skips_co_departing_targets_and_retains_counted_undo(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    seed_second_thread(&app, cx, false);
    let row = cx.debug_bounds("thread-u").unwrap();
    cx.simulate_click(
        row.center(),
        gpui_kit::Modifiers {
            platform: true,
            ..Default::default()
        },
    );
    cx.run_until_parked();
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.sidebar_state.priority = true;
            let mut row = app
                .shell
                .threads
                .iter()
                .find(|row| row.id == "u")
                .unwrap()
                .clone();
            row.id = "v".into();
            row.title = "Survivor".into();
            app.apply_shell_item(
                serde_json::from_value(
                    json!({"kind":"thread-upserted", "sequence":99, "thread":row}),
                )
                .unwrap(),
                cx,
            )
            .unwrap();
            cx.notify();
        })
    });
    cx.run_until_parked();
    right_click(cx, "thread-u");
    let settle = cx.debug_bounds("bulk-settle").unwrap();
    cx.simulate_click(settle.center(), Default::default());
    cx.run_until_parked();
    for (sequence, id) in [(100, "t"), (101, "u")] {
        let connection::Command::Call {
            tag,
            payload,
            reply,
        } = requests.try_recv().unwrap()
        else {
            panic!("settle receipt")
        };
        assert_eq!(tag, "orchestration.dispatchCommand");
        assert_eq!(payload["type"], "thread.settle");
        assert_eq!(payload["threadId"], id);
        reply.send_blocking(Ok(json!({}))).unwrap();
        cx.update(|_, cx| {
            app.update(cx, |app, cx| {
                let mut row = serde_json::to_value(
                    app.shell.threads.iter().find(|row| row.id == id).unwrap(),
                )
                .unwrap();
                row["settledOverride"] = json!("settled");
                app.apply_shell_item(
                    serde_json::from_value(
                        json!({"kind":"thread-upserted", "sequence":sequence, "thread":row}),
                    )
                    .unwrap(),
                    cx,
                )
                .unwrap();
                cx.notify();
            })
        });
        cx.run_until_parked();
    }
    assert_eq!(
        app.read_with(cx, |app, _| app.selected_thread.clone()),
        Some("v".into())
    );
    assert_eq!(
        app.read_with(cx, |app, _| app
            .thread_undo
            .record
            .as_ref()
            .unwrap()
            .title
            .clone()),
        "2 threads"
    );
    assert!(cx.debug_bounds("thread-v").is_some());
}

fn review(app: &Entity<NativeApp>, cx: &mut VisualTestContext) {
    seed_second_thread(app, cx, false);
    let row = cx.debug_bounds("thread-u").unwrap();
    cx.simulate_click(
        row.center(),
        gpui_kit::Modifiers {
            platform: true,
            ..Default::default()
        },
    );
    cx.run_until_parked();
    right_click(cx, "thread-u");
    let delete = cx.debug_bounds("bulk-delete").unwrap();
    cx.simulate_click(delete.center(), Default::default());
    cx.run_until_parked();
    assert!(cx.debug_bounds("bulk-delete-review").is_some());
}

#[gpui_kit::test]
fn r16_b_delete_review_cancel_sends_nothing_and_keeps_selection(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    review(&app, cx);
    assert!(requests.try_recv().is_err());
    let cancel = cx.debug_bounds("bulk-delete-cancel").unwrap();
    cx.simulate_click(cancel.center(), Default::default());
    cx.run_until_parked();
    assert!(cx.debug_bounds("bulk-delete-review").is_none());
    assert!(requests.try_recv().is_err());
    assert_eq!(
        app.read_with(cx, |app, _| app.sidebar_state.selection.clone()),
        vec!["t", "u"]
    );
}

#[gpui_kit::test]
fn r16_b_delete_review_changed_targets_rejects_confirmation(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    review(&app, cx);
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.sidebar_state.selection = vec!["u".into()];
            cx.notify();
        })
    });
    cx.run_until_parked();
    let confirm = cx.debug_bounds("bulk-delete").unwrap();
    cx.simulate_click(confirm.center(), Default::default());
    cx.run_until_parked();
    assert!(requests.try_recv().is_err());
    assert!(app.read_with(cx, |app, _| app.sidebar_state.bulk_delete_review.is_none()));
    assert_eq!(
        app.read_with(cx, |app, _| app.selected_thread.clone()),
        Some("t".into())
    );
}

#[gpui_kit::test]
fn r16_b_delete_receipt_failure_moves_selected_to_surviving_target(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    review(&app, cx);
    let confirm = cx.debug_bounds("bulk-delete").unwrap();
    cx.simulate_click(confirm.center(), Default::default());
    cx.run_until_parked();
    for (id, result) in [("t", Ok(json!({}))), ("u", Err("denied".into()))] {
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
        assert_eq!(payload["threadId"], id);
        assert_eq!(
            app.read_with(cx, |app, _| app.selected_thread.clone()),
            Some("t".into())
        );
        reply.send_blocking(result).unwrap();
        if id == "t" {
            cx.update(|_, cx| {
                app.update(cx, |app, cx| {
                    app.apply_shell_item(
                        serde_json::from_value(
                            json!({"kind":"thread-removed", "sequence":101, "threadId":"t"}),
                        )
                        .unwrap(),
                        cx,
                    )
                    .unwrap();
                    cx.notify();
                })
            });
        }
        cx.run_until_parked();
    }
    assert_eq!(
        app.read_with(cx, |app, _| app.selected_thread.clone()),
        Some("u".into())
    );
    assert_eq!(
        app.read_with(cx, |app, _| app.sidebar_state.selection.clone()),
        vec!["u"]
    );
    assert!(app.read_with(cx, |app, _| app.thread_undo.record.is_none()));
    assert!(cx.debug_bounds("thread-u").is_some());
}
