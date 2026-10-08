//! Checkout-scoped requests and ordered session receipts.
use super::branch_workflows::{complete_stop, finish_owned_stop, refresh_refs, stop_observation};
use super::*;

fn call(
    requests: &Requests,
    tag: &str,
) -> (Value, async_channel::Sender<Result<Value, String>>) {
    let (actual, payload, reply) = loop {
        match requests.try_recv().unwrap() {
            connection::Command::Call {
                tag,
                payload,
                reply,
            }
            | connection::Command::CallMutation {
                tag,
                payload,
                reply,
                ..
            } => break (tag, payload, reply),
            connection::Command::Unsubscribe(key) if key.starts_with("checkout-stop-") => continue,
            _ => panic!("expected {tag}"),
        }
    };
    assert_eq!(actual, tag);
    (payload, reply)
}

fn other_checkout(app: &Entity<NativeApp>, cx: &mut VisualTestContext) {
    cx.update(|_, cx| {
        app.update(cx, |app, _| {
            let mut thread = app.shell.threads[0].clone();
            thread.id = "other".into();
            thread.worktree_path = Some("/tmp/other-checkout".into());
            app.shell.threads.push(thread);
        })
    });
}

fn select_other(
    app: &Entity<NativeApp>,
    cx: &mut VisualTestContext,
    requests: &Requests,
) {
    cx.update(|window, cx| app.update(cx, |app, cx| app.select_thread("other", window, cx)));
    cx.run_until_parked();
    assert!(matches!(
        requests.try_recv().unwrap(),
        connection::Command::Subscribe { .. }
    ));
    assert!(
        matches!(requests.try_recv().unwrap(), connection::Command::SelectThread(Some(id)) if id == "other")
    );
}

#[gpui_kit::test]
fn same_project_checkout_retires_listing_content_refs_and_busy_receipts(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    other_checkout(&app, cx);
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.file_path
                .update(cx, |input, cx| input.set_value("old.txt", window, cx));
            app.list_files(false, cx);
            app.file_operation(false, window, cx);
            app.worktree_operation("vcs.listRefs", false, cx);
            app.action_busy = true;
            app.confirm_worktree_removal = Some("/tmp/old".into());
        })
    });
    cx.run_until_parked();
    let (_, listing) = call(&requests, "projects.listEntries");
    let (_, content) = call(&requests, "projects.readFile");
    let (_, refs) = call(&requests, "vcs.listRefs");
    select_other(&app, cx, &requests);
    // Task cancellation can close a reply receiver; a detached read still delivers.
    let _ = listing.send_blocking(Ok(
        json!({"entries":[{"path":"stale","kind":"file"}],"truncated":false}),
    ));
    content
        .send_blocking(Ok(
            json!({"relativePath":"old.txt","contents":"stale","truncated":false}),
        ))
        .unwrap();
    let _ = refs.send_blocking(Ok(
        json!({"refs":[{"name":"stale","current":true}],"isRepo":true,"nextCursor":null}),
    ));
    cx.run_until_parked();
    app.read_with(cx, |app, _| {
        assert!(app.file_entries.is_empty());
        assert!(app.refs.is_empty());
        assert!(app.file_editor.target.is_none());
        assert!(!app.file_busy && !app.worktree_busy && !app.action_busy);
        assert!(app.confirm_worktree_removal.is_none());
    });
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            assert_ne!(app.file_contents.read(cx).value().as_str(), "stale");
            app.list_files(false, cx);
            app.file_operation(false, window, cx);
            app.worktree_operation("vcs.listRefs", false, cx);
        })
    });
    cx.run_until_parked();
    let (payload, listing) = call(&requests, "projects.listEntries");
    assert_eq!(payload["cwd"], "/tmp/other-checkout");
    listing
        .send_blocking(Ok(
            json!({"entries":[{"path":"fresh","kind":"file"}],"truncated":false}),
        ))
        .unwrap();
    let (payload, content) = call(&requests, "projects.readFile");
    assert_eq!(payload["cwd"], "/tmp/other-checkout");
    content
        .send_blocking(Ok(
            json!({"relativePath":"old.txt","contents":"fresh","truncated":false}),
        ))
        .unwrap();
    let (payload, refs) = call(&requests, "vcs.listRefs");
    assert_eq!(payload["cwd"], "/tmp/other-checkout");
    refs.send_blocking(Ok(
        json!({"refs":[{"name":"fresh","current":true}],"isRepo":true,"nextCursor":null}),
    ))
    .unwrap();
    cx.run_until_parked();
    app.read_with(cx, |app, _| {
        assert_eq!(app.file_entries[0].path, "fresh");
        assert_eq!(app.refs[0].name, "fresh");
        assert!(!app.file_busy && !app.worktree_busy);
    });
}

