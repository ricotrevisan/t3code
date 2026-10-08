//! Saved-action terminal RPCs and stream receipts, with no timer/polling assertions.
use super::*;

fn call(
    requests: &Requests,
    expected: &str,
) -> (Value, async_channel::Sender<Result<Value, String>>) {
    let connection::Command::Call {
        tag,
        payload,
        reply,
    } = requests.try_recv().unwrap()
    else {
        panic!("expected {expected}")
    };
    assert_eq!(tag, expected);
    (payload, reply)
}

fn saved(app: &Entity<NativeApp>, cx: &mut VisualTestContext) {
    cx.update(|window, cx| app.update(cx, |app, cx| {
        app.project_settings = Some(json!({"projectSettingsFolded":true,"defaultProjectScripts":[{"id":"test","name":"Test","command":"printf saved","icon":"play"}]}));
        app.shell.threads[0].worktree_path = Some("/tmp/selected-checkout".into());
        // These are intentionally unrelated, editable forms.
        app.action_command.update(cx, |input,cx| input.set_value("UNSAVED",window,cx));
        app.workspace.update(cx, |input,cx| input.set_value("/tmp/add-form",window,cx));
        app.run_project_action("test", cx);
    }));
    cx.run_until_parked();
}

#[gpui_kit::test]
fn captured_checkout_receipts_output_and_reverse_controls(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    saved(&app, cx);
    let (open, receipt) = call(&requests, "terminal.open");
    assert_eq!(open["cwd"], "/tmp/selected-checkout");
    assert_eq!(open["worktreePath"], "/tmp/selected-checkout");
    assert_eq!(open["threadId"], "t");
    assert_eq!(
        open["env"],
        json!({"T3CODE_PROJECT_ROOT":"/tmp/fixture","T3CODE_WORKTREE_PATH":"/tmp/selected-checkout"})
    );
    let key = open["terminalId"].as_str().unwrap().to_owned();
    // Switch selection while startup is pending; the original PTY remains a
    // labelled row and completes against its captured thread, never the new one.
    cx.update(|_, cx| app.update(cx, |app, _| app.selected_thread = Some("other".into())));
    receipt.send_blocking(Ok(json!({"threadId":"t","terminalId":key,"status":"running","history":"shell ready\n","sequence":0}))).unwrap();
    cx.run_until_parked();
    let connection::Command::Subscribe {
        key: subscription,
        tag,
        payload,
    } = requests.try_recv().unwrap()
    else {
        panic!("expected output subscription")
    };
    assert_eq!(tag, "terminal.attach");
    assert_eq!(subscription, key);
    assert_eq!(
        payload,
        json!({"threadId":"t","terminalId":key,"restartIfNotRunning":false})
    );
    let (write, receipt) = call(&requests, "terminal.write");
    assert_eq!(
        write,
        json!({"threadId":"t","terminalId":key,"data":"printf saved\r"})
    );
    receipt.send_blocking(Ok(Value::Null)).unwrap();
    cx.run_until_parked();
    cx.update(|_,cx| app.update(cx,|app,cx| {
        app.action_run_update(&key,Ok(json!({"type":"output","threadId":"wrong","terminalId":key,"sequence":90,"data":"WRONG"})),cx);
        app.action_run_update(&key,Ok(json!({"type":"output","threadId":"t","terminalId":key,"sequence":1,"data":"saved\n"})),cx);
        app.action_run_update(&key,Ok(json!({"type":"output","threadId":"t","terminalId":key,"sequence":1,"data":"duplicate"})),cx);
        assert_eq!(app.action_runs[0].output,"shell ready\nsaved\n");
        assert_eq!(app.selected_thread.as_deref(),Some("other"));
        app.control_action_run(&key,false,cx);
    }));
    cx.run_until_parked();
    let (stop, receipt) = call(&requests, "terminal.write");
    assert_eq!(
        stop,
        json!({"threadId":"t","terminalId":key,"data":"\u{3}"})
    );
    receipt.send_blocking(Ok(Value::Null)).unwrap();
    cx.run_until_parked();
    cx.update(|_, cx| app.update(cx, |app, cx| app.control_action_run(&key, true, cx)));
    cx.run_until_parked();
    let (close, receipt) = call(&requests, "terminal.close");
    assert_eq!(close, json!({"threadId":"t","terminalId":key}));
    receipt.send_blocking(Ok(Value::Null)).unwrap();
    cx.run_until_parked();
    assert!(
        matches!(requests.try_recv().unwrap(),connection::Command::Unsubscribe(actual) if actual == key)
    );
    app.read_with(cx, |app, _| {
        assert!(!app.action_runs[0].terminal_exists);
        assert_eq!(app.action_runs[0].status, "Closed");
    });
}

