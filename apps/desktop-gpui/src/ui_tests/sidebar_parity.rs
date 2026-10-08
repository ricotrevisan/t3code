use super::*;

#[gpui_kit::test]
fn sidebar_parity_draft_row_discard_is_local_and_preserves_selection(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.pending_threads.push(serde_json::from_value(json!({
            "thread":{"id":"draft","projectId":"p","title":"Unsent"},
            "created_at":"2026-04-01T00:00:00Z","prepare_worktree":null,"run_setup_script":false
        })).unwrap());
            cx.notify();
        })
    });
    cx.run_until_parked();
    right_click(cx, "draft-draft");
    assert!(cx.debug_bounds("rename").is_some());
    for id in [
        "pin", "unpin", "settle", "unsettle", "snooze", "archive", "delete",
    ] {
        assert!(cx.debug_bounds(id).is_none(), "invalid draft action {id}");
    }
    let discard = cx.debug_bounds("discard").unwrap();
    cx.simulate_click(discard.center(), Default::default());
    cx.run_until_parked();
    assert_eq!(app.read_with(cx, |app, _| app.pending_threads.len()), 1);
    let discard = cx.debug_bounds("discard").unwrap();
    cx.simulate_click(discard.center(), Default::default());
    cx.run_until_parked();
    assert!(app.read_with(cx, |app, _| app.pending_threads.is_empty()));
    assert_eq!(
        app.read_with(cx, |app, _| app.selected_thread.clone()),
        Some("t".into())
    );
    assert!(requests.try_recv().is_err());
}

#[gpui_kit::test]
fn sidebar_parity_mark_unread_and_copy_keep_row_target(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    seed_second_thread(&app, cx, false);
    right_click(cx, "thread-u");
    let mark = cx.debug_bounds("mark-unread").unwrap();
    cx.simulate_click(mark.center(), Default::default());
    cx.run_until_parked();
    assert!(app.read_with(cx, |app, _| {
        app.sidebar_state.unread(
            &app.environment_scope,
            app.shell
                .threads
                .iter()
                .find(|thread| thread.id == "u")
                .unwrap(),
        )
    }));
    right_click(cx, "thread-u");
    let copy = cx.debug_bounds("copy-thread-id").unwrap();
    cx.simulate_click(copy.center(), Default::default());
    cx.run_until_parked();
    assert_eq!(
        cx.update(|_, cx| cx.read_from_clipboard().unwrap().text().unwrap()),
        "u"
    );
    assert_eq!(
        app.read_with(cx, |app, _| app.selected_thread.clone()),
        Some("t".into())
    );
    assert!(requests.try_recv().is_err());
}

#[gpui_kit::test]
fn sidebar_parity_snooze_presets_capture_row_not_selection(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    seed_second_thread(&app, cx, false);
    right_click(cx, "thread-u");
    // Presets are below the core commands, scroll-free in the test window.
    let preset = cx.debug_bounds("snooze-preset-0").unwrap();
    cx.simulate_click(preset.center(), Default::default());
    cx.run_until_parked();
    let connection::Command::Call { payload, .. } = requests.try_recv().unwrap() else {
        panic!("snooze receipt");
    };
    assert_eq!(payload["type"], "thread.snooze");
    assert_eq!(payload["threadId"], "u");
    let until =
        chrono::DateTime::parse_from_rfc3339(payload["snoozedUntil"].as_str().unwrap()).unwrap();
    assert!(until > chrono::Utc::now());
    assert_eq!(
        app.read_with(cx, |app, _| app.selected_thread.clone()),
        Some("t".into())
    );
}

#[gpui_kit::test]
fn sidebar_parity_modifier_selection_and_bulk_failure_retention(cx: &mut TestAppContext) {
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
    assert_eq!(
        app.read_with(cx, |app, _| app.sidebar_state.selection.clone()),
        vec!["t", "u"]
    );
    assert_eq!(
        app.read_with(cx, |app, _| app.selected_thread.clone()),
        Some("t".into())
    );
    right_click(cx, "thread-u");
    let pin = cx.debug_bounds("bulk-pin").unwrap();
    cx.simulate_click(pin.center(), Default::default());
    cx.run_until_parked();
    let connection::Command::Call { payload, reply, .. } = requests.try_recv().unwrap() else {
        panic!("first bulk item");
    };
    assert_eq!(payload["threadId"], "t");
    reply.send_blocking(Ok(json!({}))).unwrap();
    cx.run_until_parked();
    let connection::Command::Call { payload, reply, .. } = requests.try_recv().unwrap() else {
        panic!("second bulk item");
    };
    assert_eq!(payload["threadId"], "u");
    reply.send_blocking(Err("pin denied".into())).unwrap();
    cx.run_until_parked();
    assert_eq!(
        app.read_with(cx, |app, _| app.sidebar_state.selection.clone()),
        vec!["u"]
    );
    assert!(!app.read_with(cx, |app, _| app.sidebar_state.bulk_busy));
    assert_eq!(
        app.read_with(cx, |app, _| app.selected_thread.clone()),
        Some("t".into())
    );
}

