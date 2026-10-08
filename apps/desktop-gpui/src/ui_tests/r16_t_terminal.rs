//! Round-16 terminal input leases, streamed file routes and durable split presentation.
use super::surfaces_terminal::{call, drain_sizes, open};
use super::*;
use gpui_kit::EntityInputHandler;

fn click(cx: &mut VisualTestContext, id: &str) {
    let bounds = cx
        .debug_bounds(Box::leak(id.to_owned().into_boxed_str()))
        .unwrap_or_else(|| panic!("missing {id}"));
    cx.simulate_click(bounds.center(), Default::default());
    cx.run_until_parked();
}

#[gpui_kit::test]
fn r16_t_preedit_keys_and_blurred_native_callbacks_cannot_write_to_pty(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    cx.update(|window, _| window.activate_window());
    open(&app, cx, &requests);
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.replace_and_mark_text_in_range(None, "日本😀", Some(2..4), window, cx);
            assert!(app.accepts_text_input(window, cx));
        })
    });
    cx.simulate_keystrokes("enter up escape ctrl-p ctrl-n ctrl-h");
    cx.run_until_parked();
    assert!(
        requests.is_empty(),
        "candidate-editing keys must not leak to the shell"
    );
    click(cx, "terminal-rename");
    assert!(cx.debug_bounds("terminal-rename-save").is_some());
    drain_sizes(cx, &requests);
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            assert!(app.terminal_drawer.renaming.is_some(), "rename opened");
            assert!(
                !app.terminal_drawer.focus.is_focused(window),
                "rename input owns focus"
            );
            assert_eq!(
                app.marked_text_range(window, cx),
                None,
                "blur retires preedit"
            );
            assert!(!app.accepts_text_input(window, cx));
            app.replace_text_in_range(None, "late native commit", window, cx);
            app.replace_and_mark_text_in_range(None, "late candidate", None, window, cx);
            app.unmark_text(window, cx);
        })
    });
    assert!(
        requests.is_empty(),
        "callbacks after another responder gained focus are discarded"
    );
    click(cx, "terminal-rename-save");
    drain_sizes(cx, &requests);
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.replace_text_in_range(None, "é€😀", window, cx);
            app.unmark_text(window, cx);
        })
    });
    let (payload, reply) = call(&requests, "terminal.write");
    assert_eq!(payload["threadId"], "t");
    assert_eq!(payload["data"], "é€😀");
    reply.send_blocking(Ok(Value::Null)).unwrap();
    assert!(
        requests.is_empty(),
        "returning focus never resurrects abandoned preedit"
    );
    cx.simulate_keystrokes("cmd-j");
    assert!(cx.debug_bounds("terminal-drawer").is_none());
    cx.simulate_keystrokes("cmd-j");
    assert!(cx.debug_bounds("terminal-drawer").is_some());
}

fn stream_path(
    app: &Entity<NativeApp>,
    cx: &mut VisualTestContext,
    stream: &str,
    id: &str,
    path: &str,
    sequence: u64,
) {
    cx.update(|_, cx| app.update(cx, |app, cx| app.terminal_update(stream, Ok(json!({"type":"snapshot","snapshot":{"threadId":"t","terminalId":id,"sequence":sequence,"cwd":"/tmp/selected-checkout/build","status":"running","history":format!("{path}\r\n")}})), cx)));
    cx.run_until_parked();
}
fn modifier_click_path(app: &Entity<NativeApp>, cx: &mut VisualTestContext) {
    let grid = cx.debug_bounds("terminal-grid").unwrap();
    let (width, height) = app.read_with(cx, |app, _| {
        (
            app.terminal_drawer.cell_width,
            app.terminal_drawer.line_height,
        )
    });
    cx.simulate_click(
        point(
            grid.origin.x + px(width * 4.),
            grid.origin.y + px(height * 0.5),
        ),
        gpui_kit::Modifiers {
            platform: cfg!(target_os = "macos"),
            control: !cfg!(target_os = "macos"),
            ..Default::default()
        },
    );
    cx.run_until_parked();
}