#[gpui_kit::test]
fn same_project_content_search_response_cannot_cross_checkout(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    other_checkout(&app, cx);
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.content_query
                .update(cx, |input, cx| input.set_value("needle", window, cx));
            app.search_contents(cx);
        })
    });
    cx.run_until_parked();
    let (_, old) = call(&requests, "projects.searchContents");
    select_other(&app, cx, &requests);
    let _ = old.send_blocking(Ok(json!({"matches":[],"truncated":false})));
    cx.run_until_parked();
    assert!(app.read_with(cx, |app, _| app.content_results.is_none()));
    cx.update(|_, cx| app.update(cx, |app, cx| app.search_contents(cx)));
    cx.run_until_parked();
    let (payload, fresh) = call(&requests, "projects.searchContents");
    assert_eq!(payload["cwd"], "/tmp/other-checkout");
    fresh
        .send_blocking(Ok(json!({"matches":[],"truncated":false})))
        .unwrap();
    cx.run_until_parked();
    assert!(app.read_with(cx, |app, _| app.content_results.is_some()));
}

#[gpui_kit::test]
fn dirty_selection_is_preserved_until_explicit_save_or_discard(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    other_checkout(&app, cx);
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.file_editor.loaded(
                "/tmp/fixture".into(),
                serde_json::from_value(json!({
                    "relativePath":"edit.txt", "contents":"saved", "truncated":false
                }))
                .unwrap(),
            );
            app.file_environment = app.environment_scope.clone();
            app.file_contents
                .update(cx, |input, cx| input.set_value("dirty text", window, cx));
            app.inspector_section = Some(InspectorSection::Files);
            app.select_thread("t", window, cx);
            assert_eq!(app.file_contents.read(cx).value().as_str(), "dirty text");
        })
    });
    cx.run_until_parked();
    let project = cx.debug_bounds("project-p").unwrap();
    cx.simulate_click(project.center(), Default::default());
    cx.run_until_parked();
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            assert_eq!(app.selected_thread.as_deref(), Some("t"));
            assert_eq!(app.file_contents.read(cx).value().as_str(), "dirty text");
            app.select_thread("other", window, cx);
            assert_eq!(app.selected_thread.as_deref(), Some("t"));
            assert_eq!(app.file_contents.read(cx).value().as_str(), "dirty text");
            assert!(app.status.contains("Save or Discard"));
            app.pick_checkout("feature".into(), None, window, cx);
            app.new_thread(window, cx);
            assert!(app.pending_threads.is_empty());
            assert_eq!(app.selected_thread.as_deref(), Some("t"));
            assert_eq!(app.file_contents.read(cx).value().as_str(), "dirty text");
        })
    });
    cx.run_until_parked();
    assert!(requests.try_recv().is_err());
    // The existing visible discard control is the explicit way out.
    let discard = cx.debug_bounds("discard-file-edits").unwrap();
    cx.simulate_click(discard.center(), Default::default());
    cx.run_until_parked();
    cx.update(|_, cx| {
        app.read_with(cx, |app, cx| {
            assert_eq!(app.file_contents.read(cx).value().as_str(), "saved");
        })
    });
    select_other(&app, cx, &requests);
    assert!(app.read_with(cx, |app, _| app.file_editor.target.is_none()));
}