#[gpui_kit::test]
fn sidebar_parity_shift_click_selects_visible_range(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    seed_second_thread(&app, cx, false);
    let row = cx.debug_bounds("thread-u").unwrap();
    cx.simulate_click(
        row.center(),
        gpui_kit::Modifiers {
            shift: true,
            ..Default::default()
        },
    );
    cx.run_until_parked();
    let selected = app.read_with(cx, |app, _| app.sidebar_state.selection.clone());
    assert_eq!(selected.len(), 2);
    assert!(selected.contains(&"t".into()) && selected.contains(&"u".into()));
    assert!(requests.try_recv().is_err());
}

#[gpui_kit::test]
fn sidebar_parity_rejected_move_keeps_authoritative_order(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    seed_second_thread(&app, cx, false);
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.sidebar_state.capabilities = json!({"threadActiveReorder":true});
            app.shell
                .threads
                .iter_mut()
                .find(|thread| thread.id == "t")
                .unwrap()
                .active_order_key = Some("b".into());
            app.shell
                .threads
                .iter_mut()
                .find(|thread| thread.id == "u")
                .unwrap()
                .active_order_key = Some("z".into());
            app.shell.sequence = Some(app.shell.sequence.unwrap_or(0) + 1);
            app.shell.revision += 1;
            cx.notify();
        })
    });
    cx.run_until_parked();
    right_click(cx, "thread-u");
    let up = cx.debug_bounds("move-up").unwrap();
    cx.simulate_click(up.center(), Default::default());
    cx.run_until_parked();
    let connection::Command::Call { payload, reply, .. } = requests.try_recv().unwrap() else {
        panic!("reorder");
    };
    assert_eq!(payload["type"], "thread.active.reorder");
    assert_eq!(payload["threadId"], "u");
    assert!(payload["orderKey"].as_str().unwrap() < "b");
    reply.send_blocking(Err("ordering denied".into())).unwrap();
    cx.run_until_parked();
    assert!(app.read_with(cx, |app, _| !app.sidebar_state.order_busy));
    assert_eq!(
        app.read_with(cx, |app, _| app
            .shell
            .threads
            .iter()
            .find(|thread| thread.id == "u")
            .unwrap()
            .active_order_key
            .clone()),
        Some("z".into())
    );
    assert_eq!(
        app.read_with(cx, |app, _| app.selected_thread.clone()),
        Some("t".into())
    );
}

#[gpui_kit::test]
fn sidebar_parity_search_ignores_earlier_reply_and_cancels(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    let submit = |query: &str, app: &Entity<NativeApp>, cx: &mut VisualTestContext| {
        cx.update(|window, cx| {
            app.update(cx, |app, cx| {
                app.search
                    .update(cx, |input, cx| input.set_value(query, window, cx));
                app.search_threads(cx);
            })
        });
        cx.run_until_parked();
    };
    submit("earlier", &app, cx);
    let connection::Command::Call { reply: earlier, .. } = requests.try_recv().unwrap() else {
        panic!("first search");
    };
    submit("current", &app, cx);
    let connection::Command::Call { reply: current, .. } = requests.try_recv().unwrap() else {
        panic!("second search");
    };
    current.send_blocking(Ok(json!({"matches":[{"threadId":"t","projectId":"p","source":"assistant","snippet":"current answer","messageCreatedAt":"2026-04-01T01:00:00Z"}]}))).unwrap();
    cx.run_until_parked();
    earlier.send_blocking(Ok(json!({"matches":[]}))).unwrap();
    cx.run_until_parked();
    assert_eq!(app.read_with(cx, |app, _| app.search_results.len()), 1);
    let clear = cx.debug_bounds("clear-search").unwrap();
    cx.simulate_click(clear.center(), Default::default());
    cx.run_until_parked();
    assert!(app.read_with(cx, |app, _| app.search_results.is_empty()
        && !app.sidebar_state.search_loading));
}