#[gpui_kit::test]
fn r16_t_streamed_compiler_path_reads_owned_checkout_and_preserves_dirty_file(
    cx: &mut TestAppContext,
) {
    let (app, cx, requests) = harness(cx);
    let (id, stream) = open(&app, cx, &requests);
    stream_path(&app, cx, &stream, &id, "../src/main.rs:2:3:", 1);
    modifier_click_path(&app, cx);
    let (payload, reply) = call(&requests, "projects.readFile");
    assert_eq!(
        payload,
        json!({"cwd":"/tmp/selected-checkout","relativePath":"src/main.rs"})
    );
    assert_eq!(cx.opened_url(), None, "paths never open an OS file URL");
    reply.send_blocking(Ok(json!({"relativePath":"src/main.rs","contents":"first\nsecond\nthird","truncated":false}))).unwrap();
    cx.run_until_parked();
    app.read_with(cx, |app, cx| {
        assert_eq!(app.file_editor.target.as_ref().unwrap().path, "src/main.rs");
        assert_eq!(
            app.file_contents.read(cx).value().as_str(),
            "first\nsecond\nthird"
        );
        assert!(app.inspector_open);
    });
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.file_contents
                .update(cx, |input, cx| input.set_value("unsaved", window, cx))
        })
    });
    stream_path(&app, cx, &stream, &id, "../src/other.rs:8", 2);
    modifier_click_path(&app, cx);
    assert!(
        requests.is_empty(),
        "dirty-file guard runs before a new read"
    );
    app.read_with(cx, |app, cx| {
        assert_eq!(app.file_contents.read(cx).value().as_str(), "unsaved")
    });
    stream_path(&app, cx, &stream, &id, "/tmp/outside.rs:2", 3);
    modifier_click_path(&app, cx);
    assert!(
        requests.is_empty(),
        "no external-workspace or client-side read"
    );
}

#[gpui_kit::test]
fn r16_t_file_read_denial_and_stale_reply_never_replace_owned_projection(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    let (id, stream) = open(&app, cx, &requests);
    stream_path(&app, cx, &stream, &id, "../src/main.rs:2", 1);
    modifier_click_path(&app, cx);
    let (_, reply) = call(&requests, "projects.readFile");
    reply.send_blocking(Err("read denied".into())).unwrap();
    cx.run_until_parked();
    app.read_with(cx, |app, _| assert!(app.file_editor.target.is_none()));
    modifier_click_path(&app, cx);
    let (_, reply) = call(&requests, "projects.readFile");
    cx.update(|_, cx| app.update(cx, |app, _| app.generation += 1));
    reply
        .send_blocking(Ok(
            json!({"relativePath":"src/main.rs","contents":"stale","truncated":false}),
        ))
        .unwrap();
    cx.run_until_parked();
    app.read_with(cx, |app, _| assert!(app.file_editor.target.is_none()));
}

