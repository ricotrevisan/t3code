//! Workspace actions must use selection, never the editable clone destination.
use super::*;

fn add_other_project(app: &Entity<NativeApp>, cx: &mut VisualTestContext) {
    cx.update(|_, cx| {
        app.update(cx, |app, _| {
            let mut project = app.shell.projects[0].clone();
            project.id = "q".into();
            project.workspace_root = "/tmp/other-project".into();
            app.shell.projects.push(project);
            let mut thread = app.shell.threads[0].clone();
            thread.id = "u".into();
            thread.project_id = "q".into();
            app.shell.threads.push(thread);
        })
    });
}

fn switch_thread(
    app: &Entity<NativeApp>,
    cx: &mut VisualTestContext,
    requests: &async_channel::Receiver<connection::Command>,
    id: &str,
    pending: bool,
) {
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.select_thread(id, window, cx);
            // Model an edited clone destination after the selection changed.
            app.workspace.update(cx, |input, cx| {
                input.set_value("/tmp/wrong-clone-destination", window, cx)
            });
            app.available_editors = vec!["file-manager".into(), "vscode".into(), "zed".into()];
        })
    });
    cx.run_until_parked();
    assert!(
        matches!(requests.try_recv().unwrap(), connection::Command::Subscribe { tag, .. } if tag == "subscribeWorktreeSetup")
    );
    let connection::Command::SelectThread(selected) = requests.try_recv().unwrap() else {
        panic!("expected thread selection");
    };
    assert_eq!(selected.as_deref(), if pending { None } else { Some(id) });
    assert!(requests.try_recv().is_err());
}

fn assert_action_cwds(
    app: &Entity<NativeApp>,
    cx: &mut VisualTestContext,
    requests: &async_channel::Receiver<connection::Command>,
    id: &str,
    cwd: &str,
) {
    cx.update(|_, cx| {
        app.update(cx, |app, cx| app.open_in_editor(cx));
    });
    cx.run_until_parked();
    let connection::Command::Call {
        tag,
        payload,
        reply,
    } = requests.try_recv().unwrap()
    else {
        panic!("expected editor RPC");
    };
    assert_eq!(tag, "shell.openInEditor");
    assert_eq!(payload, json!({"cwd":cwd,"editor":"zed"}));
    reply.send_blocking(Ok(json!({}))).unwrap();
    cx.run_until_parked();

    cx.update(|_, cx| {
        app.update(cx, |app, cx| app.push_and_create_pr(cx));
    });
    cx.run_until_parked();
    let connection::Command::CallStream {
        tag,
        mut payload,
        reply,
    } = requests.try_recv().unwrap()
    else {
        panic!("expected git RPC");
    };
    assert_eq!(tag, "git.runStackedAction");
    let action_id = payload.as_object_mut().unwrap().remove("actionId").unwrap();
    assert!(uuid::Uuid::parse_str(action_id.as_str().unwrap()).is_ok());
    assert_eq!(
        payload,
        json!({
            "cwd":cwd,"action":"commit_push_pr","featureBranch":true,"threadId":id
        })
    );
    reply
        .send_blocking(Ok(vec![json!({
            "kind":"action_finished","result":{"toast":{"title":"Done"}}
        })]))
        .unwrap();
    cx.run_until_parked();
    assert!(requests.try_recv().is_err());
    assert_eq!(app.read_with(cx, |app, _| app.pending_actions), 0);
}

#[gpui_kit::test]
fn workspace_actions_follow_project_and_thread_switch_not_clone_input(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    add_other_project(&app, cx);
    switch_thread(&app, cx, &requests, "u", false);
    assert_action_cwds(&app, cx, &requests, "u", "/tmp/other-project");
    switch_thread(&app, cx, &requests, "t", false);
    assert_action_cwds(&app, cx, &requests, "t", "/tmp/fixture");
}