#[gpui_kit::test]
fn sidebar_parity_row_rename_failure_retains_edit_without_selecting(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    seed_second_thread(&app, cx, false);
    right_click(cx, "thread-u");
    let rename = cx.debug_bounds("rename").unwrap();
    cx.simulate_click(rename.center(), Default::default());
    cx.run_until_parked();
    assert_eq!(
        app.read_with(cx, |app, _| app.selected_thread.clone()),
        Some("t".into())
    );
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.sidebar_rename
                .update(cx, |input, cx| input.set_value("New row title", window, cx))
        })
    });
    let save = cx.debug_bounds("row-save-rename").unwrap();
    cx.simulate_click(save.center(), Default::default());
    cx.run_until_parked();
    let connection::Command::Call { payload, reply, .. } = requests.try_recv().unwrap() else {
        panic!("row rename");
    };
    assert_eq!(payload["threadId"], "u");
    assert_eq!(payload["title"], "New row title");
    reply.send_blocking(Err("rename denied".into())).unwrap();
    cx.run_until_parked();
    assert!(app.read_with(cx, |app, _| app.sidebar_state.rename_target.is_some()));
    assert_eq!(
        app.read_with(cx, |app, cx| app
            .sidebar_rename
            .read(cx)
            .value()
            .to_string()),
        "New row title"
    );
    assert_eq!(
        app.read_with(cx, |app, _| app.selected_thread.clone()),
        Some("t".into())
    );
}

#[gpui_kit::test]
fn sidebar_parity_undo_waits_for_acceptance_and_retries_failed_inverse(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    seed_second_thread(&app, cx, false);
    right_click(cx, "thread-u");
    let settle = cx.debug_bounds("settle").unwrap();
    cx.simulate_click(settle.center(), Default::default());
    cx.run_until_parked();
    let connection::Command::Call { payload, reply, .. } = requests.try_recv().unwrap() else {
        panic!("settle");
    };
    assert_eq!(payload["threadId"], "u");
    assert!(cx.debug_bounds("undo-thread-navigation").is_none());
    reply.send_blocking(Ok(json!({}))).unwrap();
    cx.run_until_parked();
    let undo = cx.debug_bounds("undo-thread-navigation").unwrap();
    cx.simulate_click(undo.center(), Default::default());
    cx.run_until_parked();
    let connection::Command::Call { payload, reply, .. } = requests.try_recv().unwrap() else {
        panic!("inverse");
    };
    assert_eq!(payload["type"], "thread.unsettle");
    assert_eq!(payload["threadId"], "u");
    reply.send_blocking(Err("undo denied".into())).unwrap();
    cx.run_until_parked();
    assert!(cx.debug_bounds("undo-thread-navigation").is_some());
    let undo = cx.debug_bounds("undo-thread-navigation").unwrap();
    cx.simulate_click(undo.center(), Default::default());
    cx.run_until_parked();
    let connection::Command::Call { reply, .. } = requests.try_recv().unwrap() else {
        panic!("retry inverse");
    };
    reply.send_blocking(Ok(json!({}))).unwrap();
    cx.run_until_parked();
    assert!(app.read_with(cx, |app, _| app.thread_undo.record.is_none()));
}

#[gpui_kit::test]
fn sidebar_parity_cmd_z_keeps_text_undo_owned_by_input(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    cx.update(|_, cx| {
        app.update(cx, |app, _| {
            app.thread_undo.record = Some(crate::app::thread_undo::UndoRecord {
                environment: app.environment_scope.clone(),
                generation: app.generation,
                title: "Thread".into(),
                batch: false,
                inverses: vec![commands::command("thread.unsettle", "t")],
            });
        })
    });
    cx.simulate_input("edit");
    cx.simulate_keystrokes("cmd-z");
    cx.run_until_parked();
    assert!(requests.try_recv().is_err());
    cx.update(|window, cx| app.update(cx, |app, cx| app.focus.focus(window, cx)));
    cx.simulate_keystrokes("cmd-z");
    cx.run_until_parked();
    let connection::Command::Call { payload, .. } = requests.try_recv().unwrap() else {
        panic!("navigation undo");
    };
    assert_eq!(payload["type"], "thread.unsettle");
}

#[gpui_kit::test]
fn sidebar_parity_search_enter_exports_content_anchor(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.search.update(cx, |input, cx| {
                input.set_value("answer", window, cx);
                input.focus(window, cx);
            });
        })
    });
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    let connection::Command::Call { reply, .. } = requests.try_recv().unwrap() else {
        panic!("search");
    };
    reply.send_blocking(Ok(json!({"matches":[{"threadId":"t","projectId":"p","source":"assistant","snippet":"answer","messageCreatedAt":"2026-04-01T01:00:00Z"}]}))).unwrap();
    cx.run_until_parked();
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    let anchor = app
        .read_with(cx, |app, _| app.sidebar_state.message_anchor.clone())
        .unwrap();
    assert_eq!(anchor.thread_id, "t");
    assert_eq!(anchor.source, "assistant");
    assert_eq!(anchor.message_created_at, "2026-04-01T01:00:00Z");
    assert!(requests.try_recv().is_err());
}
