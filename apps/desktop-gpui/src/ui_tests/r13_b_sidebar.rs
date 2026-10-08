use super::*;

#[gpui_kit::test]
fn r13_b_priority_headers_left_align_and_toggle_membership(cx: &mut TestAppContext) {
    let (app, cx, _) = harness(cx);
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.sidebar_state.priority = true;
            cx.notify();
        })
    });
    cx.run_until_parked();
    for width in [208., 288., 480.] {
        cx.update(|_, cx| {
            app.update(cx, |app, cx| {
                app.sidebar_width = width;
                cx.notify();
            })
        });
        cx.run_until_parked();
        let header = cx.debug_bounds("priority-active").unwrap();
        let label = cx.debug_bounds("priority-active-label").unwrap();
        assert!(
            label.left() - header.left() < px(40.),
            "left aligned at {width}: {label:?} in {header:?}"
        );
    }
    let header = cx.debug_bounds("priority-active").unwrap();
    cx.simulate_click(header.center(), Default::default());
    cx.run_until_parked();
    assert!(cx.debug_bounds("thread-t").is_none());
    let header = cx.debug_bounds("priority-active").unwrap();
    cx.simulate_click(header.center(), Default::default());
    cx.run_until_parked();
    assert!(cx.debug_bounds("thread-t").is_some());
}

#[gpui_kit::test]
fn r13_b_row_isolation_reuses_unchanged_subtrees_and_invalidates_only_changed_row(
    cx: &mut TestAppContext,
) {
    let (app, cx, _) = harness(cx);
    seed_second_thread(&app, cx, false);
    for _ in 0..3 {
        cx.update(|_, cx| app.update(cx, |_, cx| cx.notify()));
        cx.run_until_parked();
    }
    let warmed = app.read_with(cx, |app, _| app.sidebar_rows.constructions());
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.composer.update(cx, |input, cx| {
                input.set_value("unrelated draft", window, cx)
            });
            cx.notify();
        })
    });
    cx.run_until_parked();
    assert_eq!(
        app.read_with(cx, |app, _| app.sidebar_rows.constructions()),
        warmed
    );
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.shell
                .threads
                .iter_mut()
                .find(|row| row.id == "u")
                .unwrap()
                .title = "Changed row".into();
            app.shell.revision += 1;
            cx.notify();
        })
    });
    cx.run_until_parked();
    assert_eq!(
        app.read_with(cx, |app, _| app.sidebar_rows.constructions()),
        warmed + 1
    );
    assert!(cx.debug_bounds("sidebar-title-u").is_some());
    let row = cx.debug_bounds("thread-u").unwrap();
    cx.simulate_click(row.center(), Default::default());
    cx.run_until_parked();
    assert_eq!(
        app.read_with(cx, |app, _| app.selected_thread.clone()),
        Some("u".into())
    );
}

fn select_pair(app: &Entity<NativeApp>, cx: &mut VisualTestContext) {
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
}

