use super::*;

fn background_item(app: &Entity<NativeApp>, cx: &mut VisualTestContext, key: &str, item: Value) {
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.apply_queue_subscription(key, Ok(item), cx).unwrap();
        })
    });
    cx.run_until_parked();
}

fn background_snapshot(tool: &str, sequence: u64) -> Value {
    json!({"kind":"snapshot","snapshot":{"snapshotSequence":sequence,"thread":{
        "id":"t","messages":[],"activities":[{"id":tool,"kind":"tool.completed","sequence":sequence,"payload":{}}],
        "session":{"status":"running"},"latestTurn":{"turnId":"old"}
    }}})
}

fn background_tool(tool: &str, sequence: u64) -> Value {
    json!({"kind":"event","event":{"sequence":sequence,"type":"thread.activity-appended",
    "occurredAt":"2026-01-01T00:00:03Z","payload":{"threadId":"t","activity":{
        "id":tool,"kind":"tool.completed","sequence":sequence,"payload":{}
    }}}})
}

fn prepare_background(
    app: &Entity<NativeApp>,
    cx: &mut VisualTestContext,
    requests: &Requests,
) -> (String, t3code_desktop_gpui::queue::QueuedMessage) {
    projection(app, cx, "running", "old", Some("known-tool"), false);
    cx.update(|_, cx| app.update(cx, |app, _| {
        app.shell.synchronized = true;
        app.shell.threads[0].session = Some(t3code_desktop_gpui::shell::Session {
            status: "running".into(), last_error: None,
        });
        app.attachments.push(json!({"id":"attachment","name":"notes.md","mimeType":"text/markdown","sizeBytes":12}));
    }));
    queue_prompt(cx, "later [main.rs](src/main.rs)");
    let connection::Command::Subscribe { key, tag, payload } = requests.try_recv().unwrap() else {
        panic!("queue must subscribe its thread");
    };
    assert_eq!(tag, "orchestration.subscribeThread");
    assert_eq!(
        payload,
        json!({"threadId":"t","reasoningMessages":false,"requestCompletionMarker":true})
    );
    assert!(requests.try_recv().is_err());
    let captured = cx.update(|_, cx| {
        app.update(cx, |app, _| {
            app.selected_thread = None;
            app.detail = thread::ThreadDetail::default();
            app.queues.messages("t")[0].clone()
        })
    });
    (key, captured)
}

#[gpui_kit::test]
fn loading_selected_detail_uses_shell_phase_and_queues_instead_of_steering(
    cx: &mut TestAppContext,
) {
    let (app, cx, requests) = harness(cx);
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.detail = Default::default();
            app.shell.threads[0].session =
                Some(serde_json::from_value(json!({"status":"running"})).unwrap());
            app.composer.update(cx, |input, cx| {
                input.set_value("while detail loads", window, cx)
            });
            cx.notify();
        })
    });
    cx.run_until_parked();
    assert!(cx.debug_bounds("stop-turn").is_some());
    assert!(cx.debug_bounds("send").is_none());
    cx.update(|window, cx| app.update(cx, |app, cx| app.send_prompt(window, cx)));
    cx.run_until_parked();
    app.read_with(cx, |app, cx| {
        assert_eq!(app.queues.messages("t").len(), 1);
        assert_eq!(app.queues.messages("t")[0].text, "while detail loads");
        assert!(app.composer.read(cx).value().is_empty());
    });
    while let Ok(command) = requests.try_recv() {
        assert!(
            !matches!(command, connection::Command::Call { .. }),
            "default Queue must not dispatch an in-turn steer while detail loads"
        );
    }
}