#[gpui_kit::test]
fn rejected_startup_is_visible_and_retry_uses_captured_script(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    saved(&app, cx);
    let (open, receipt) = call(&requests, "terminal.open");
    let key = open["terminalId"].as_str().unwrap().to_owned();
    receipt
        .send_blocking(Err("Terminal cwd does not exist".into()))
        .unwrap();
    cx.run_until_parked();
    assert!(requests.is_empty());
    app.read_with(cx, |app, _| {
        assert_eq!(app.action_runs[0].status, "Startup failed");
        assert_eq!(
            app.action_runs[0].error.as_deref(),
            Some("Terminal cwd does not exist")
        );
        assert!(!app.action_runs[0].busy);
    });
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.project_settings.as_mut().unwrap()["defaultProjectScripts"][0]["command"] =
                "changed".into();
            app.retry_action_run(&key, cx);
        })
    });
    cx.run_until_parked();
    let (retry, receipt) = call(&requests, "terminal.open");
    assert_eq!(retry["cwd"], open["cwd"]);
    assert_ne!(retry["terminalId"], open["terminalId"]);
    receipt.send_blocking(Ok(json!({"threadId":"t","terminalId":retry["terminalId"],"status":"running","history":"","sequence":0}))).unwrap();
    cx.run_until_parked();
    assert!(matches!(
        requests.try_recv().unwrap(),
        connection::Command::Subscribe { .. }
    ));
    let (write, receipt) = call(&requests, "terminal.write");
    assert_eq!(write["data"], "printf saved\r");
    receipt.send_blocking(Ok(Value::Null)).unwrap();
    cx.run_until_parked();
}

#[gpui_kit::test]
fn old_generation_launch_does_not_write_into_new_connection(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    saved(&app, cx);
    let (open, receipt) = call(&requests, "terminal.open");
    let key = open["terminalId"].as_str().unwrap().to_owned();
    cx.update(|_, cx| app.update(cx, |app, _| app.generation += 1));
    receipt.send_blocking(Ok(json!({"threadId":"t","terminalId":key,"status":"running","history":"old","sequence":0}))).unwrap();
    cx.run_until_parked();
    assert!(requests.is_empty());
    cx.update(|_,cx| app.update(cx,|app,cx| {
        app.action_run_update(&key,Ok(json!({"type":"output","threadId":"t","terminalId":key,"data":"stale","sequence":1})),cx);
        assert_eq!(app.action_runs[0].output,"");
        assert!(!app.action_runs[0].busy);
        assert!(app.action_runs[0].terminal_exists);
        assert!(app.action_runs[0].error.is_some());
    }));
}

