use super::*;

#[gpui_kit::test]
fn r15_m_background_queue_send_and_checkout_stop_keep_owning_command_channel(
    cx: &mut TestAppContext,
) {
    let (office, lab, cx) = pair(cx);
    cx.update(|window, cx| {
        office.app.update(cx, |app, cx| {
            app.shell.threads[0].session = Some(shell::Session {
                status: "running".into(),
                last_error: None,
            });
            app.composer
                .update(cx, |input, cx| input.set_value("office queued", window, cx));
            app.send_prompt(window, cx);
        })
    });
    cx.run_until_parked();
    let connection::Command::Subscribe {
        key: queue_key,
        tag,
        ..
    } = office.requests.try_recv().unwrap()
    else {
        panic!("queue subscription");
    };
    assert_eq!(tag, "orchestration.subscribeThread");
    for item in [
        json!({"kind":"snapshot","snapshot":{"snapshotSequence":1,"thread":{"id":"t","messages":[],"activities":[],"session":{"status":"running"}}}}),
        json!({"kind":"synchronized"}),
    ] {
        office
            .updates
            .try_send(connection::Update::Subscription {
                key: queue_key.clone(),
                item: Ok(item),
            })
            .unwrap();
    }
    cx.run_until_parked();
    let queued = office
        .app
        .read_with(cx, |app, _| app.queues.messages("t")[0].id);
    switch(&office, LAB, cx);
    assert_eq!(
        office
            .app
            .read_with(cx, |app, _| app.queues.messages("t")[0].text.clone()),
        "office queued"
    );
    assert!(
        lab.app
            .read_with(cx, |app, _| app.queues.messages("t").is_empty())
    );
    cx.update(|_, cx| {
        office
            .app
            .update(cx, |app, cx| app.send_queued("t", queued, cx))
    });
    cx.run_until_parked();
    let (payload, reply) = call(&office.requests, "orchestration.dispatchCommand");
    assert_eq!(payload["type"], "thread.turn.start");
    assert_eq!(payload["threadId"], "t");
    assert_eq!(payload["message"]["text"], "office queued");
    assert!(lab.requests.is_empty());
    reply.try_send(Ok(json!({"sequence":2}))).unwrap();
    cx.run_until_parked();
    assert!(
        office
            .app
            .read_with(cx, |app, _| app.queues.messages("t").is_empty())
    );
    while let Ok(command) = office.requests.try_recv() {
        assert!(matches!(command, connection::Command::Unsubscribe(_)));
    }
    let stop = cx.update(|_, cx| office.app.update(cx, |app, cx| app.test_checkout_stop(cx)));
    cx.run_until_parked();
    let connection::Command::Subscribe { key, tag, payload } = office.requests.try_recv().unwrap()
    else {
        panic!("checkout stop subscription");
    };
    assert_eq!(tag, "orchestration.subscribeThread");
    assert_eq!(payload["threadId"], "t");
    for item in [
        json!({"kind":"snapshot","supportsCorrelatedSessionStops":true,"snapshot":{"snapshotSequence":5,"thread":{"id":"t","messages":[],"activities":[],"session":{"status":"stopped","updatedAt":"old"}}}}),
        json!({"kind":"synchronized"}),
    ] {
        office
            .updates
            .try_send(connection::Update::Subscription {
                key: key.clone(),
                item: Ok(item),
            })
            .unwrap();
    }
    cx.run_until_parked();
    let connection::Command::CallMutation {
        tag,
        payload,
        reply,
        ..
    } = office.requests.try_recv().unwrap()
    else {
        panic!("stop mutation");
    };
    assert_eq!(tag, "orchestration.dispatchCommand");
    assert_eq!(payload["type"], "thread.session.stop");
    let command_id = payload["commandId"].as_str().unwrap();
    reply.try_send(Ok(json!({"sequence":7}))).unwrap();
    cx.run_until_parked();
    assert!(stop.try_recv().is_err());
    // Delivering OFFICE's correlated stop on LAB must not complete OFFICE's lease.
    let event = json!({"kind":"event","event":{"sequence":9,"type":"thread.session-set","occurredAt":"2026-01-01T00:01:00.123Z","payload":{"threadId":"t","session":{"status":"stopped","updatedAt":"2026-01-01T00:01:00.123Z","activeTurnId":null,"stopCommandId":command_id}}}});
    lab.updates
        .try_send(connection::Update::Subscription {
            key: key.clone(),
            item: Ok(event.clone()),
        })
        .unwrap();
    cx.run_until_parked();
    assert!(stop.try_recv().is_err());
    office
        .updates
        .try_send(connection::Update::Subscription {
            key: key.clone(),
            item: Ok(event),
        })
        .unwrap();
    cx.run_until_parked();
    assert_eq!(stop.try_recv().unwrap(), Ok(()));
    assert!(
        matches!(office.requests.try_recv().unwrap(), connection::Command::Unsubscribe(k) if k == key)
    );
    assert!(lab.requests.is_empty());
    assert!(
        lab.app
            .read_with(cx, |app, _| app.environment_is_selected())
    );
}