#[gpui_kit::test]
fn background_stream_baselines_history_guards_requests_and_tears_down(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    let (key, captured) = prepare_background(&app, cx, &requests);
    background_item(&app, cx, &key, background_snapshot("historical-tool", 10));
    background_item(&app, cx, &key, background_tool("replayed-tool", 11));
    assert!(requests.try_recv().is_err());
    background_item(&app, cx, &key, json!({"kind":"synchronized"}));
    assert!(requests.try_recv().is_err());
    // Approval and user input shell guards apply even with an independent detail cache.
    for guard in ["approval", "user-input"] {
        cx.update(|_, cx| {
            app.update(cx, |app, _| {
                app.shell.threads[0].has_pending_approvals = guard == "approval";
                app.shell.threads[0].has_pending_user_input = guard == "user-input";
            })
        });
        background_item(
            &app,
            cx,
            &key,
            background_tool(guard, if guard == "approval" { 12 } else { 13 }),
        );
        assert!(requests.try_recv().is_err());
    }
    cx.update(|_, cx| {
        app.update(cx, |app, _| {
            app.shell.threads[0].has_pending_approvals = false;
            app.shell.threads[0].has_pending_user_input = false;
            app.shell.threads[0].runtime_mode = Some("approval-required".into());
        })
    });
    background_item(&app, cx, &key, background_tool("live-tool", 14));
    let connection::Command::Call {
        tag,
        payload,
        reply,
    } = requests.try_recv().unwrap()
    else {
        panic!("a genuine live background boundary must start the queued turn");
    };
    assert_eq!(tag, "orchestration.dispatchCommand");
    let expected = captured.start_command("t").unwrap();
    for field in [
        "type",
        "threadId",
        "modelSelection",
        "runtimeMode",
        "interactionMode",
    ] {
        assert_eq!(payload[field], expected[field]);
    }
    for field in ["text", "attachments", "context", "role"] {
        assert_eq!(payload["message"][field], expected["message"][field]);
    }
    assert!(
        payload["message"]["messageId"]
            .as_str()
            .is_some_and(|id| !id.is_empty())
    );
    reply.send_blocking(Ok(json!({"sequence":15}))).unwrap();
    cx.run_until_parked();
    assert!(
        matches!(requests.try_recv().unwrap(), connection::Command::Unsubscribe(old) if old == key)
    );
    assert!(requests.try_recv().is_err());
    background_item(&app, cx, &key, background_tool("late-tool", 16));
    assert!(requests.try_recv().is_err());
}

#[gpui_kit::test]
fn background_reconnect_rejects_old_callbacks_and_requires_fresh_live_boundary(
    cx: &mut TestAppContext,
) {
    let (app, cx, requests) = harness(cx);
    let (old, captured) = prepare_background(&app, cx, &requests);
    background_item(&app, cx, &old, background_snapshot("known-tool", 10));
    background_item(&app, cx, &old, json!({"kind":"synchronized"}));
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.clear_queue_subscriptions();
            app.generation += 1;
            app.flush_queues(cx);
        })
    });
    assert!(
        matches!(requests.try_recv().unwrap(), connection::Command::Unsubscribe(key) if key == old)
    );
    let connection::Command::Subscribe { key, .. } = requests.try_recv().unwrap() else {
        panic!("fresh subscription");
    };
    assert_ne!(key, old);
    background_item(&app, cx, &old, background_tool("stale-tool", 11));
    assert!(requests.try_recv().is_err());
    background_item(&app, cx, &key, background_snapshot("gap-tool", 20));
    background_item(&app, cx, &key, json!({"kind":"synchronized"}));
    cx.update(|_, cx| app.update(cx, |app, cx| app.flush_queues(cx)));
    cx.run_until_parked();
    assert!(requests.try_recv().is_err());
    background_item(&app, cx, &key, background_tool("gap-tool", 21));
    assert!(requests.try_recv().is_err());
    background_item(&app, cx, &key, background_tool("fresh-tool", 22));
    let sent = calls(&requests);
    assert_eq!(sent.len(), 1);
    let payload = &sent[0].0;
    assert_eq!(
        payload["message"]["text"],
        "later [main.rs](t3-context://v1/mention/mention_1)"
    );
    assert_eq!(
        payload["message"]["context"]["records"][0]["kind"],
        "mention"
    );
    assert_eq!(
        payload["message"]["context"]["records"][0]["path"],
        "src/main.rs"
    );
    let expected = captured.start_command("t").unwrap();
    for field in ["modelSelection", "runtimeMode", "interactionMode"] {
        assert_eq!(payload[field], expected[field]);
    }
    for field in ["attachments", "context", "role"] {
        assert_eq!(payload["message"][field], expected["message"][field]);
    }
    assert!(
        payload["message"]["messageId"]
            .as_str()
            .is_some_and(|id| !id.is_empty())
    );
}

#[gpui_kit::test]
fn fresh_ready_background_baseline_releases_head_without_live_events(cx: &mut TestAppContext) {
    for status in ["running", "ready"] {
        let (app, cx, requests) = harness(cx);
        let (key, _) = prepare_background(&app, cx, &requests);
        let mut snapshot = background_snapshot("historical-tool", 10);
        snapshot["snapshot"]["thread"]["session"]["status"] = json!(status);
        background_item(&app, cx, &key, snapshot);
        assert!(requests.try_recv().is_err());
        background_item(&app, cx, &key, background_tool("replay-tool", 11));
        assert!(requests.try_recv().is_err());
        background_item(&app, cx, &key, json!({"kind":"synchronized"}));
        let sent = calls(&requests);
        assert_eq!(sent.len(), usize::from(status == "ready"), "{status}");
    }
}