#[gpui_kit::test]
fn r13_b_keyless_move_materializes_receipt_order_and_retains_partial_failure(
    cx: &mut TestAppContext,
) {
    let (app, cx, requests) = harness(cx);
    seed_second_thread(&app, cx, false);
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.sidebar_state.capabilities = json!({"threadActiveReorder":true});
            app.shell.threads[0].active_order_key = None;
            app.shell.threads[1].active_order_key = Some("bad-key".into());
            app.shell.revision += 1;
            cx.notify();
        })
    });
    cx.run_until_parked();
    right_click(cx, "thread-u");
    let up = cx.debug_bounds("move-up").unwrap();
    cx.simulate_click(up.center(), Default::default());
    cx.run_until_parked();
    let connection::Command::Call {
        tag,
        payload,
        reply,
    } = requests.try_recv().unwrap()
    else {
        panic!("repair receipt")
    };
    assert_eq!(tag, "orchestration.dispatchCommand");
    assert_eq!(payload["type"], "thread.active.reorder");
    assert_eq!(payload["threadId"], "u");
    let key = payload["orderKey"].as_str().unwrap().to_owned();
    assert!(sidebar::key_between(Some(&key), None).is_some());
    assert!(
        requests.try_recv().is_err(),
        "next write must await receipt"
    );
    reply.send_blocking(Ok(json!({}))).unwrap();
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            let row = app.shell.threads.iter().find(|row| row.id == "u").unwrap();
            let mut value = serde_json::to_value(row).unwrap();
            value["activeOrderKey"] = json!(key);
            app.apply_shell_item(
                serde_json::from_value(
                    json!({"kind":"thread-upserted", "sequence":100, "thread":value}),
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
    reply.send_blocking(Err("neighbor denied".into())).unwrap();
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
    assert!(app.read_with(cx, |app, _| app.shell.threads[0].active_order_key.is_none()));
    assert!(requests.try_recv().is_err());
}

#[gpui_kit::test]
fn r13_b_bulk_pin_undo_only_accepted_targets_and_retry_remaining(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    select_pair(&app, cx);
    right_click(cx, "thread-u");
    let pin = cx.debug_bounds("bulk-pin").unwrap();
    cx.simulate_click(pin.center(), Default::default());
    cx.run_until_parked();
    let connection::Command::Call { payload, reply, .. } = requests.try_recv().unwrap() else {
        panic!("pin t")
    };
    assert_eq!(payload["threadId"], "t");
    reply.send_blocking(Ok(json!({}))).unwrap();
    cx.run_until_parked();
    let connection::Command::Call { reply, .. } = requests.try_recv().unwrap() else {
        panic!("pin u")
    };
    reply.send_blocking(Err("rejected".into())).unwrap();
    cx.run_until_parked();
    assert_eq!(
        app.read_with(cx, |app, _| app.sidebar_state.selection.clone()),
        vec!["u"]
    );
    let undo = cx.debug_bounds("undo-thread-navigation").unwrap();
    cx.simulate_click(undo.center(), Default::default());
    cx.run_until_parked();
    let connection::Command::Call { payload, reply, .. } = requests.try_recv().unwrap() else {
        panic!("undo t")
    };
    assert_eq!(payload["type"], "thread.unpin");
    assert_eq!(payload["threadId"], "t");
    reply.send_blocking(Err("retry me".into())).unwrap();
    cx.run_until_parked();
    assert!(app.read_with(cx, |app, _| app.thread_undo.record.is_some()));
    let undo = cx.debug_bounds("undo-thread-navigation").unwrap();
    cx.simulate_click(undo.center(), Default::default());
    cx.run_until_parked();
    let connection::Command::Call { payload, reply, .. } = requests.try_recv().unwrap() else {
        panic!("retry t")
    };
    assert_eq!(payload["threadId"], "t");
    reply.send_blocking(Ok(json!({}))).unwrap();
    cx.run_until_parked();
    assert!(app.read_with(cx, |app, _| app.thread_undo.record.is_none()));
    assert!(requests.try_recv().is_err());
}

#[gpui_kit::test]
fn r13_b_partial_batch_undo_counts_and_retries_only_remaining_target(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    select_pair(&app, cx);
    right_click(cx, "thread-u");
    let pin = cx.debug_bounds("bulk-pin").unwrap();
    cx.simulate_click(pin.center(), Default::default());
    cx.run_until_parked();
    for _ in 0..2 {
        let connection::Command::Call { reply, .. } = requests.try_recv().unwrap() else {
            panic!("pin")
        };
        reply.send_blocking(Ok(json!({}))).unwrap();
        cx.run_until_parked();
    }
    let undo = cx.debug_bounds("undo-thread-navigation").unwrap();
    cx.simulate_click(undo.center(), Default::default());
    cx.run_until_parked();
    let connection::Command::Call { payload, reply, .. } = requests.try_recv().unwrap() else {
        panic!("undo t")
    };
    assert_eq!(payload["threadId"], "t");
    reply.send_blocking(Ok(json!({}))).unwrap();
    cx.run_until_parked();
    let connection::Command::Call { payload, reply, .. } = requests.try_recv().unwrap() else {
        panic!("undo u")
    };
    assert_eq!(payload["threadId"], "u");
    reply.send_blocking(Err("retry u".into())).unwrap();
    cx.run_until_parked();
    assert_eq!(
        app.read_with(cx, |app, _| app
            .thread_undo
            .record
            .as_ref()
            .unwrap()
            .label()),
        "1 thread · Undo remaining"
    );
    let undo = cx.debug_bounds("undo-thread-navigation").unwrap();
    cx.simulate_click(undo.center(), Default::default());
    cx.run_until_parked();
    let connection::Command::Call { payload, reply, .. } = requests.try_recv().unwrap() else {
        panic!("retry u")
    };
    assert_eq!(payload["threadId"], "u");
    reply.send_blocking(Ok(json!({}))).unwrap();
    cx.run_until_parked();
    assert!(
        requests.try_recv().is_err(),
        "must not replay accepted inverse t"
    );
    assert!(app.read_with(cx, |app, _| app.thread_undo.record.is_none()));
}

#[gpui_kit::test]
fn r13_b_bulk_delete_requires_review_and_retains_failed_targets(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    select_pair(&app, cx);
    right_click(cx, "thread-u");
    let delete = cx.debug_bounds("bulk-delete").unwrap();
    cx.simulate_click(delete.center(), Default::default());
    cx.run_until_parked();
    assert!(requests.try_recv().is_err());
    let delete = cx.debug_bounds("bulk-delete").unwrap();
    cx.simulate_click(delete.center(), Default::default());
    cx.run_until_parked();
    let connection::Command::Call {
        tag,
        payload,
        reply,
    } = requests.try_recv().unwrap()
    else {
        panic!("delete t")
    };
    assert_eq!(tag, "orchestration.dispatchCommand");
    assert_eq!(payload["type"], "thread.delete");
    assert_eq!(payload["threadId"], "t");
    reply.send_blocking(Ok(json!({}))).unwrap();
    cx.run_until_parked();
    let connection::Command::Call { reply, .. } = requests.try_recv().unwrap() else {
        panic!("delete u")
    };
    reply.send_blocking(Err("delete denied".into())).unwrap();
    cx.run_until_parked();
    assert_eq!(
        app.read_with(cx, |app, _| app.sidebar_state.selection.clone()),
        vec!["u"]
    );
    assert!(
        app.read_with(cx, |app, _| app.thread_undo.record.is_none()),
        "delete must not fabricate undo"
    );
}

#[gpui_kit::test]
fn r13_b_bulk_title_transport_and_mixed_snooze_applicability(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    select_pair(&app, cx);
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.sidebar_state.capabilities =
                json!({"threadTitleRegeneration":true,"threadSnooze":true});
            app.shell.threads[0].has_pending_approvals = true;
            app.shell.revision += 1;
            cx.notify();
        })
    });
    cx.run_until_parked();
    right_click(cx, "thread-u");
    assert!(
        cx.debug_bounds("bulk-snooze-preset-0").is_none(),
        "mixed snooze must not half-apply"
    );
    let title = cx.debug_bounds("bulk-regenerate-title").unwrap();
    cx.simulate_click(title.center(), Default::default());
    cx.run_until_parked();
    for id in ["t", "u"] {
        let connection::Command::Call {
            tag,
            payload,
            reply,
        } = requests.try_recv().unwrap()
        else {
            panic!("title")
        };
        assert_eq!(tag, "orchestration.dispatchCommand");
        assert_eq!(payload["type"], "thread.meta.update");
        assert_eq!(payload["regenerateTitle"], true);
        assert_eq!(payload["threadId"], id);
        reply.send_blocking(Ok(json!({}))).unwrap();
        cx.run_until_parked();
    }
    assert!(app.read_with(cx, |app, _| app.sidebar_state.selection.is_empty()));
}