#[gpui_kit::test]
fn failed_write_and_close_keep_terminal_reachable_and_forms_intact(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    saved(&app, cx);
    let (open, receipt) = call(&requests, "terminal.open");
    let key = open["terminalId"].as_str().unwrap().to_owned();
    receipt
        .send_blocking(Ok(
            json!({"threadId":"t","terminalId":key,"status":"running","history":"","sequence":0}),
        ))
        .unwrap();
    cx.run_until_parked();
    assert!(matches!(
        requests.try_recv().unwrap(),
        connection::Command::Subscribe { .. }
    ));
    let (_, receipt) = call(&requests, "terminal.write");
    receipt.send_blocking(Err("write rejected".into())).unwrap();
    cx.run_until_parked();
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            assert!(app.action_runs[0].terminal_exists);
            assert_eq!(app.action_runs[0].error.as_deref(), Some("write rejected"));
            assert_eq!(app.action_command.read(cx).value().as_str(), "UNSAVED");
            assert_eq!(app.workspace.read(cx).value().as_str(), "/tmp/add-form");
            app.retry_action_run(&key, cx); // Must not blindly duplicate a live shell.
            assert!(requests.is_empty());
            app.action_run_update(&key, Err("output subscription failed".into()), cx);
            assert_eq!(
                app.action_runs[0].error.as_deref(),
                Some("output subscription failed")
            );
            app.control_action_run(&key, true, cx);
        })
    });
    cx.run_until_parked();
    let (_, receipt) = call(&requests, "terminal.close");
    receipt.send_blocking(Err("close rejected".into())).unwrap();
    cx.run_until_parked();
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            assert!(!app.action_runs[0].busy);
            assert!(app.action_runs[0].terminal_exists);
            assert_eq!(app.action_runs[0].error.as_deref(), Some("close rejected"));
            app.control_action_run(&key, true, cx);
        })
    });
    cx.run_until_parked();
    let (_, receipt) = call(&requests, "terminal.close");
    receipt.send_blocking(Ok(Value::Null)).unwrap();
    cx.run_until_parked();
    assert!(matches!(
        requests.try_recv().unwrap(),
        connection::Command::Unsubscribe(_)
    ));
}

#[gpui_kit::test]
fn output_is_bounded_and_ansi_sequences_can_span_subscription_chunks(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    saved(&app, cx);
    let (open, receipt) = call(&requests, "terminal.open");
    let key = open["terminalId"].as_str().unwrap().to_owned();
    receipt
        .send_blocking(Ok(
            json!({"threadId":"t","terminalId":key,"status":"running","history":"","sequence":0}),
        ))
        .unwrap();
    cx.run_until_parked();
    assert!(matches!(
        requests.try_recv().unwrap(),
        connection::Command::Subscribe { .. }
    ));
    let (_, receipt) = call(&requests, "terminal.write");
    receipt.send_blocking(Ok(Value::Null)).unwrap();
    cx.run_until_parked();
    cx.update(|_, cx| app.update(cx, |app, cx| {
        for (sequence, data) in [(1, "\u{1b}[3"), (2, "1mhello\u{1b}[0m\n")] {
            app.action_run_update(&key, Ok(json!({"type":"output","threadId":"t","terminalId":key,"sequence":sequence,"data":data})), cx);
        }
        assert_eq!(app.action_runs[0].output, "hello\n");
        app.action_run_update(&key, Ok(json!({"type":"output","threadId":"t","terminalId":key,"sequence":3,"data":"é".repeat(40_000)})), cx);
        assert!(app.action_runs[0].output.len() <= 64 * 1024);
        assert!(app.action_runs[0].output.ends_with('é'));
    }));
}

fn run_entrypoint(cx: &mut TestAppContext, entry: &str) {
    let (app, cx, requests) = harness(cx);
    match entry {
        "palette" => cx.update(|window, cx| {
            app.update(cx, |app, cx| {
                app.execute_command(palette::Command::ProjectActions, window, cx)
            })
        }),
        "inspector" => {
            cx.update(|_, cx| {
                app.update(cx, |app, cx| {
                    app.set_inspector_open(true, cx);
                    app.right_tab = RightTab::Inspector;
                })
            });
            cx.run_until_parked();
            let bounds = cx.debug_bounds("section-project-actions").unwrap();
            cx.simulate_click(bounds.center(), Default::default());
        }
        _ => {
            let bounds = cx.debug_bounds("project-actions-header").unwrap();
            cx.simulate_click(bounds.center(), Default::default());
        }
    }
    cx.run_until_parked();
    let (_, receipt) = call(&requests, "server.getSettings");
    receipt.send_blocking(Ok(json!({"projectSettingsFolded":true,"defaultProjectScripts":[{"id":"test","name":"Test","command":"printf reachable","icon":"play"}]}))).unwrap();
    cx.run_until_parked();
    assert_eq!(
        app.read_with(cx, |app, _| app.inspector_section),
        Some(InspectorSection::ProjectActions)
    );
    let bounds = cx
        .debug_bounds("action-run-0")
        .expect("saved Run path is painted");
    cx.simulate_click(bounds.center(), Default::default());
    cx.run_until_parked();
    let (open, receipt) = call(&requests, "terminal.open");
    assert_eq!(open["cwd"], "/tmp/fixture");
    assert_eq!(open["threadId"], "t");
    receipt
        .send_blocking(Err("deliberate rejection".into()))
        .unwrap();
    cx.run_until_parked();
}