#[gpui_kit::test]
fn shell_completion_while_cache_connects_still_honors_request_guards(cx: &mut TestAppContext) {
    for guard in ["approval", "user-input"] {
        let (app, cx, requests) = harness(cx);
        let (key, _) = prepare_background(&app, cx, &requests);
        background_item(&app, cx, &key, background_snapshot("history", 10));
        cx.update(|_, cx| {
            app.update(cx, |app, cx| {
                app.shell.threads[0].session.as_mut().unwrap().status = "ready".into();
                app.shell.threads[0].has_pending_approvals = guard == "approval";
                app.shell.threads[0].has_pending_user_input = guard == "user-input";
                app.flush_queues(cx);
            })
        });
        cx.run_until_parked();
        assert!(requests.try_recv().is_err());
        cx.update(|_, cx| {
            app.update(cx, |app, cx| {
                app.shell.threads[0].has_pending_approvals = false;
                app.shell.threads[0].has_pending_user_input = false;
                app.flush_queues(cx);
            })
        });
        cx.run_until_parked();
        let sent = calls(&requests);
        assert_eq!(sent.len(), 1, "{guard}");
    }
}

#[gpui_kit::test]
fn connecting_queue_cache_does_not_hide_selected_tool_boundary(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    projection(&app, cx, "running", "old", Some("known"), false);
    cx.update(|_, cx| {
        app.update(cx, |app, _| {
            app.shell.synchronized = true;
        })
    });
    queue_prompt(cx, "next");
    assert!(matches!(
        requests.try_recv().unwrap(),
        connection::Command::Subscribe { .. }
    ));
    assert!(requests.try_recv().is_err());
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.apply_thread_update("t", background_tool("fresh", 2), cx)
                .unwrap();
        })
    });
    cx.run_until_parked();
    let sent = calls(&requests);
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].0["message"]["text"], "next");
}

#[gpui_kit::test]
fn reconnect_ready_baseline_cannot_ack_accepted_dispatch_or_replay_tool(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    let (old, captured) = prepare_background(&app, cx, &requests);
    cx.update(|_, cx| {
        app.update(cx, |app, _| {
            app.queues.push("t", captured, false);
        })
    });
    background_item(&app, cx, &old, background_snapshot("history", 10));
    background_item(&app, cx, &old, json!({"kind":"synchronized"}));
    background_item(&app, cx, &old, background_tool("live", 11));
    let mut sent = calls(&requests);
    assert_eq!(sent.len(), 1);
    sent.remove(0)
        .1
        .send_blocking(Ok(json!({"sequence":12})))
        .unwrap();
    cx.run_until_parked();
    assert!(requests.try_recv().is_err());
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.clear_queue_subscriptions();
            app.generation += 1;
            app.flush_queues(cx);
        })
    });
    assert!(
        matches!(requests.try_recv().unwrap(), connection::Command::Unsubscribe(key) if key == old)
    );
    let connection::Command::Subscribe { key, .. } = requests.try_recv().unwrap() else {
        panic!("fresh subscription");
    };
    let mut snapshot = background_snapshot("gap-tool", 20);
    snapshot["snapshot"]["thread"]["session"]["status"] = json!("ready");
    snapshot["snapshot"]["thread"]["latestTurn"]["turnId"] = json!("gap-turn");
    background_item(&app, cx, &key, snapshot);
    background_item(&app, cx, &key, background_tool("replay-tool", 21));
    background_item(&app, cx, &key, json!({"kind":"synchronized"}));
    cx.update(|_, cx| app.update(cx, |app, cx| app.flush_queues(cx)));
    cx.run_until_parked();
    assert!(
        requests.try_recv().is_err(),
        "history must not acknowledge an accepted head"
    );
    background_item(&app, cx, &key, background_tool("replay-tool", 22));
    assert!(
        requests.try_recv().is_err(),
        "a repeated history tool is not a receipt"
    );
}

