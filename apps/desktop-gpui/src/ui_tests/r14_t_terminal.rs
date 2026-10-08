//! Round-14 terminal safety and input acceptance through real UI + Command receipts.
use super::surfaces_terminal::{call, drain_sizes, open};
use super::*;
use gpui_kit::EntityInputHandler;
use std::collections::HashMap;

fn click(cx: &mut VisualTestContext, id: &'static str) {
    let bounds = cx
        .debug_bounds(id)
        .unwrap_or_else(|| panic!("missing {id}"));
    cx.simulate_click(bounds.center(), Default::default());
    cx.run_until_parked();
}
fn metadata(requests: &Requests) -> String {
    let connection::Command::Subscribe { key, tag, payload } = requests.try_recv().unwrap() else {
        panic!("metadata")
    };
    assert_eq!(tag, "subscribeTerminalMetadata");
    assert_eq!(payload, json!({}));
    key
}
fn summary(thread: &str, id: &str) -> Value {
    json!({"threadId":thread,"terminalId":id,"cwd":"/tmp/restored","status":"running","hasRunningSubprocess":true,"label":id})
}

#[gpui_kit::test]
fn r14_t_restart_confirmation_cannot_cross_same_id_thread_environment_or_generation(
    cx: &mut TestAppContext,
) {
    let (app, cx, requests) = harness(cx);
    let (id, _) = open(&app, cx, &requests);
    click(cx, "terminal-restart");
    drain_sizes(cx, &requests);
    assert!(cx.debug_bounds("terminal-confirm-restart").is_some());
    let captured = app.read_with(cx, |app, _| {
        app.terminal_drawer.restart_confirm.clone().unwrap()
    });
    assert_eq!(
        (captured.1.as_str(), captured.2.as_str()),
        ("t", id.as_str())
    );
    click(cx, "terminal-discover");
    let key = metadata(&requests);
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.selected_thread = Some("u".into());
            app.terminal_update(
                &key,
                Ok(json!({"type":"upsert","terminal":summary("u", &id)})),
                cx,
            );
        })
    });
    cx.run_until_parked();
    assert!(cx.debug_bounds("terminal-confirm-restart").is_none());
    app.read_with(cx, |app, _| {
        assert!(app.terminal_drawer.restart_confirm.is_none())
    });
    while let Ok(command) = requests.try_recv() {
        assert!(
            !matches!(command, connection::Command::Call { tag, .. } if tag == "terminal.restart")
        );
    }
    for change_environment in [false, true] {
        cx.update(|_, cx| {
            app.update(cx, |app, cx| {
                app.selected_thread = Some("t".into());
                app.terminal_drawer.restart_confirm = Some(captured.clone());
                if change_environment {
                    app.environment_scope = "another-environment".into();
                } else {
                    app.generation += 1;
                }
                app.terminal_synchronize(cx);
            })
        });
        app.read_with(cx, |app, _| {
            assert!(app.terminal_drawer.restart_confirm.is_none())
        });
        while let Ok(command) = requests.try_recv() {
            assert!(
                !matches!(command, connection::Command::Call { tag, .. } if tag == "terminal.restart")
            );
        }
    }
}