#[gpui_kit::test]
fn header_actions_reveals_runnable_saved_actions(cx: &mut TestAppContext) {
    run_entrypoint(cx, "header");
}
#[gpui_kit::test]
fn inspector_actions_reveals_runnable_saved_actions(cx: &mut TestAppContext) {
    run_entrypoint(cx, "inspector");
}
#[gpui_kit::test]
fn palette_actions_reveals_runnable_saved_actions(cx: &mut TestAppContext) {
    run_entrypoint(cx, "palette");
}

fn live_run(
    app: &Entity<NativeApp>,
    cx: &mut VisualTestContext,
    requests: &Requests,
) -> String {
    saved(app, cx);
    let (open, receipt) = call(requests, "terminal.open");
    let key = open["terminalId"].as_str().unwrap().to_owned();
    receipt.send_blocking(Ok(json!({"threadId":"t","terminalId":key,"status":"running","history":"old\n","sequence":2}))).unwrap();
    cx.run_until_parked();
    assert!(matches!(
        requests.try_recv().unwrap(),
        connection::Command::Subscribe { .. }
    ));
    let (_, receipt) = call(requests, "terminal.write");
    receipt.send_blocking(Ok(Value::Null)).unwrap();
    cx.run_until_parked();
    key
}

fn fresh_sync(
    app: &Entity<NativeApp>,
    cx: &mut VisualTestContext,
    scope: &str,
) -> Requests {
    let (sender, requests) = async_channel::bounded(16);
    cx.update(|_, cx| app.update(cx, |app, cx| {
        app.environment_scope = scope.into();
        app.commands = Some(sender);
        app.action_runs_synchronized(cx);
        assert!(requests.is_empty(), "must wait for synchronized shell");
        app.shell.apply(serde_json::from_value(json!({"kind":"snapshot","snapshot":{"snapshotSequence":0,"projects":[],"threads":[]}})).unwrap()).unwrap();
        app.shell.apply(shell::ShellItem::Synchronized).unwrap();
        app.action_runs_synchronized(cx);
        app.action_runs_synchronized(cx); // Exactly once, even on repeated sync notifications.
    }));
    cx.run_until_parked();
    requests.into()
}