fn projection(
    app: &Entity<NativeApp>,
    cx: &mut VisualTestContext,
    status: &str,
    turn: &str,
    tool: Option<&str>,
    blocked: bool,
) {
    cx.update(|_, cx| app.update(cx, |app, cx| {
        let mut activities = Vec::new();
        if let Some(id) = tool { activities.push(json!({"id":id,"kind":"tool.completed","sequence":1,"createdAt":"2026-01-01T00:00:01Z","payload":{}})); }
        if blocked { activities.push(json!({"id":"approval","kind":"approval.requested","createdAt":"2026-01-01T00:00:02Z","payload":{"requestId":"request","requestKind":"command"}})); }
        let mut user = message(0);
        user["role"] = json!("user");
        let item = json!({"kind":"snapshot","snapshot":{"snapshotSequence":1,"thread":{"id":"t","title":"Thread","messages":[user],"activities":activities,"latestTurn":{"turnId":turn,"state":if status == "running" {"running"} else {"completed"}},"session":{"status":status}}}});
        app.apply_thread_update("t", item, cx).unwrap();
        cx.notify();
    }));
    cx.run_until_parked();
}

#[gpui_kit::test]
fn tool_boundary_dispatch_waits_for_receipt_and_does_not_reuse_boundary(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    projection(&app, cx, "running", "old", None, false);
    queue_prompt(cx, "first");
    queue_prompt(cx, "second");
    projection(&app, cx, "running", "old", Some("tool-1"), false);
    let mut sent = calls(&requests);
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].0["message"]["text"], "first");
    projection(&app, cx, "running", "new", Some("tool-1"), false);
    assert!(calls(&requests).is_empty());
    sent.remove(0)
        .1
        .send_blocking(Ok(json!({"sequence":2})))
        .unwrap();
    cx.run_until_parked();
    assert!(calls(&requests).is_empty());
    projection(&app, cx, "running", "new", Some("tool-2"), false);
    let sent = calls(&requests);
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].0["message"]["text"], "second");
}

#[gpui_kit::test]
fn completion_identity_unstalls_queue_without_running_projection(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    projection(&app, cx, "running", "old", None, false);
    queue_prompt(cx, "first");
    queue_prompt(cx, "second");
    projection(&app, cx, "ready", "old", None, false);
    let mut sent = calls(&requests);
    assert_eq!(sent.len(), 1);
    sent.remove(0)
        .1
        .send_blocking(Ok(json!({"sequence":2})))
        .unwrap();
    cx.run_until_parked();
    assert!(calls(&requests).is_empty());
    projection(&app, cx, "ready", "completed-without-running", None, false);
    let sent = calls(&requests);
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].0["message"]["text"], "second");
}

#[gpui_kit::test]
fn switching_selection_with_completed_tool_anchor_never_sends_early(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    projection(&app, cx, "running", "old", Some("tool-1"), false);
    cx.update(|_, cx| {
        app.update(cx, |app, _| {
            app.shell.threads[0].session = Some(t3code_desktop_gpui::shell::Session {
                status: "running".into(),
                last_error: None,
            });
            let mut other = app.shell.threads[0].clone();
            other.id = "u".into();
            app.shell.threads.push(other);
        })
    });
    queue_prompt(cx, "wait for a real boundary");
    assert!(calls(&requests).is_empty());
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.select_thread("u", window, cx);
            app.flush_queues(cx);
        })
    });
    cx.run_until_parked();
    assert!(calls(&requests).is_empty());
    // Older shells can omit live session state entirely. Unknown is not idle.
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.shell.threads[0].session = None;
            app.flush_queues(cx);
        })
    });
    cx.run_until_parked();
    assert!(calls(&requests).is_empty());
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.select_thread("t", window, cx);
            app.flush_queues(cx);
        })
    });
    cx.run_until_parked();
    assert!(calls(&requests).is_empty());
    projection(&app, cx, "running", "old", Some("tool-1"), false);
    assert!(calls(&requests).is_empty());
    projection(&app, cx, "running", "old", Some("tool-2"), false);
    let sent = calls(&requests);
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].0["message"]["text"], "wait for a real boundary");
}