#[gpui_kit::test]
fn r14_t_metadata_only_restart_is_disabled_and_names_do_not_expose_internal_ids(
    cx: &mut TestAppContext,
) {
    let (app, cx, requests) = harness(cx);
    let (id, stream) = open(&app, cx, &requests);
    cx.update(|_, cx| app.update(cx, |app, cx| app.terminal_update(&stream, Ok(json!({"type":"snapshot","snapshot":{"threadId":"t","terminalId":id,"history":"prompt","status":"running","label":id,"sequence":1}})), cx)));
    app.read_with(cx, |app, _| {
        assert_eq!(app.terminal_drawer.tabs[0].title, "Terminal 1")
    });
    click(cx, "terminal-discover");
    let key = metadata(&requests);
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.terminal_update(
                &key,
                Ok(json!({"type":"snapshot","terminals":[summary("t", "drawer-restored-id")]})),
                cx,
            )
        })
    });
    cx.run_until_parked();
    assert!(
        matches!(requests.try_recv().unwrap(), connection::Command::Unsubscribe(key) if key == stream)
    );
    let connection::Command::Subscribe {
        key: attach,
        tag,
        payload,
    } = requests.try_recv().unwrap()
    else {
        panic!("attach")
    };
    assert_eq!(tag, "terminal.attach");
    assert_eq!(payload["restartIfNotRunning"], false);
    cx.update(|_, cx| app.update(cx, |app, cx| app.terminal_update(&attach, Ok(json!({"type":"snapshot","snapshot":{"threadId":"t","terminalId":"drawer-restored-id","history":"original account shell","status":"running","label":"drawer-restored-id"}})), cx)));
    drain_sizes(cx, &requests);
    app.read_with(cx, |app, _| {
        assert_eq!(app.terminal_drawer.tabs[0].title, "Terminal 1");
        assert!(app.terminal_drawer.tabs[0].launch.is_null());
    });
    click(cx, "terminal-restart");
    assert!(cx.debug_bounds("terminal-confirm-restart").is_none());
    assert!(
        requests.is_empty(),
        "unknown env must never synthesize a restart"
    );
    click(cx, "terminal-rename");
    cx.simulate_keystrokes("cmd-a");
    cx.simulate_input("Work account");
    click(cx, "terminal-rename-save");
    drain_sizes(cx, &requests);
    app.read_with(cx, |app, _| {
        assert_eq!(app.terminal_drawer.tabs[0].title, "Work account")
    });
}

#[gpui_kit::test]
fn r14_t_metadata_replacement_keeps_cancellation_owner_when_channel_is_full(
    cx: &mut TestAppContext,
) {
    let (app, cx, requests) = harness(cx);
    open(&app, cx, &requests);
    click(cx, "terminal-discover");
    let original = metadata(&requests);
    let sender = app.read_with(cx, |app, _| app.commands.clone().unwrap());
    for index in 0..16 {
        sender
            .try_send(connection::Command::Unsubscribe(format!("filler-{index}")))
            .unwrap();
    }
    click(cx, "terminal-discover");
    app.read_with(cx, |app, _| {
        assert!(
            app.terminal_drawer
                .local_error
                .as_ref()
                .unwrap()
                .contains("discovery failed")
        )
    });
    for _ in 0..16 {
        assert!(matches!(
            requests.try_recv().unwrap(),
            connection::Command::Unsubscribe(_)
        ));
    }
    click(cx, "terminal-discover");
    let replacement = metadata(&requests);
    assert_eq!(
        original, replacement,
        "transport replaces/interrupts the same owned key"
    );
    drain_sizes(cx, &requests);
    assert!(requests.is_empty(), "no unowned second subscription");
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.terminal_update(
                &replacement,
                Ok(json!({"type":"snapshot","terminals":[]})),
                cx,
            )
        })
    });
    cx.run_until_parked();
    while let Ok(command) = requests.try_recv() {
        assert!(matches!(command, connection::Command::Unsubscribe(_)));
    }
}