#[gpui_kit::test]
fn dirty_project_change_requires_save_receipt_and_keeps_original_write_target(
    cx: &mut TestAppContext,
) {
    let (app, cx, requests) = harness(cx);
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            let mut project = app.shell.projects[0].clone();
            project.id = "q".into();
            project.workspace_root = "/tmp/second-project".into();
            app.shell.projects.push(project);
            app.file_editor.loaded(
                "/tmp/fixture".into(),
                serde_json::from_value(json!({
                    "relativePath":"edit.txt", "contents":"saved", "truncated":false
                }))
                .unwrap(),
            );
            app.file_environment = app.environment_scope.clone();
            app.file_contents
                .update(cx, |input, cx| input.set_value("edited", window, cx));
            cx.notify();
        })
    });
    cx.run_until_parked();
    let project = cx.debug_bounds("project-q").unwrap();
    cx.simulate_click(project.center(), Default::default());
    cx.run_until_parked();
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            assert_eq!(app.selected_project.as_deref(), Some("p"));
            assert_eq!(app.selected_thread.as_deref(), Some("t"));
            assert_eq!(app.file_contents.read(cx).value().as_str(), "edited");
            assert!(app.status.contains("Save or Discard"));
            app.file_operation(true, window, cx);
        })
    });
    cx.run_until_parked();
    let (payload, save) = call(&requests, "projects.writeFile");
    assert_eq!(
        payload,
        json!({"cwd":"/tmp/fixture", "relativePath":"edit.txt", "contents":"edited"})
    );
    cx.simulate_click(project.center(), Default::default());
    cx.run_until_parked();
    assert_eq!(
        app.read_with(cx, |app, _| app.selected_project.clone())
            .as_deref(),
        Some("p")
    );
    save.send_blocking(Ok(json!({}))).unwrap();
    cx.run_until_parked();
    cx.simulate_click(project.center(), Default::default());
    cx.run_until_parked();
    app.read_with(cx, |app, _| {
        assert_eq!(app.selected_project.as_deref(), Some("q"));
        assert!(app.selected_thread.is_none());
        assert!(app.file_editor.target.is_none());
    });
}

#[gpui_kit::test]
fn root_worktree_path_is_normalized_to_null_before_metadata(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    active_worktree(&app, cx);
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.refs.push(
                serde_json::from_value(
                    json!({"name":"root-branch","current":false,"worktreePath":"/tmp/fixture/"}),
                )
                .unwrap(),
            );
            app.pick_checkout(
                "root-branch".into(),
                Some("/tmp/fixture/".into()),
                window,
                cx,
            );
        })
    });
    cx.run_until_parked();
    let key = stop_observation(&app, cx, &requests);
    let (stop_payload, stop) = call(&requests, "orchestration.dispatchCommand");
    stop.send_blocking(Ok(json!({"sequence":10}))).unwrap();
    cx.run_until_parked();
    assert!(requests.try_recv().is_err(), "acceptance is not shutdown");
    complete_stop(&app, cx, &key, &stop_payload["commandId"]);
    let (payload, meta) = call(&requests, "orchestration.dispatchCommand");
    assert_eq!(payload["branch"], "root-branch");
    assert!(payload["worktreePath"].is_null());
    // Detail projection can precede both the shell projection and receipt.
    cx.update(|_, cx| {
        app.update(cx, |app, _| {
            app.detail.thread =
                Some(json!({"id":"t", "branch":"root-branch", "worktreePath":null}).into());
        })
    });
    meta.send_blocking(Ok(json!({"sequence":2}))).unwrap();
    cx.run_until_parked();
    refresh_refs(cx, &requests);
    app.read_with(cx, |app, _| {
        assert!(!app.worktree_busy);
        assert!(app.shell.threads[0].worktree_path.is_none());
        assert_eq!(app.selected_workspace().as_deref(), Some("/tmp/fixture"));
    });
}

#[gpui_kit::test]
fn archived_picker_keeps_selection_and_composer(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            let thread = app.shell.threads.remove(0);
            app.archived_shell.threads.push(thread);
            app.composer.update(cx, |input, cx| {
                input.set_value("archived prompt", window, cx)
            });
            app.pick_checkout("feature".into(), None, window, cx);
            assert_eq!(app.selected_thread.as_deref(), Some("t"));
            assert_eq!(app.composer.read(cx).value().as_str(), "archived prompt");
            assert!(app.new_thread_checkout.is_none());
            assert!(app.pending_threads.is_empty());
            assert!(app.status.contains("Restore the archived thread"));
        })
    });
    cx.run_until_parked();
    assert!(requests.try_recv().is_err());
}