#[gpui_kit::test]
fn session_and_matching_start_failure_receipts_unstall_without_new_turn(cx: &mut TestAppContext) {
    for marker in [
        "session",
        "shell-session",
        "failure",
        "unrelated-failure",
        "turn-timestamp",
    ] {
        let (app, cx, requests) = harness(cx);
        projection(&app, cx, "running", "old", None, false);
        queue_prompt(cx, "first");
        queue_prompt(cx, "second");
        projection(&app, cx, "ready", "old", None, false);
        let mut sent = calls(&requests);
        assert_eq!(sent.len(), 1);
        sent.remove(0)
            .1
            .send_blocking(Ok(json!({"sequence":2})))
            .unwrap();
        cx.run_until_parked();
        assert!(calls(&requests).is_empty());
        cx.update(|_, cx| app.update(cx, |app, cx| {
            if marker == "shell-session" {
                // Background shell receipts still acknowledge, but a fresh
                // detail baseline must arrive before automatic dispatch.
                app.shell.threads[0].session = Some(t3code_desktop_gpui::shell::Session {
                    status: "error".into(), last_error: Some("start failed".into()),
                });
                app.selected_thread = None;
                app.detail = thread::ThreadDetail::default();
                app.shell.synchronized = true;
                app.flush_queues(cx);
                let key = app.queue_details["t"].key.clone();
                let mut snapshot = background_snapshot("historical", 10);
                snapshot["snapshot"]["thread"]["session"]["status"] = json!("error");
                app.apply_queue_subscription(&key, Ok(snapshot), cx).unwrap();
                app.apply_queue_subscription(&key, Ok(json!({"kind":"synchronized"})), cx).unwrap();
                app.apply_queue_subscription(&key, Ok(background_tool("live", 11)), cx).unwrap();
                return;
            }
            let mut detail = app.detail.thread.as_ref().unwrap().as_ref().clone();
            match marker {
                "session" => detail["session"]["updatedAt"] = json!("2026-01-01T00:00:03Z"),
                "turn-timestamp" => detail["latestTurn"]["completedAt"] = json!("2026-01-01T00:00:03Z"),
                _ => {
                    let request_id = if marker == "failure" {
                        detail["messages"][0]["id"].clone()
                    } else { json!("unrelated") };
                    detail["activities"] = json!([{"id":"failed-start","kind":"provider.turn.start.failed","payload":{"requestId":request_id}}]);
                }
            }
            app.apply_thread_update("t", json!({"kind":"snapshot","snapshot":{"snapshotSequence":2,"thread":detail}}), cx).unwrap();
        }));
        cx.run_until_parked();
        let sent = calls(&requests);
        if marker == "unrelated-failure" {
            assert!(sent.is_empty());
        } else {
            assert_eq!(sent.len(), 1, "{marker}");
            assert_eq!(sent[0].0["message"]["text"], "second");
        }
    }
}

#[gpui_kit::test]
fn rejected_steer_retains_attachments_context_and_settings_for_retry(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    projection(&app, cx, "running", "old", None, false);
    cx.update(|_, cx| app.update(cx, |app, _| {
        app.follow_up_behavior = t3code_desktop_gpui::queue::FollowUpBehavior::Steer;
        app.attachments.push(json!({"id":"attachment","name":"notes.md","mimeType":"text/markdown","sizeBytes":12}));
    }));
    queue_prompt(cx, "steer me [main.rs](src/main.rs)");
    let mut sent = calls(&requests);
    assert_eq!(sent.len(), 1);
    let (original, reply) = sent.remove(0);
    assert_eq!(original["type"], "thread.turn.start");
    assert_eq!(
        original["message"]["context"]["records"][0]["kind"],
        "mention"
    );
    reply
        .send_blocking(Err("Provider does not support steering".into()))
        .unwrap();
    cx.run_until_parked();
    projection(&app, cx, "ready", "old", None, false);
    assert!(calls(&requests).is_empty());
    assert!(cx.debug_bounds("queued-0").is_some());
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.shell.threads[0].runtime_mode = Some("approval-required".into());
            let id = app.queues.messages("t")[0].id;
            app.send_queued("t", id, cx);
        })
    });
    cx.run_until_parked();
    let sent = calls(&requests);
    assert_eq!(sent.len(), 1);
    for key in ["modelSelection", "runtimeMode", "interactionMode"] {
        assert_eq!(sent[0].0[key], original[key]);
    }
    assert_eq!(
        sent[0].0["message"]["attachments"],
        original["message"]["attachments"]
    );
    assert_eq!(sent[0].0["message"]["text"], original["message"]["text"]);
    assert_eq!(
        sent[0].0["message"]["context"],
        original["message"]["context"]
    );
}

#[gpui_kit::test]
fn alternate_queues_with_steer_preference_and_approval_blocks_boundary(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    projection(&app, cx, "running", "old", None, false);
    cx.update(|_, cx| {
        app.update(cx, |app, _| {
            app.follow_up_behavior = t3code_desktop_gpui::queue::FollowUpBehavior::Steer
        })
    });
    cx.simulate_input("later");
    cx.simulate_keystrokes("cmd-shift-enter");
    cx.run_until_parked();
    assert!(calls(&requests).is_empty());
    projection(&app, cx, "running", "old", Some("tool-1"), true);
    assert!(calls(&requests).is_empty());
    projection(&app, cx, "running", "old", Some("tool-1"), false);
    let sent = calls(&requests);
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].0["message"]["text"], "later");
}