fn recovered_stream(cx: &mut TestAppContext, switch_environment: bool) {
    let (app, cx, requests) = harness(cx);
    let key = live_run(&app, cx, &requests);
    let scope = app.read_with(cx, |app, _| app.environment_scope.clone());
    cx.update(|_, cx| app.update(cx, |app, cx| app.disconnect(cx)));
    if switch_environment {
        let b = fresh_sync(&app, cx, "environment-B");
        assert!(b.is_empty());
        cx.update(|_, cx| {
            app.update(cx, |app, cx| {
                app.action_run_update(
                    &key,
                    Ok(json!({"type":"closed","threadId":"t","terminalId":key,"sequence":99})),
                    cx,
                );
                assert!(app.action_runs[0].terminal_exists);
                app.disconnect(cx);
            })
        });
    }
    let recovered = fresh_sync(&app, cx, &scope);
    let connection::Command::Subscribe {
        key: stream,
        tag,
        payload,
    } = recovered.try_recv().unwrap()
    else {
        panic!("expected attach only")
    };
    assert_ne!(stream, key);
    assert_eq!(tag, "terminal.attach");
    assert_eq!(
        payload,
        json!({"threadId":"t","terminalId":key,"restartIfNotRunning":false})
    );
    assert!(recovered.is_empty()); // Never open or replay command.
    cx.update(|_, cx| app.update(cx, |app, cx| {
        app.action_run_update(&key, Err("stale A callback".into()), cx);
        app.action_run_update(&key, Ok(json!({"type":"closed","threadId":"t","terminalId":key,"sequence":99})), cx);
        app.action_run_update(&stream, Ok(json!({"type":"snapshot","snapshot":{"threadId":"wrong","terminalId":key,"sequence":99,"history":"B"}})), cx);
        app.action_run_update(&stream, Ok(json!({"type":"snapshot","snapshot":{"threadId":"t","terminalId":key,"sequence":5,"history":"caught up\n","status":"running"}})), cx);
        app.action_run_update(&stream, Ok(json!({"type":"output","threadId":"t","terminalId":key,"sequence":5,"data":"duplicate"})), cx);
        app.action_run_update(&stream, Ok(json!({"type":"output","threadId":"t","terminalId":key,"sequence":6,"data":"live\n"})), cx);
        app.action_run_update(&stream, Ok(json!({"type":"exited","threadId":"t","terminalId":key,"sequence":7,"exitCode":0})), cx);
        assert_eq!(app.action_runs[0].output, "caught up\nlive\n");
        assert_eq!(app.action_runs[0].status, "Exited · 0");
        assert!(app.action_runs[0].error.is_none());
        app.retry_action_run(&key, cx);
        assert!(recovered.is_empty());
        app.control_action_run(&key, false, cx);
    }));
    cx.run_until_parked();
    let (stop, receipt) = call(&recovered, "terminal.write");
    assert_eq!(
        stop,
        json!({"threadId":"t","terminalId":key,"data":"\u{3}"})
    );
    receipt.send_blocking(Ok(Value::Null)).unwrap();
    cx.run_until_parked();
    cx.update(|_, cx| app.update(cx, |app, cx| app.control_action_run(&key, true, cx)));
    cx.run_until_parked();
    let (close, receipt) = call(&recovered, "terminal.close");
    assert_eq!(close, json!({"threadId":"t","terminalId":key}));
    receipt.send_blocking(Ok(Value::Null)).unwrap();
    cx.run_until_parked();
    assert!(
        matches!(recovered.try_recv().unwrap(), connection::Command::Unsubscribe(actual) if actual == stream)
    );
}

#[gpui_kit::test]
fn disconnect_sync_return_attaches_without_restarting(cx: &mut TestAppContext) {
    recovered_stream(cx, false);
}

#[gpui_kit::test]
fn environment_a_b_a_renews_only_a_stream(cx: &mut TestAppContext) {
    recovered_stream(cx, true);
}

#[gpui_kit::test]
fn reconnect_attach_survives_command_channel_backpressure(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    let first = live_run(&app, cx, &requests);
    let second = live_run(&app, cx, &requests);
    let (sender, recovered) = async_channel::bounded(1);
    let recovered = Requests::from(recovered);
    cx.update(|_, cx| app.update(cx, |app, cx| {
        app.disconnect(cx);
        app.commands = Some(sender);
        app.shell.apply(serde_json::from_value(json!({"kind":"snapshot","snapshot":{"snapshotSequence":0,"projects":[],"threads":[]}})).unwrap()).unwrap();
        app.shell.apply(shell::ShellItem::Synchronized).unwrap();
        app.action_runs_synchronized(cx);
        app.action_runs_synchronized(cx);
    }));
    let mut terminals = Vec::new();
    for _ in 0..2 {
        cx.run_until_parked();
        let connection::Command::Subscribe { tag, payload, .. } = recovered.try_recv().unwrap()
        else {
            panic!("attach only")
        };
        assert_eq!(tag, "terminal.attach");
        assert_eq!(payload["restartIfNotRunning"], false);
        terminals.push(payload["terminalId"].as_str().unwrap().to_owned());
    }
    terminals.sort();
    let mut expected = vec![first, second];
    expected.sort();
    assert_eq!(terminals, expected);
    cx.run_until_parked();
    assert!(recovered.is_empty());
}