#[gpui_kit::test]
fn r14_t_many_thread_discovery_and_removal_bounds_screens_and_unsubscribes_output(
    cx: &mut TestAppContext,
) {
    let (app, cx, requests) = harness(cx);
    open(&app, cx, &requests);
    click(cx, "terminal-discover");
    let key = metadata(&requests);
    let mut subscriptions = HashMap::new();
    for index in 0..12 {
        let thread = format!("thread-{index}");
        let terminals = (0..40)
            .map(|n| summary(&thread, &format!("term-{n}")))
            .collect::<Vec<_>>();
        cx.update(|_, cx| {
            app.update(cx, |app, cx| {
                app.selected_thread = Some(thread.clone());
                app.terminal_update(
                    &key,
                    Ok(json!({"type":"snapshot","terminals":terminals})),
                    cx,
                );
            })
        });
        cx.run_until_parked();
        while let Ok(command) = requests.try_recv() {
            match command {
                connection::Command::Subscribe { key, tag, payload } => {
                    assert_eq!(tag, "terminal.attach");
                    assert_eq!(payload["threadId"], thread);
                    assert_eq!(payload["restartIfNotRunning"], false);
                    subscriptions.insert(key, payload);
                }
                connection::Command::Unsubscribe(key) => {
                    subscriptions.remove(&key);
                }
                connection::Command::Call { tag, reply, .. } => {
                    assert_eq!(tag, "terminal.resize");
                    reply.send_blocking(Ok(Value::Null)).unwrap();
                }
                _ => panic!("no PTY open or restart during discovery"),
            }
        }
        assert!(subscriptions.len() <= 16);
        app.read_with(cx, |app, _| assert!(app.terminal_drawer.tabs.len() <= 16));
        // Deliver actual attach snapshots/output, not directly installed grids.
        for (stream, payload) in &subscriptions {
            cx.update(|_, cx| app.update(cx, |app, cx| app.terminal_update(stream, Ok(json!({"type":"snapshot","snapshot":{"threadId":payload["threadId"],"terminalId":payload["terminalId"],"history":"retained output\r\n".repeat(20),"status":"running"}})), cx)));
        }
        cx.run_until_parked();
        while let Ok(command) = requests.try_recv() {
            let connection::Command::Call { tag, reply, .. } = command else {
                panic!("resize")
            };
            assert_eq!(tag, "terminal.resize");
            reply.send_blocking(Ok(Value::Null)).unwrap();
        }
    }
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.terminal_update(&key, Ok(json!({"type":"snapshot","terminals":[]})), cx)
        })
    });
    cx.run_until_parked();
    while let Ok(command) = requests.try_recv() {
        let connection::Command::Unsubscribe(key) = command else {
            panic!("retire")
        };
        subscriptions.remove(&key);
    }
    assert!(subscriptions.is_empty());
    app.read_with(cx, |app, _| {
        assert!(
            app.terminal_drawer.tabs.is_empty(),
            "removed grids must be freed"
        )
    });
}