#[gpui_kit::test]
fn workspace_actions_honor_worktree_detail_override_and_explicit_null(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    add_other_project(&app, cx);
    cx.update(|_, cx| {
        app.update(cx, |app, _| {
            app.shell.threads[1].worktree_path = Some("/tmp/shell-worktree".into());
        })
    });
    switch_thread(&app, cx, &requests, "u", false);
    assert_action_cwds(&app, cx, &requests, "u", "/tmp/shell-worktree");
    cx.update(|_, cx| {
        app.update(cx, |app, _| {
            app.detail.thread = Some(json!({"id":"u","worktreePath":"/tmp/detail-worktree"}));
        })
    });
    assert_action_cwds(&app, cx, &requests, "u", "/tmp/detail-worktree");
    cx.update(|_, cx| {
        app.update(cx, |app, _| {
            app.detail.thread = Some(json!({"id":"u","worktreePath":null}));
        })
    });
    assert_action_cwds(&app, cx, &requests, "u", "/tmp/other-project");
    // Detail for another thread must not replace the selected shell checkout.
    cx.update(|_, cx| {
        app.update(cx, |app, _| {
            app.detail.thread = Some(json!({"id":"t","worktreePath":"/tmp/stale-detail"}));
        })
    });
    assert_action_cwds(&app, cx, &requests, "u", "/tmp/shell-worktree");
}

#[gpui_kit::test]
fn workspace_actions_resolve_pending_and_archived_threads(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    add_other_project(&app, cx);
    cx.update(|_, cx| {
        app.update(cx, |app, _| {
            let mut thread = app.shell.threads.remove(1);
            thread.worktree_path = Some("/tmp/pending-worktree".into());
            app.pending_threads.push(drafts::PendingThread {
                thread,
                created_at: commands::now(),
                prepare_worktree: None,
                run_setup_script: false,
            });
        })
    });
    switch_thread(&app, cx, &requests, "u", true);
    assert_action_cwds(&app, cx, &requests, "u", "/tmp/pending-worktree");
    cx.update(|_, cx| {
        app.update(cx, |app, _| {
            let mut thread = app.pending_threads.remove(0).thread;
            thread.worktree_path = Some("/tmp/archived-worktree".into());
            thread.archived_at = Some(commands::now());
            app.archived_shell.threads.push(thread);
        })
    });
    // A same-thread click is deliberately a no-op; re-enter the archived row
    // from another thread rather than fabricating a subscription refresh.
    switch_thread(&app, cx, &requests, "t", false);
    switch_thread(&app, cx, &requests, "u", false);
    assert_action_cwds(&app, cx, &requests, "u", "/tmp/archived-worktree");
    cx.update(|_, cx| {
        app.update(cx, |app, _| {
            app.detail.thread = Some(json!({"id":"u","worktreePath":null}));
        })
    });
    assert_action_cwds(&app, cx, &requests, "u", "/tmp/other-project");
}

#[gpui_kit::test]
fn workspace_actions_send_no_rpc_for_invalid_selected_context(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    add_other_project(&app, cx);
    for (project, thread) in [
        (None, Some("t")),
        (Some("missing-project"), Some("t")),
        (Some("p"), Some("missing-thread")),
        (Some("q"), Some("t")),
        (None, None),
    ] {
        cx.update(|window, cx| {
            app.update(cx, |app, cx| {
                app.selected_project = project.map(str::to_owned);
                app.selected_thread = thread.map(str::to_owned);
                // Stale detail and next-thread checkout must not rescue invalid selection.
                app.detail.thread = Some(json!({"id":"t","worktreePath":"/tmp/stale-detail"}));
                app.new_thread_checkout =
                    Some(("p".into(), "main".into(), Some("/tmp/next-checkout".into())));
                app.workspace.update(cx, |input, cx| {
                    input.set_value("/tmp/wrong-clone-destination", window, cx)
                });
                app.open_in_editor(cx);
                app.push_and_create_pr(cx);
            })
        });
        cx.run_until_parked();
        assert!(
            requests.try_recv().is_err(),
            "invalid context sent an RPC: {project:?}/{thread:?}"
        );
        assert_eq!(app.read_with(cx, |app, _| app.pending_actions), 0);
    }
    // Even a known project/thread with an empty resolved path must fail closed.
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.selected_project = Some("p".into());
            app.selected_thread = Some("t".into());
            app.detail.thread = Some(json!({"id":"t","worktreePath":"  "}));
            app.workspace.update(cx, |input, cx| {
                input.set_value("/tmp/wrong-clone-destination", window, cx)
            });
            app.open_in_editor(cx);
            app.push_and_create_pr(cx);
        })
    });
    cx.run_until_parked();
    assert!(requests.try_recv().is_err());
}