#[gpui_kit::test]
fn remote_ref_metadata_uses_actual_branch_and_detached_null(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    for actual in [json!("foo"), Value::Null] {
        cx.update(|window, cx| {
            app.update(cx, |app, cx| {
                app.shell.synchronized = true;
                app.pick_checkout("origin/foo".into(), None, window, cx);
            })
        });
        cx.run_until_parked();
        finish_owned_stop(&app, cx, &requests);
        let (payload, switch) = call(&requests, "vcs.switchRef");
        assert_eq!(
            payload,
            json!({"cwd":"/tmp/fixture", "refName":"origin/foo"})
        );
        switch.send_blocking(Ok(json!({"refName":actual}))).unwrap();
        cx.run_until_parked();
        let (payload, meta) = call(&requests, "orchestration.dispatchCommand");
        assert_eq!(payload["threadId"], "t");
        assert_eq!(payload["branch"], actual);
        assert!(payload["worktreePath"].is_null());
        // Project the metadata before its receipt, as the server can do.
        cx.update(|_, cx| {
            app.update(cx, |app, _| {
                app.shell.threads[0].branch = actual.as_str().map(str::to_owned);
                app.detail.thread = Some(json!({"id":"t", "branch":actual, "worktreePath":null}).into());
            })
        });
        meta.send_blocking(Ok(json!({"sequence":2}))).unwrap();
        cx.run_until_parked();
        refresh_refs(cx, &requests);
        app.read_with(cx, |app, _| {
            assert_eq!(app.shell.threads[0].branch.as_deref(), actual.as_str());
            assert_eq!(app.detail.thread.as_ref().unwrap()["branch"], actual);
            assert!(!app.worktree_busy);
        });
    }
}

fn active_worktree(app: &Entity<NativeApp>, cx: &mut VisualTestContext) {
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.shell.synchronized = true;
            app.shell.threads[0].worktree_path = Some("/tmp/active-tree".into());
            app.shell.threads[0].session = Some(shell::Session {
                status: "ready".into(),
                last_error: None,
            });
            app.refs = serde_json::from_value(json!([
                {"name":"main","current":false,"isDefault":true,"worktreePath":null},
                {"name":"reuse","current":false,"isDefault":false,"worktreePath":"/tmp/reused-tree"}
            ]))
            .unwrap();
            app.composer.update(cx, |input, cx| {
                input.set_value("retain my prompt", window, cx)
            });
        })
    });
}

#[gpui_kit::test]
fn root_picker_blocks_other_active_session_but_reuse_does_not_switch_root(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    active_worktree(&app, cx);
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            let mut other = app.shell.threads[0].clone();
            other.id = "other".into();
            other.worktree_path = None;
            app.shell.threads.push(other);
            app.pick_checkout("main".into(), None, window, cx);
            assert!(app.checkout_operations.is_empty());
            assert!(
                app.status
                    .contains("Stop it first or use a separate worktree")
            );
        })
    });
    cx.run_until_parked();
    assert!(requests.try_recv().is_err());
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.pick_checkout("reuse".into(), Some("/tmp/reused-tree".into()), window, cx);
        })
    });
    cx.run_until_parked();
    finish_owned_stop(&app, cx, &requests);
    let (payload, metadata) = call(&requests, "orchestration.dispatchCommand");
    assert_eq!(payload["type"], "thread.meta.update");
    assert_eq!(payload["worktreePath"], "/tmp/reused-tree");
    metadata.send_blocking(Ok(json!({"sequence":20}))).unwrap();
    cx.run_until_parked();
    refresh_refs(cx, &requests);
    assert!(
        requests.try_recv().is_err(),
        "reuse emits no physical switch"
    );
}

#[gpui_kit::test]
fn other_worktree_session_does_not_block_root_switch(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    other_checkout(&app, cx);
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.shell.synchronized = true;
            app.shell.threads[1].session = Some(shell::Session {
                status: "ready".into(),
                last_error: None,
            });
            app.pick_checkout("feature".into(), None, window, cx);
        })
    });
    cx.run_until_parked();
    finish_owned_stop(&app, cx, &requests);
    let (payload, switch) = call(&requests, "vcs.switchRef");
    assert_eq!(payload["cwd"], "/tmp/fixture");
    switch.send_blocking(Err("not transmitted".into())).unwrap();
    cx.run_until_parked();
    assert!(requests.try_recv().is_err());
}