#[gpui_kit::test]
fn r16_t_reloaded_vertical_group_discovers_attaches_without_reconstructing_launch(
    cx: &mut TestAppContext,
) {
    let root = std::env::temp_dir().join(format!("r16-t-ui-{}", uuid::Uuid::new_v4()));
    let path = root.join("layouts.json");
    let (app, cx, requests) = harness(cx);
    cx.update(|_, cx| {
        app.update(cx, |app, _| {
            app.terminal_drawer.test_layout_storage(path.clone())
        })
    });
    let (first, _) = open(&app, cx, &requests);
    click(cx, "terminal-split-vertical");
    let (payload, reply) = loop {
        let connection::Command::Call {
            tag,
            payload,
            reply,
        } = requests.try_recv().unwrap()
        else {
            panic!("split call");
        };
        if tag == "terminal.open" {
            break (payload, reply);
        }
        assert_eq!(tag, "terminal.resize");
        reply.send_blocking(Ok(Value::Null)).unwrap();
    };
    let second = payload["terminalId"].as_str().unwrap().to_owned();
    reply
        .send_blocking(Ok(
            json!({"threadId":"t","terminalId":second,"history":"second","status":"running"}),
        ))
        .unwrap();
    cx.run_until_parked();
    let connection::Command::Subscribe { key, .. } = requests.try_recv().unwrap() else {
        panic!("attach");
    };
    cx.update(|_, cx| app.update(cx, |app, cx| app.terminal_update(&key, Ok(json!({"type":"snapshot","snapshot":{"threadId":"t","terminalId":second,"history":"second","status":"running"}})), cx)));
    drain_sizes(cx, &requests);
    click(cx, "terminal-hide");
    while let Ok(command) = requests.try_recv() {
        assert!(matches!(command, connection::Command::Unsubscribe(_)));
    }
    // A fresh client drawer reads the actual document written by the UI above.
    // Server PTYs are not invented/restarted: fresh metadata/attach receipts supply them.
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.terminal_drawer = crate::runtime_terminal::Drawer::new(window, cx);
            app.terminal_drawer.test_layout_storage(path.clone());
            cx.notify();
        })
    });
    cx.simulate_keystrokes("cmd-j");
    cx.run_until_parked();
    let connection::Command::Subscribe { key, tag, payload } = requests.try_recv().unwrap() else {
        panic!("discovery");
    };
    assert_eq!(tag, "subscribeTerminalMetadata");
    assert_eq!(payload, json!({}));
    let summaries = [&first, &second].into_iter().map(|id| json!({"threadId":"t","terminalId":id,"cwd":"/tmp/selected-checkout","status":"running","hasRunningSubprocess":false})).collect::<Vec<_>>();
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.terminal_update(
                &key,
                Ok(json!({"type":"snapshot","terminals":summaries})),
                cx,
            )
        })
    });
    cx.run_until_parked();
    let mut attached = Vec::new();
    while let Ok(command) = requests.try_recv() {
        let (key, tag, payload) = match command {
            connection::Command::Subscribe { key, tag, payload } => (key, tag, payload),
            connection::Command::Call {
                tag,
                payload,
                reply,
            } => {
                assert_eq!(
                    tag, "terminal.resize",
                    "restoration never opens or restarts PTYs"
                );
                assert!(payload["terminalId"] == first || payload["terminalId"] == second);
                reply.send_blocking(Ok(Value::Null)).unwrap();
                continue;
            }
            _ => panic!("restoration only attaches/resizes existing PTYs"),
        };
        assert_eq!(tag, "terminal.attach");
        assert_eq!(payload["restartIfNotRunning"], false);
        let id = payload["terminalId"].as_str().unwrap().to_owned();
        assert!(id == first || id == second);
        attached.push(id.clone());
        cx.update(|_, cx| app.update(cx, |app, cx| app.terminal_update(&key, Ok(json!({"type":"snapshot","snapshot":{"threadId":"t","terminalId":id,"history":"restored","status":"running"}})), cx)));
    }
    attached.sort();
    let mut expected = vec![first.clone(), second.clone()];
    expected.sort();
    assert_eq!(attached, expected);
    drain_sizes(cx, &requests);
    app.read_with(cx, |app, _| {
        assert_eq!(app.terminal_drawer.active.as_deref(), Some(second.as_str()));
        assert!(app.terminal_drawer.vertical);
        assert_eq!(
            app.terminal_drawer.split.as_ref().unwrap().2,
            vec![first.clone(), second.clone()]
        );
        assert!(
            app.terminal_drawer
                .tabs
                .iter()
                .all(|tab| tab.launch.is_null())
        );
    });
    click(cx, "terminal-restart");
    assert!(
        requests.is_empty(),
        "layout is not authoritative launch configuration"
    );
    assert!(cx.debug_bounds("terminal-confirm-restart").is_none());
    std::fs::remove_dir_all(root).unwrap();
}