#[gpui_kit::test]
fn r15_m_background_terminal_stream_and_browser_tabs_are_retained(cx: &mut TestAppContext) {
    let (office, lab, cx) = pair(cx);
    let (_, stream) = super::surfaces_terminal::open(&office.app, cx, &office.requests);
    let browser = cx.debug_bounds("browser-add").unwrap();
    cx.simulate_click(browser.center(), Default::default());
    cx.run_until_parked();
    let tabs = office
        .app
        .read_with(cx, |app, cx| app.browser_panel.read(cx).tabs.entries.len());
    switch(&office, LAB, cx);
    office.updates.try_send(connection::Update::Subscription {
        key: stream, item: Ok(json!({"type":"snapshot","snapshot":{"threadId":"t","terminalId":office.app.read_with(cx, |app, _| app.terminal_drawer.tabs[0].id.clone()),"history":"background output","status":"running","sequence":1}}))
    }).unwrap();
    cx.run_until_parked();
    office.app.read_with(cx, |app, cx| {
        assert_eq!(app.browser_panel.read(cx).tabs.entries.len(), tabs);
        assert_eq!(
            app.terminal_drawer.tabs[0]
                .screen
                .cells()
                .iter()
                .take(17)
                .map(|cell| cell.text.as_str())
                .collect::<String>(),
            "background output"
        );
    });
    lab.app.read_with(cx, |app, cx| {
        assert!(app.terminal_drawer.tabs.is_empty());
        assert!(app.browser_panel.read(cx).tabs.entries.is_empty());
    });
    switch(&lab, OFFICE, cx);
    assert!(cx.debug_bounds("browser-tab-1").is_some());
    assert!(office.app.read_with(cx, |app, _| app.terminal_drawer.open));
    super::surfaces_terminal::drain_sizes(cx, &office.requests);
    assert!(lab.requests.is_empty());
}

#[gpui_kit::test]
fn r15_m_background_retirement_guards_dirty_state_and_does_not_disconnect_selection(
    cx: &mut TestAppContext,
) {
    let (office, lab, cx) = pair(cx);
    switch(&office, LAB, cx);
    cx.update(|window, cx| {
        office.app.update(cx, |app, cx| {
            app.file_editor.loaded(
                "/tmp/fixture".into(),
                files::Read {
                    relative_path: "notes.txt".into(),
                    contents: "saved".into(),
                    truncated: false,
                },
            );
            app.file_contents
                .update(cx, |input, cx| input.set_value("dirty", window, cx));
        })
    });
    let allowed = cx.update(|_, cx| {
        lab.app
            .update(cx, |app, cx| app.can_retire_environment(OFFICE, cx))
    });
    assert!(!allowed);
    assert!(office.app.read_with(cx, |app, _| app.connection_ready()));
    cx.update(|window, cx| {
        office.app.update(cx, |app, cx| {
            app.file_contents
                .update(cx, |input, cx| input.set_value("saved", window, cx))
        })
    });
    cx.update(|_, cx| {
        lab.app.update(cx, |app, cx| {
            assert!(app.can_retire_environment(OFFICE, cx));
            app.retire_environment_page(OFFICE, cx);
        })
    });
    cx.run_until_parked();
    assert!(office.app.read_with(cx, |app, _| app.commands.is_none()));
    assert!(office.updates.is_closed());
    lab.app.read_with(cx, |app, _| {
        assert!(app.connection_ready());
        assert!(app.environment_is_selected());
    });
    assert!(lab.requests.is_empty());
}