#[gpui_kit::test]
fn default_root_waits_for_stop_then_switch_then_metadata(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    active_worktree(&app, cx);
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.pick_checkout("main".into(), None, window, cx)
        })
    });
    cx.run_until_parked();
    let key = stop_observation(&app, cx, &requests);
    let (payload, stop) = call(&requests, "orchestration.dispatchCommand");
    assert_eq!(payload["type"], "thread.session.stop");
    assert!(requests.try_recv().is_err());
    stop.send_blocking(Ok(json!({"sequence":10}))).unwrap();
    cx.run_until_parked();
    assert!(requests.try_recv().is_err(), "acceptance is not shutdown");
    complete_stop(&app, cx, &key, &payload["commandId"]);
    let (payload, switch) = call(&requests, "vcs.switchRef");
    assert_eq!(payload, json!({"cwd":"/tmp/fixture","refName":"main"}));
    assert!(requests.try_recv().is_err());
    switch.send_blocking(Ok(json!({"refName":"main"}))).unwrap();
    cx.run_until_parked();
    let (payload, meta) = call(&requests, "orchestration.dispatchCommand");
    assert_eq!(payload["type"], "thread.meta.update");
    assert_eq!(payload["branch"], "main");
    assert!(payload.get("worktreePath").unwrap().is_null());
    meta.send_blocking(Ok(json!({"sequence":2}))).unwrap();
    cx.run_until_parked();
    refresh_refs(cx, &requests);
    cx.update(|_, cx| {
        app.read_with(cx, |app, cx| {
            assert_eq!(app.selected_workspace().as_deref(), Some("/tmp/fixture"));
            assert_eq!(app.composer.read(cx).value().as_str(), "retain my prompt");
            assert!(!app.worktree_busy);
        })
    });
}

#[gpui_kit::test]
fn failed_stop_and_failed_switch_never_move_thread(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    active_worktree(&app, cx);
    for fail_stop in [true, false] {
        cx.update(|window, cx| {
            app.update(cx, |app, cx| {
                app.pick_checkout("main".into(), None, window, cx)
            })
        });
        cx.run_until_parked();
        let key = stop_observation(&app, cx, &requests);
        let (payload, stop) = call(&requests, "orchestration.dispatchCommand");
        stop.send_blocking(if fail_stop {
            Err("stop rejected".into())
        } else {
            Ok(json!({"sequence":10}))
        })
        .unwrap();
        cx.run_until_parked();
        if !fail_stop {
            assert!(requests.try_recv().is_err());
            complete_stop(&app, cx, &key, &payload["commandId"]);
            let (_, switch) = call(&requests, "vcs.switchRef");
            switch.send_blocking(Err("switch rejected".into())).unwrap();
            cx.run_until_parked();
        }
        if fail_stop {
            assert!(matches!(
                requests.try_recv().unwrap(),
                connection::Command::Unsubscribe(_)
            ));
        }
        assert!(requests.try_recv().is_err());
        app.read_with(cx, |app, _| {
            assert_eq!(
                app.selected_workspace().as_deref(),
                Some("/tmp/active-tree")
            );
            assert!(!app.worktree_busy);
        });
    }
}

#[gpui_kit::test]
fn existing_worktree_waits_for_stop_without_switching_ref(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    active_worktree(&app, cx);
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.pick_checkout("reuse".into(), Some("/tmp/reused-tree".into()), window, cx)
        })
    });
    cx.run_until_parked();
    let key = stop_observation(&app, cx, &requests);
    let (payload, stop) = call(&requests, "orchestration.dispatchCommand");
    assert_eq!(payload["type"], "thread.session.stop");
    assert!(requests.try_recv().is_err());
    stop.send_blocking(Ok(json!({"sequence":10}))).unwrap();
    cx.run_until_parked();
    assert!(requests.try_recv().is_err(), "acceptance is not shutdown");
    complete_stop(&app, cx, &key, &payload["commandId"]);
    let (payload, meta) = call(&requests, "orchestration.dispatchCommand");
    assert_eq!(payload["type"], "thread.meta.update");
    assert_eq!(payload["worktreePath"], "/tmp/reused-tree");
    meta.send_blocking(Ok(json!({"sequence":2}))).unwrap();
    cx.run_until_parked();
    refresh_refs(cx, &requests);
    assert!(requests.try_recv().is_err());
}