#[gpui_kit::test]
fn r16_t_vertical_groups_keep_independent_pty_geometry_and_survive_new_tab_hide(
    cx: &mut TestAppContext,
) {
    let (app, cx, requests) = harness(cx);
    let (first, _) = open(&app, cx, &requests);
    let full = cx.debug_bounds("terminal-grid").unwrap();
    click(cx, "terminal-split-vertical");
    let (payload, reply) = loop {
        let connection::Command::Call {
            tag,
            payload,
            reply,
        } = requests.try_recv().unwrap()
        else {
            panic!("split call");
        };
        if tag == "terminal.open" {
            break (payload, reply);
        }
        assert_eq!(tag, "terminal.resize");
        assert_eq!(payload["terminalId"], first);
        reply.send_blocking(Ok(Value::Null)).unwrap();
    };
    let second = payload["terminalId"].as_str().unwrap().to_owned();
    reply
        .send_blocking(Ok(
            json!({"threadId":"t","terminalId":second,"history":"second", "status":"running"}),
        ))
        .unwrap();
    cx.run_until_parked();
    let connection::Command::Subscribe { key, tag, payload } = requests.try_recv().unwrap() else {
        panic!("attach");
    };
    assert_eq!(tag, "terminal.attach");
    assert_eq!(payload["terminalId"], second);
    cx.update(|_, cx| app.update(cx, |app, cx| app.terminal_update(&key, Ok(json!({"type":"snapshot","snapshot":{"threadId":"t","terminalId":second,"history":"second", "status":"running"}})), cx)));
    drain_sizes(cx, &requests);
    let lower = cx.debug_bounds("terminal-grid").unwrap();
    let upper = cx
        .debug_bounds(Box::leak(format!("terminal-grid-{first}").into_boxed_str()))
        .unwrap();
    assert!(lower.origin.y > upper.origin.y);
    assert!(lower.size.height < full.size.height);
    assert!(upper.size.width > full.size.width * 0.8);
    click(cx, "terminal-new");
    let (payload, reply) = loop {
        match requests.try_recv().unwrap() {
            connection::Command::Call {
                tag,
                payload,
                reply,
            } if tag == "terminal.open" => break (payload, reply),
            connection::Command::Unsubscribe(_) => {}
            connection::Command::Call { tag, reply, .. } => {
                assert_eq!(tag, "terminal.resize");
                reply.send_blocking(Ok(Value::Null)).unwrap();
            }
            _ => panic!("new tab"),
        }
    };
    let third = payload["terminalId"].as_str().unwrap().to_owned();
    reply
        .send_blocking(Ok(
            json!({"threadId":"t","terminalId":third,"history":"third", "status":"running"}),
        ))
        .unwrap();
    cx.run_until_parked();
    assert!(
        cx.debug_bounds(Box::leak(format!("terminal-grid-{first}").into_boxed_str()))
            .is_none()
    );
    click(cx, &format!("terminal-tab-{first}"));
    app.read_with(cx, |app, _| {
        assert!(app.terminal_drawer.vertical);
        assert_eq!(
            app.terminal_drawer.split.as_ref().unwrap().2,
            vec![first.clone(), second.clone()]
        );
    });
    assert!(
        cx.debug_bounds(Box::leak(
            format!("terminal-grid-{second}").into_boxed_str()
        ))
        .is_some()
    );
    click(cx, "terminal-hide");
    cx.simulate_keystrokes("cmd-j");
    app.read_with(cx, |app, _| assert!(app.terminal_drawer.vertical));
    click(cx, "terminal-unsplit");
    app.read_with(cx, |app, _| {
        assert!(app.terminal_drawer.split.is_none());
        assert_eq!(
            app.terminal_drawer.tabs.len(),
            3,
            "unsplit is presentation-only"
        );
    });
    while let Ok(command) = requests.try_recv() {
        assert!(
            !matches!(command, connection::Command::Call { tag, .. } if tag == "terminal.close" || tag == "terminal.write" || tag == "terminal.restart")
        );
    }
}