const OFFICE: &str = "https://office.example";
const LAB: &str = "https://lab.example";

fn config(id: &str) -> Value {
    json!({"environment":{"environmentId":id,"label":id},"providers":[],"settings":{}})
}
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
        panic!("expected {expected}");
    };
    assert_eq!(tag, expected);
    (payload, reply)
}
struct Live {
    app: Entity<NativeApp>,
    requests: Requests,
    updates: async_channel::Sender<connection::Update>,
}
fn connect(app: Entity<NativeApp>, cx: &mut VisualTestContext, id: &str) -> Live {
    let (commands, requests) = async_channel::bounded(64);
    let (updates, receiver) = async_channel::bounded(64);
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.disconnect(cx);
            app.start_owned_session(
                connection::Session {
                    commands,
                    updates: receiver,
                },
                cx,
            );
        })
    });
    cx.run_until_parked();
    let requests = Requests::from(requests);
    let (payload, reply) = call(&requests, "server.getConfig");
    assert_eq!(payload, json!({}));
    reply.try_send(Ok(config(id))).unwrap();
    cx.run_until_parked();
    updates.try_send(connection::Update::Shell(Box::new(serde_json::from_value(json!({"kind":"snapshot","snapshot":{"snapshotSequence":0,"projects":[{"id":"p","title":"Repository","workspaceRoot":"/tmp/fixture","repositoryIdentity":{"canonicalKey":"github:acme/repo"}}],"threads":[{"id":"t","projectId":"p","title":format!("{id} thread"),"modelSelection":{"instanceId":"codex","model":"gpt-5.4"},"runtimeMode":"full-access","interactionMode":"default","archivedAt":null}]}})).unwrap()))).unwrap();
    updates
        .try_send(connection::Update::Shell(Box::new(
            shell::ShellItem::Synchronized,
        )))
        .unwrap();
    cx.run_until_parked();
    let connection::Command::Subscribe { tag, payload, .. } = requests.try_recv().unwrap() else {
        panic!("server config subscription");
    };
    assert_eq!(tag, "subscribeServerConfig");
    assert_eq!(payload, json!({"usageLimitSources":true}));
    let (_, reply) = call(&requests, "server.getConfig");
    reply.try_send(Ok(config(id))).unwrap();
    cx.run_until_parked();
    cx.update(|window, cx| app.update(cx, |app, cx| app.select_thread("t", window, cx)));
    cx.run_until_parked();
    assert!(
        matches!(requests.try_recv().unwrap(), connection::Command::Subscribe { tag, .. } if tag == "subscribeWorktreeSetup")
    );
    assert!(
        matches!(requests.try_recv().unwrap(), connection::Command::SelectThread(Some(id)) if id == "t")
    );
    assert!(requests.is_empty());
    Live {
        app,
        requests,
        updates,
    }
}
fn pair(cx: &mut TestAppContext) -> (Live, Live, &mut VisualTestContext) {
    let (app, cx, _) = harness(cx);
    let lab = cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.environment_scope = OFFICE.into();
            app.server
                .update(cx, |input, cx| input.set_value(OFFICE, window, cx));
            app.save_environment(cx);
            app.server
                .update(cx, |input, cx| input.set_value(LAB, window, cx));
            app.save_environment(cx);
            app.server
                .update(cx, |input, cx| input.set_value(OFFICE, window, cx));
            app.create_environment_page(LAB, window, cx)
        })
    });
    let office = connect(app, cx, "office-id");
    let lab = connect(lab, cx, "lab-id");
    (office, lab, cx)
}
fn switch(from: &Live, origin: &str, cx: &mut VisualTestContext) {
    cx.update(|window, cx| {
        from.app
            .update(cx, |app, cx| app.switch_environment(origin, window, cx))
    });
    cx.run_until_parked();
}
#[gpui_kit::test]
fn r15_m_two_sessions_switch_preserves_drafts_dirty_files_and_background_receipts(
    cx: &mut TestAppContext,
) {
    let (office, lab, cx) = pair(cx);
    let epoch = office.app.read_with(cx, |app, _| app.generation);
    cx.update(|window, cx| {
        office.app.update(cx, |app, cx| {
            app.composer
                .update(cx, |input, cx| input.set_value("office draft", window, cx));
            app.open_pull_requests(window, cx);
        })
    });
    cx.run_until_parked();
    let (payload, receipt) = call(&office.requests, "pullRequests.list");
    assert_eq!(
        payload,
        json!({"state":"open","involvement":"all","limit":50})
    );
    switch(&office, LAB, cx);
    cx.update(|window, cx| {
        lab.app.update(cx, |app, cx| {
            app.composer
                .update(cx, |input, cx| input.set_value("lab draft", window, cx));
            app.attachments.push(json!({"id":"lab-owned"}));
        })
    });
    receipt.try_send(Ok(json!({"entries":[{"projectId":"p","host":"github.com","repository":"acme/repo","number":7,"title":"Office PR","state":"open"}],"errors":[],"providers":[],"truncated":false,"nextCursors":{}}))).unwrap();
    office.updates.try_send(connection::Update::Shell(Box::new(shell::ShellItem::ProjectUpserted { sequence: 1, project: serde_json::from_value(json!({"id":"p","title":"Background update","workspaceRoot":"/tmp/fixture","repositoryIdentity":{"canonicalKey":"github:acme/repo"}})).unwrap() }))).unwrap();
    cx.run_until_parked();
    office.app.read_with(cx, |app, cx| {
        assert_eq!(app.generation, epoch);
        assert!(app.commands.is_some());
        assert_eq!(app.shell.projects[0].title, "Background update");
        assert_eq!(app.pull_requests.list_rows[0]["title"], "Office PR");
        assert_eq!(app.composer.read(cx).value().as_str(), "office draft");
    });
    lab.app.read_with(cx, |app, cx| {
        assert_eq!(app.composer.read(cx).value().as_str(), "lab draft");
        assert!(app.pull_requests.list.is_null());
        assert_eq!(app.attachments[0]["id"], "lab-owned");
    });
    switch(&lab, OFFICE, cx);
    assert!(office.requests.is_empty());
    assert!(lab.requests.is_empty());
    cx.update(|window, cx| {
        office.app.update(cx, |app, cx| {
            app.file_editor.loaded(
                "/tmp/fixture".into(),
                files::Read {
                    relative_path: "notes.txt".into(),
                    contents: "saved".into(),
                    truncated: false,
                },
            );
            app.file_contents
                .update(cx, |input, cx| input.set_value("dirty", window, cx));
            app.switch_environment(LAB, window, cx);
        })
    });
    office.app.read_with(cx, |app, cx| {
        assert!(app.environment_is_selected());
        assert_eq!(app.file_contents.read(cx).value().as_str(), "dirty");
        assert!(app.status.contains("Unsaved file edits"));
    });
}
#[gpui_kit::test]
fn r15_m_background_upload_receipt_and_reconnect_do_not_touch_other_environment(
    cx: &mut TestAppContext,
) {
    let (office, lab, cx) = pair(cx);
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("notes.txt");
    std::fs::write(&path, b"attachment body").unwrap();
    cx.update(|_, cx| {
        office.app.update(cx, |app, cx| {
            let scope = app.attachment_scope().unwrap();
            app.attach_paths(scope, Some(vec![path]), cx);
        })
    });
    cx.run_until_parked();
    let (payload, receipt) = call(&office.requests, "attachments.createUploadUrl");
    assert_eq!(payload["sizeBytes"], 15);
    receipt.try_send(Ok(json!({"attachmentId":"pending-00000000-0000-4000-8000-000000000001-txt","relativeUrl":"/api/attachments/upload/claims.signature"}))).unwrap();
    cx.run_until_parked();
    let connection::Command::Upload {
        bytes, url, reply, ..
    } = office.requests.try_recv().unwrap()
    else {
        panic!("upload bytes");
    };
    assert_eq!(bytes, b"attachment body");
    assert!(url.starts_with(OFFICE));
    switch(&office, LAB, cx);
    reply.try_send(Ok(())).unwrap();
    cx.run_until_parked();
    assert_eq!(
        office
            .app
            .read_with(cx, |app, _| app.attachments[0]["id"].clone()),
        "pending-00000000-0000-4000-8000-000000000001-txt"
    );
    assert!(lab.app.read_with(cx, |app, _| app.attachments.is_empty()));
    let lab_epoch = lab.app.read_with(cx, |app, _| app.generation);
    cx.update(|_, cx| {
        office.app.update(cx, |app, cx| {
            app.perform("server.getConfig", json!({}), None, cx)
        })
    });
    cx.run_until_parked();
    let (_, stale) = call(&office.requests, "server.getConfig");
    office
        .updates
        .try_send(connection::Update::Resynchronizing)
        .unwrap();
    cx.run_until_parked();
    stale.try_send(Ok(json!({"environment":{"environmentId":"office-id","label":"STALE"},"providers":[],"settings":{}}))).unwrap();
    cx.run_until_parked();
    office.app.read_with(cx, |app, _| {
        assert_ne!(app.environment_name.as_deref(), Some("STALE"))
    });
    office.updates.try_send(connection::Update::Shell(Box::new(serde_json::from_value(json!({"kind":"snapshot","snapshot":{"snapshotSequence":1,"projects":[{"id":"p","title":"Fixture","workspaceRoot":"/tmp/fixture"}],"threads":[{"id":"t","projectId":"p","title":"Reloaded"}]}})).unwrap()))).unwrap();
    office
        .updates
        .try_send(connection::Update::Phase(connection::Phase::Ready))
        .unwrap();
    office
        .updates
        .try_send(connection::Update::Shell(Box::new(
            shell::ShellItem::Synchronized,
        )))
        .unwrap();
    cx.run_until_parked();
    office.app.read_with(cx, |app, _| {
        assert!(!app.connection_ready());
        assert!(matches!(app.connection_phase, connection::Phase::Syncing));
    });
    let (_, fresh) = call(&office.requests, "server.getConfig");
    fresh.try_send(Ok(config("different-server-id"))).unwrap();
    cx.run_until_parked();
    office.app.read_with(cx, |app, _| {
        assert!(app.commands.is_none());
        assert!(matches!(app.connection_phase, connection::Phase::Error(_)));
    });
    lab.app.read_with(cx, |app, _| {
        assert_eq!(app.generation, lab_epoch);
        assert!(app.connection_ready());
        assert!(app.environment_is_selected());
    });
}
#[gpui_kit::test]
fn r15_m_all_environments_merges_repository_and_routes_same_thread_id_and_alert(
    cx: &mut TestAppContext,
) {
    let (office, lab, cx) = pair(cx);
    let toggle = cx.debug_bounds("all-environments").unwrap();
    cx.simulate_click(toggle.center(), Default::default());
    cx.run_until_parked();
    assert!(cx.debug_bounds("environment-project-group-0").is_some());
    assert!(
        cx.debug_bounds("environment-project-group-1").is_none(),
        "canonical repository must merge"
    );
    let grouping = cx.debug_bounds("environment-grouping").unwrap();
    cx.simulate_click(grouping.center(), Default::default());
    cx.run_until_parked();
    assert!(
        cx.debug_bounds("environment-project-group-1").is_some(),
        "machine grouping separates the two hosts"
    );
    let thread = cx
        .debug_bounds("environment-thread-https://lab.example-t")
        .unwrap();
    cx.simulate_click(thread.center(), Default::default());
    cx.run_until_parked();
    assert!(
        lab.app
            .read_with(cx, |app, _| app.environment_is_selected())
    );
    assert!(
        !office
            .app
            .read_with(cx, |app, _| app.environment_is_selected())
    );
    // Each page already selected t through its own real command channel.
    // Navigating to that retained selection must not replay either request.
    assert!(lab.requests.is_empty());
    assert!(office.requests.is_empty());
    cx.update(|_, cx| {
        office.app.update(cx, |app, _| {
            app.notification_preferences.in_app = true;
            app.window_active = true;
        })
    });
    office.updates.try_send(connection::Update::Shell(Box::new(shell::ShellItem::ThreadUpserted { sequence: 1, thread: serde_json::from_value(json!({"id":"t","projectId":"p","title":"Office failed","session":{"status":"error","lastError":"failure"}})).unwrap() }))).unwrap();
    cx.run_until_parked();
    let alert = cx.debug_bounds("environment-alert-0").unwrap();
    cx.simulate_click(alert.center(), Default::default());
    cx.run_until_parked();
    assert!(
        office
            .app
            .read_with(cx, |app, _| app.environment_is_selected())
    );
    assert_eq!(
        office
            .app
            .read_with(cx, |app, _| app.selected_thread.clone()),
        Some("t".into())
    );
}