fn late_mutation(cx: &mut TestAppContext, closing: bool) {
    let (app, cx, requests) = harness(cx);
    saved(&app, cx);
    let (open, receipt) = call(&requests, "terminal.open");
    let key = open["terminalId"].as_str().unwrap().to_owned();
    receipt
        .send_blocking(Ok(
            json!({"threadId":"t","terminalId":key,"sequence":0,"history":"","status":"running"}),
        ))
        .unwrap();
    cx.run_until_parked();
    assert!(matches!(
        requests.try_recv().unwrap(),
        connection::Command::Subscribe { .. }
    ));
    let (_, write) = call(&requests, "terminal.write");
    let late = if closing {
        write.send_blocking(Ok(Value::Null)).unwrap();
        cx.run_until_parked();
        cx.update(|_, cx| app.update(cx, |app, cx| app.control_action_run(&key, true, cx)));
        cx.run_until_parked();
        call(&requests, "terminal.close").1
    } else {
        write
    };
    let scope = app.read_with(cx, |app, _| app.environment_scope.clone());
    cx.update(|_, cx| app.update(cx, |app, cx| app.disconnect(cx)));
    let recovered = fresh_sync(&app, cx, &scope);
    let connection::Command::Subscribe { key: stream, .. } = recovered.try_recv().unwrap() else {
        panic!("expected attach")
    };
    cx.update(|_, cx| app.update(cx, |app, cx| {
        app.action_run_update(&stream, Ok(json!({"type":"snapshot","snapshot":{"threadId":"t","terminalId":key,"sequence":9,"history":"recovered","status":"running"}})), cx);
        app.control_action_run(&key, true, cx);
    }));
    cx.run_until_parked();
    let (_, current) = call(&recovered, "terminal.close");
    late.send_blocking(if closing {
        Ok(Value::Null)
    } else {
        Err("outcome is unknown".into())
    })
    .unwrap();
    cx.run_until_parked();
    app.read_with(cx, |app, _| {
        assert!(
            app.action_runs[0].busy,
            "late receipt must not release a new control"
        );
        assert!(app.action_runs[0].terminal_exists);
        assert_eq!(app.action_runs[0].output, "recovered");
        assert!(app.action_runs[0].error.is_none());
    });
    assert!(
        recovered.is_empty(),
        "old close cannot unsubscribe new stream"
    );
    current.send_blocking(Ok(Value::Null)).unwrap();
    cx.run_until_parked();
    assert!(
        matches!(recovered.try_recv().unwrap(), connection::Command::Unsubscribe(actual) if actual == stream)
    );
}

#[gpui_kit::test]
fn late_unknown_write_preserves_new_stream_and_control(cx: &mut TestAppContext) {
    late_mutation(cx, false);
}

#[gpui_kit::test]
fn close_in_flight_disconnect_return_and_close_again(cx: &mut TestAppContext) {
    late_mutation(cx, true);
}