#[gpui_kit::test]
fn r14_t_split_panes_resize_focus_write_and_unsplit_without_closing_sessions(
    cx: &mut TestAppContext,
) {
    let (app, cx, requests) = harness(cx);
    let (first, _) = open(&app, cx, &requests);
    let full_width = cx.debug_bounds("terminal-grid").unwrap().size.width;
    click(cx, "terminal-split");
    // The existing pane resizes as soon as the split is painted, before the
    // async open call is scheduled; both operations are owned and observable.
    let (payload, reply) = loop {
        let connection::Command::Call {
            tag,
            payload,
            reply,
        } = requests.try_recv().unwrap()
        else {
            panic!("split call")
        };
        if tag == "terminal.open" {
            break (payload, reply);
        }
        assert_eq!(tag, "terminal.resize");
        assert_eq!(payload["terminalId"], first);
        reply.send_blocking(Ok(Value::Null)).unwrap();
        cx.run_until_parked();
    };
    let second = payload["terminalId"].as_str().unwrap().to_owned();
    reply
        .send_blocking(Ok(
            json!({"threadId":"t","terminalId":second,"history":"second> ","status":"running"}),
        ))
        .unwrap();
    cx.run_until_parked();
    let connection::Command::Subscribe { key, tag, payload } = requests.try_recv().unwrap() else {
        panic!("attach split")
    };
    assert_eq!(tag, "terminal.attach");
    assert_eq!(payload["terminalId"], second);
    cx.update(|_, cx| app.update(cx, |app, cx| app.terminal_update(&key, Ok(json!({"type":"snapshot","snapshot":{"threadId":"t","terminalId":second,"history":"second> ","status":"running"}})), cx)));
    drain_sizes(cx, &requests);
    let active = cx.debug_bounds("terminal-grid").unwrap();
    assert!(active.size.width < full_width * 0.6);
    let first_bounds = app.read_with(cx, |app, _| {
        app.terminal_drawer
            .tabs
            .iter()
            .find(|tab| tab.id == first)
            .unwrap()
            .bounds
            .unwrap()
    });
    assert!(first_bounds.origin.x < active.origin.x);
    cx.simulate_click(first_bounds.center(), Default::default());
    cx.run_until_parked();
    drain_sizes(cx, &requests);
    cx.update(|window, cx| {
        app.read_with(cx, |app, _| {
            assert_eq!(
                app.terminal_drawer.active.as_deref(),
                Some(first.as_str()),
                "pane click must select its terminal"
            );
            assert!(
                app.terminal_drawer.focus.is_focused(window),
                "pane click must keep terminal focus"
            );
            let tab = app
                .terminal_drawer
                .tabs
                .iter()
                .find(|tab| tab.id == first)
                .unwrap();
            assert!(
                tab.stream_ready && tab.attached == Some(app.generation),
                "pane click must preserve first attachment"
            );
        })
    });
    cx.simulate_input("猫");
    let (payload, reply) = call(&requests, "terminal.write");
    assert_eq!(
        payload,
        json!({"threadId":"t","terminalId":first,"data":"猫"})
    );
    reply.send_blocking(Ok(Value::Null)).unwrap();
    click(cx, "terminal-unsplit");
    assert!(
        matches!(requests.try_recv().unwrap(), connection::Command::Unsubscribe(stream) if stream == key),
        "unsplit must retire the hidden pane's output without closing its PTY"
    );
    drain_sizes(cx, &requests);
    assert_eq!(
        cx.debug_bounds("terminal-grid").unwrap().size.width,
        full_width
    );
    app.read_with(cx, |app, _| assert_eq!(app.terminal_drawer.tabs.len(), 2));
    assert!(requests.is_empty());
}

#[gpui_kit::test]
fn r14_t_ime_cjk_candidate_bounds_commit_cancel_and_same_id_scope_fence(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    open(&app, cx, &requests);
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.replace_and_mark_text_in_range(None, "に😀", Some(1..3), window, cx);
            assert_eq!(app.marked_text_range(window, cx), Some(0..3));
            let bounds = app.terminal_drawer.bounds.unwrap();
            let candidate = app.bounds_for_range(1..3, bounds, window, cx).unwrap();
            assert!(candidate.origin.x >= bounds.origin.x && candidate.origin.x < bounds.right());
            assert!(candidate.origin.y >= bounds.origin.y && candidate.origin.y < bounds.bottom());
            app.replace_and_mark_text_in_range(Some(0..1), "日本", Some(0..2), window, cx);
            assert_eq!(app.marked_text_range(window, cx), Some(0..4));
            app.replace_text_in_range(Some(0..4), "日本😀", window, cx);
            app.unmark_text(window, cx);
        })
    });
    let (payload, reply) = call(&requests, "terminal.write");
    assert_eq!(payload["data"], "日本😀");
    reply.send_blocking(Ok(Value::Null)).unwrap();
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.replace_and_mark_text_in_range(None, "cancel", None, window, cx);
            app.replace_and_mark_text_in_range(None, "", None, window, cx);
            app.unmark_text(window, cx);
            app.replace_and_mark_text_in_range(None, "old thread", None, window, cx);
            app.terminal_drawer.tabs[0].thread = "u".into();
            app.selected_thread = Some("u".into());
            app.replace_text_in_range(None, "must not commit", window, cx);
            app.unmark_text(window, cx);
        })
    });
    assert!(
        requests.is_empty(),
        "cancellation and a same-id other thread never commit preedit"
    );
}