#[gpui_kit::test]
fn remote_branch_change_retires_a_file_read_in_the_same_directory(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.shell.threads[0].branch = Some("before".into());
            app.detail.thread = Some(json!({"id":"t","branch":"before","worktreePath":null}).into());
            app.file_path
                .update(cx, |input, cx| input.set_value("file.txt", window, cx));
            app.file_operation(false, window, cx);
        })
    });
    cx.run_until_parked();
    let (payload, delayed) = call(&requests, "projects.readFile");
    assert_eq!(payload["cwd"], "/tmp/fixture");
    assert!(app.read_with(cx, |app, _| app.file_busy));
    cx.update(|_, cx| app.update(cx, |app, cx| {
        app.apply_thread_update("t", json!({"kind":"snapshot","snapshot":{"snapshotSequence":2,"thread":{"id":"t","branch":"after","worktreePath":null,"messages":[],"activities":[]}}}), cx).unwrap();
    }));
    delayed
        .send_blocking(Ok(
            json!({"relativePath":"file.txt","contents":"from the old branch","truncated":false}),
        ))
        .unwrap();
    cx.run_until_parked();
    assert!(app.read_with(cx, |app, cx| !app.file_busy
        && app.file_contents.read(cx).value().is_empty()));
    assert_eq!(
        app.read_with(cx, |app, _| app.selected_workspace()),
        Some("/tmp/fixture".into())
    );
    assert_eq!(
        app.read_with(cx, |app, _| app.selected_thread_branch()),
        Some("after".into())
    );
}

#[gpui_kit::test]
fn remote_shell_checkout_change_retires_refs_and_keeps_dirty_file_target(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.detail.thread = None;
            app.shell.threads[0].branch = Some("before".into());
            app.worktree_operation("vcs.listRefs", false, cx);
        })
    });
    cx.run_until_parked();
    let (_, delayed) = call(&requests, "vcs.listRefs");
    assert!(app.read_with(cx, |app, _| app.worktree_busy));
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.file_editor.loaded(
                "/tmp/fixture".into(),
                serde_json::from_value(
                    json!({"relativePath":"edited.txt","contents":"saved","truncated":false}),
                )
                .unwrap(),
            );
            app.file_environment = app.environment_scope.clone();
            app.file_contents
                .update(cx, |input, cx| input.set_value("unsaved edit", window, cx));
            let mut thread = app.shell.threads[0].clone();
            thread.branch = Some("after".into());
            thread.worktree_path = Some("/tmp/remote-checkout".into());
            app.apply_shell_item(
                shell::ShellItem::ThreadUpserted {
                    sequence: 2,
                    thread,
                },
                cx,
            )
            .unwrap();
        })
    });
    let _ = delayed.send_blocking(Ok(
        json!({"isRepo":true,"nextCursor":null,"refs":[{"name":"old-branch","current":true}]}),
    ));
    cx.run_until_parked();
    assert!(app.read_with(cx, |app, _| !app.worktree_busy && app.refs.is_empty()));
    assert_eq!(
        app.read_with(cx, |app, cx| app.file_contents.read(cx).value().to_string()),
        "unsaved edit"
    );
    cx.update(|window, cx| app.update(cx, |app, cx| app.file_operation(true, window, cx)));
    cx.run_until_parked();
    let (save, _) = call(&requests, "projects.writeFile");
    assert_eq!(save["cwd"], "/tmp/fixture");
    assert_eq!(save["relativePath"], "edited.txt");
    assert_eq!(save["contents"], "unsaved edit");
}

#[gpui_kit::test]
fn unsent_draft_checkout_stays_local_and_retains_prompt(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            let thread = app.shell.threads.remove(0);
            app.pending_threads.push(drafts::PendingThread {
                thread,
                created_at: commands::now(),
                prepare_worktree: None,
                run_setup_script: false,
            });
            app.composer
                .update(cx, |input, cx| input.set_value("local prompt", window, cx));
            app.pick_checkout("feature".into(), Some("/tmp/local-tree".into()), window, cx);
            assert_eq!(app.composer.read(cx).value().as_str(), "local prompt");

            assert_eq!(app.selected_thread.as_deref(), Some("t"));
        })
    });
    cx.run_until_parked();
    let (payload, actual) = call(&requests, "vcs.listRefs");
    assert_eq!(payload["cwd"], "/tmp/local-tree");
    actual.send_blocking(Ok(json!({"isRepo":true,"refs":[{"name":"actual-existing","current":true}],"nextCursor":null}))).unwrap();
    cx.run_until_parked();
    refresh_refs(cx, &requests);
    assert_eq!(
        app.read_with(cx, |app, _| app.pending_threads[0].thread.branch.clone()),
        Some("actual-existing".into())
    );
    assert_eq!(
        app.read_with(cx, |app, _| app.pending_threads[0]
            .thread
            .worktree_path
            .clone()),
        Some("/tmp/local-tree".into())
    );
    assert!(requests.try_recv().is_err());
}