#[gpui_kit::test]
fn unknown_open_remains_closeable_and_same_generation_resync_does_not_duplicate_attach(
    cx: &mut TestAppContext,
) {
    let (app, cx, requests) = harness(cx);
    saved(&app, cx);
    let (open, receipt) = call(&requests, "terminal.open");
    let key = open["terminalId"].as_str().unwrap().to_owned();
    receipt
        .send_blocking(Err("terminal.open outcome is unknown".into()))
        .unwrap();
    cx.run_until_parked();
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            assert!(app.action_runs[0].terminal_exists);
            app.retry_action_run(&key, cx);
            assert!(requests.is_empty());
            // Same generation, but startup never established a stream. Attach once
            // after resync; never replay the possibly accepted open or command.
            app.action_runs_connection_lost();
            app.shell.apply(shell::ShellItem::Synchronized).unwrap();
            app.action_runs_synchronized(cx);
            app.action_runs_synchronized(cx);
        })
    });
    cx.run_until_parked();
    let connection::Command::Subscribe { key: stream, .. } = requests.try_recv().unwrap() else {
        panic!("expected attach")
    };
    assert!(requests.is_empty());
    cx.update(|_, cx| app.update(cx, |app, cx| {
        app.action_run_update(&stream, Ok(json!({"type":"snapshot","snapshot":{"threadId":"t","terminalId":key,"sequence":3,"history":"history","status":"running"}})), cx);
        app.action_runs_connection_lost();
        app.action_runs_synchronized(cx); // Transport already replays this stream.
        assert!(requests.is_empty());
        app.action_run_update(&stream, Ok(json!({"type":"snapshot","snapshot":{"threadId":"t","terminalId":key,"sequence":3,"history":"history","status":"running"}})), cx);
        assert!(app.action_runs[0].error.is_none());
        assert_eq!(app.action_runs[0].output, "history");
        app.control_action_run(&key, true, cx);
    }));
    cx.run_until_parked();
    let (_, receipt) = call(&requests, "terminal.close");
    receipt.send_blocking(Ok(Value::Null)).unwrap();
    cx.run_until_parked();
    assert!(
        matches!(requests.try_recv().unwrap(), connection::Command::Unsubscribe(actual) if actual == stream)
    );
}

#[gpui_kit::test]
fn late_open_receipt_and_unknown_attach_do_not_erase_recovered_ownership(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    saved(&app, cx);
    let (open, late) = call(&requests, "terminal.open");
    let key = open["terminalId"].as_str().unwrap().to_owned();
    let scope = app.read_with(cx, |app, _| app.environment_scope.clone());
    cx.update(|_, cx| app.update(cx, |app, cx| app.disconnect(cx)));
    let recovered = fresh_sync(&app, cx, &scope);
    let connection::Command::Subscribe { key: stream, .. } = recovered.try_recv().unwrap() else {
        panic!("expected attach")
    };
    cx.update(|_, cx| app.update(cx, |app, cx| {
        app.action_run_update(&stream, Ok(json!({"type":"snapshot","snapshot":{"threadId":"t","terminalId":key,"sequence":8,"history":"new owner","status":"running"}})), cx);
    }));
    late.send_blocking(Ok(json!({"threadId":"t","terminalId":key,"sequence":0,"history":"old receipt","status":"running"}))).unwrap();
    cx.run_until_parked();
    assert!(requests.is_empty());
    assert!(recovered.is_empty());
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            assert_eq!(app.action_runs[0].output, "new owner");
            assert!(app.action_runs[0].error.is_none());
            // Neither a missing/malformed snapshot nor a failed attach proves absence.
            app.action_run_update(&stream, Ok(json!({"type":"snapshot","snapshot":null})), cx);
            app.action_run_update(
                &stream,
                Err("terminal not found / outcome is unknown".into()),
                cx,
            );
            app.retry_action_run(&key, cx);
            assert!(recovered.is_empty());
            assert!(app.action_runs[0].terminal_exists);
            app.control_action_run(&key, true, cx);
        })
    });
    cx.run_until_parked();
    let (_, receipt) = call(&recovered, "terminal.close");
    receipt
        .send_blocking(Err("outcome is unknown".into()))
        .unwrap();
    cx.run_until_parked();
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            assert!(!app.action_runs[0].busy);
            assert!(app.action_runs[0].terminal_exists);
            app.retry_action_run(&key, cx);
            assert!(recovered.is_empty());
            app.control_action_run(&key, true, cx);
        })
    });
    cx.run_until_parked();
    let (_, receipt) = call(&recovered, "terminal.close");
    receipt.send_blocking(Ok(Value::Null)).unwrap();
    cx.run_until_parked();
    assert!(
        matches!(recovered.try_recv().unwrap(), connection::Command::Unsubscribe(actual) if actual == stream)
    );
}