#[gpui_kit::test]
fn r13_b_bulk_snooze_preset_captures_one_deadline_for_every_receipt(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    select_pair(&app, cx);
    right_click(cx, "thread-u");
    let preset = cx.debug_bounds("bulk-snooze-preset-1").unwrap();
    cx.simulate_click(preset.center(), Default::default());
    cx.run_until_parked();
    let connection::Command::Call {
        tag,
        payload,
        reply,
    } = requests.try_recv().unwrap()
    else {
        panic!("snooze t")
    };
    assert_eq!(tag, "orchestration.dispatchCommand");
    assert_eq!(payload["type"], "thread.snooze");
    let until = payload["snoozedUntil"].clone();
    let deadline = chrono::DateTime::parse_from_rfc3339(until.as_str().unwrap()).unwrap();
    assert!(deadline > chrono::Utc::now() + chrono::Duration::hours(2));
    reply.send_blocking(Ok(json!({}))).unwrap();
    cx.run_until_parked();
    let connection::Command::Call { payload, reply, .. } = requests.try_recv().unwrap() else {
        panic!("snooze u")
    };
    assert_eq!(payload["threadId"], "u");
    assert_eq!(payload["snoozedUntil"], until);
    reply.send_blocking(Ok(json!({}))).unwrap();
    cx.run_until_parked();
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
}

#[gpui_kit::test]
fn r13_b_custom_bulk_snooze_rejects_changed_review_targets(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    select_pair(&app, cx);
    right_click(cx, "thread-u");
    let custom = cx.debug_bounds("bulk-snooze-custom").unwrap();
    cx.simulate_click(custom.center(), Default::default());
    cx.run_until_parked();
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.custom_snooze.update(cx, |input, cx| {
                input.set_value(
                    (chrono::Utc::now() + chrono::Duration::hours(4)).to_rfc3339(),
                    window,
                    cx,
                )
            });
            app.sidebar_state.selection = vec!["u".into()];
            cx.notify();
        })
    });
    cx.run_until_parked();
    let submit = cx.debug_bounds("row-custom-snooze").unwrap();
    cx.simulate_click(submit.center(), Default::default());
    cx.run_until_parked();
    assert!(requests.try_recv().is_err());
    assert!(app.read_with(cx, |app, _| {
        app.sidebar_state.custom_snooze_target.is_none()
    }));
}

#[gpui_kit::test]
fn r13_b_fresh_error_wire_wakes_and_dismissal_preserves_unread(cx: &mut TestAppContext) {
    let (app, cx, _) = harness(cx);
    seed_second_thread(&app, cx, false);
    let now = chrono::Utc::now();
    let install = |at: chrono::DateTime<chrono::Utc>,
                   sequence,
                   app: &Entity<NativeApp>,
                   cx: &mut VisualTestContext| {
        cx.update(|_, cx| app.update(cx, |app, cx| {
            app.sidebar_state.priority = true;
            app.apply_shell_item(serde_json::from_value(json!({"kind":"thread-upserted","sequence":sequence,"thread": {
                "id":"u","projectId":"p","title":"Other", "settledOverride":"settled",
                "snoozedAt": (now - chrono::Duration::hours(1)).to_rfc3339(),
                "snoozedUntil": (now + chrono::Duration::hours(3)).to_rfc3339(),
                "session":{"status":"error","updatedAt":at.to_rfc3339(),"lastError":"failed"}
            }})).unwrap(), cx).unwrap();
            app.sidebar_state.marked_unread.insert((app.environment_scope.clone(), "u".into()));
            cx.notify();
        }));
        cx.run_until_parked();
    };
    install(now - chrono::Duration::hours(2), 100, &app, cx);
    assert!(cx.debug_bounds("dismiss-woke-u").is_none());
    install(now - chrono::Duration::minutes(1), 101, &app, cx);
    let dismiss = cx.debug_bounds("dismiss-woke-u").unwrap();
    cx.simulate_click(dismiss.center(), Default::default());
    cx.run_until_parked();
    assert!(cx.debug_bounds("dismiss-woke-u").is_none());
    assert!(app.read_with(cx, |app, _| {
        app.sidebar_state
            .marked_unread
            .contains(&(app.environment_scope.clone(), "u".into()))
    }));
    assert_eq!(
        app.read_with(cx, |app, _| app.selected_thread.clone()),
        Some("t".into())
    );
}
