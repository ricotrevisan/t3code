use super::*;

#[gpui_kit::test]
fn r13_e_live_checkout_subscription_merges_updates_and_rejects_late_owner(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    cx.update(|_, cx| app.update(cx, |app, cx| app.sync_git_status(cx)));
    cx.run_until_parked();
    let connection::Command::Subscribe { key, tag, payload } = requests.vcs() else {
        panic!("subscription")
    };
    assert_eq!(tag, "subscribeVcsStatus");
    assert_eq!(payload, json!({"cwd":"/tmp/fixture"}));
    cx.update(|_, cx| app.update(cx, |app, cx| {
        app.apply_git_status(&key, Ok(json!({"_tag":"snapshot","local":{"isRepo":true,"refName":"feature","hasWorkingTreeChanges":true,"workingTree":{"files":[{"path":"a.rs","insertions":2,"deletions":1}],"insertions":2,"deletions":1}},"remote":{"hasUpstream":true,"aheadCount":1,"behindCount":0,"pr":null}})), cx);
    }));
    cx.run_until_parked();
    let diff = cx.debug_bounds("header-current-diff").unwrap();
    cx.simulate_click(diff.center(), Default::default());
    cx.run_until_parked();
    let connection::Command::Call {
        tag,
        payload,
        reply,
    } = requests.try_recv().unwrap()
    else {
        panic!("diff preview")
    };
    assert_eq!(tag, "review.getDiffPreview");
    assert_eq!(payload["cwd"], "/tmp/fixture");
    reply.send_blocking(Err("preview denied".into())).unwrap();
    cx.run_until_parked();
    app.read_with(cx, |app, _| {
        assert_eq!(app.diffs.error.as_deref(), Some("preview denied"))
    });
    cx.update(|_,cx| app.update(cx, |app,cx| {
        app.apply_git_status(&key,Ok(json!({"_tag":"localUpdated","local":{"isRepo":true,"hasWorkingTreeChanges":false,"workingTree":{"files":[],"insertions":0,"deletions":0}}})),cx);
        assert_eq!(app.live_git_status().unwrap()["aheadCount"],1);
        app.apply_git_status(&key,Ok(json!({"_tag":"remoteUpdated","remote":null})),cx);
        assert!(app.live_git_status().unwrap().get("aheadCount").is_none());
        app.apply_git_status(&key,Err("watch failed".into()),cx);
        assert!(app.live_git_status().is_none());
        app.shell.threads[0].worktree_path = Some("/tmp/new".into()); app.sync_git_status(cx);
        app.apply_git_status(&key,Ok(json!({"_tag":"snapshot","local":{"isRepo":true},"remote":null})),cx);
        assert!(app.live_git_status().is_none());
    }));
    cx.run_until_parked();
    assert!(matches!(requests.vcs(),connection::Command::Unsubscribe(k) if k==key));
    assert!(
        matches!(requests.vcs(),connection::Command::Subscribe{payload,..} if payload["cwd"]=="/tmp/new")
    );
}

#[gpui_kit::test]
fn r13_e_direct_saved_action_uses_checkout_and_failure_is_retryable(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    cx.update(|_,cx| app.update(cx,|app,cx| {
        app.project_settings=Some(json!({"defaultProjectScripts":[{"id":"setup","name":"Setup","command":"setup","runOnWorktreeCreate":true},{"id":"test","name":"Test","command":"printf saved"}]}));
        app.shell.threads[0].worktree_path=Some("/tmp/action-worktree".into()); cx.notify();
    }));
    cx.run_until_parked();
    let bounds = cx.debug_bounds("header-run-action").unwrap();
    cx.simulate_click(bounds.center(), Default::default());
    cx.run_until_parked();
    let connection::Command::Call {
        tag,
        payload,
        reply,
    } = requests.try_recv().unwrap()
    else {
        panic!("terminal open")
    };
    assert_eq!(tag, "terminal.open");
    assert_eq!(payload["cwd"], "/tmp/action-worktree");
    reply.send_blocking(Err("cwd missing".into())).unwrap();
    cx.run_until_parked();
    app.read_with(cx, |app, _| {
        assert_eq!(app.action_runs[0].script["id"], "test");
        assert_eq!(app.action_runs[0].error.as_deref(), Some("cwd missing"));
        assert!(!app.action_runs[0].busy);
    });
    cx.simulate_click(bounds.center(), Default::default());
    cx.run_until_parked();
    assert!(
        matches!(requests.try_recv().unwrap(),connection::Command::Call{tag,..} if tag=="terminal.open")
    );
}

#[gpui_kit::test]
fn r13_e_detached_commit_form_reaches_transport_without_branch_push(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    let bounds = cx.debug_bounds("push-pr").unwrap();
    cx.simulate_click(bounds.center(), Default::default());
    cx.run_until_parked();
    let connection::Command::Call { tag, reply, .. } = requests.try_recv().unwrap() else {
        panic!("refresh")
    };
    assert_eq!(tag, "vcs.refreshStatus");
    reply.send_blocking(Ok(json!({"isRepo":true,"refName":null,"hasWorkingTreeChanges":true,"workingTree":{"files":[{"path":"a.rs"}]}}))).unwrap();
    cx.run_until_parked();
    let bounds = cx.debug_bounds("git-quick-action").unwrap();
    cx.simulate_click(bounds.center(), Default::default());
    cx.run_until_parked();
    cx.simulate_input("Detached fix");
    cx.run_until_parked();
    let bounds = cx.debug_bounds("git-form-submit").unwrap();
    cx.simulate_click(bounds.center(), Default::default());
    cx.run_until_parked();
    let connection::Command::CallStreamLive {
        tag,
        payload,
        events,
    } = requests.try_recv().unwrap()
    else {
        panic!("commit")
    };
    assert_eq!(tag, "git.runStackedAction");
    assert_eq!(payload["action"], "commit");
    assert_eq!(payload["commitMessage"], "Detached fix");
    events.send_blocking(Err("hook rejected".into())).unwrap();
    cx.run_until_parked();
    app.read_with(cx, |app, _| assert_eq!(app.pending_actions, 0));
}

#[gpui_kit::test]
fn r13_e_diff_whitespace_full_file_failure_retry_and_editor_transport(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.available_editors = vec!["zed".into()];
            app.diffs.base_ref = Some("main".into());
            app.view_git_diff("branch-range", cx);
        })
    });
    cx.run_until_parked();
    let connection::Command::Call {
        tag,
        payload,
        reply,
    } = requests.try_recv().unwrap()
    else {
        panic!("preview")
    };
    assert_eq!(tag, "review.getDiffPreview");
    assert_eq!(payload["baseRef"], "main");
    let preview = json!({"cwd":"/tmp/fixture","sources":[{"kind":"branch-range","baseRef":"main","headRef":"HEAD","truncated":true,"diff":"diff --git a/a.rs b/a.rs\n--- a/a.rs\n+++ b/a.rs\n@@ -1 +1 @@\n-old\n+new\n"}]});
    reply.send_blocking(Ok(preview.clone())).unwrap();
    cx.run_until_parked();
    let bounds = cx.debug_bounds("diff-whitespace").unwrap();
    cx.simulate_click(bounds.center(), Default::default());
    cx.run_until_parked();
    let connection::Command::Call {
        tag,
        payload,
        reply,
    } = requests.try_recv().unwrap()
    else {
        panic!("whitespace")
    };
    assert_eq!(tag, "review.getDiffPreview");
    assert_eq!(payload["ignoreWhitespace"], true);
    assert_eq!(payload["baseRef"], "main");
    reply.send_blocking(Ok(preview)).unwrap();
    cx.run_until_parked();
    let bounds = cx.debug_bounds("diff-full-file").unwrap();
    cx.simulate_click(bounds.center(), Default::default());
    cx.run_until_parked();
    let connection::Command::Call {
        tag,
        payload,
        reply,
    } = requests.try_recv().unwrap()
    else {
        panic!("full file")
    };
    assert_eq!(tag, "review.getDiffFileContents");
    assert_eq!(
        payload,
        json!({"cwd":"/tmp/fixture","sourceKind":"branch-range","changeType":"change","baseRef":"main","headRef":"HEAD","oldPath":"a.rs","newPath":"a.rs"})
    );
    reply.send_blocking(Err("file unavailable".into())).unwrap();
    cx.run_until_parked();
    app.read_with(cx, |app, _| {
        assert_eq!(
            app.diffs.full_file_error.as_deref(),
            Some("file unavailable")
        )
    });
    let bounds = cx.debug_bounds("diff-full-file").unwrap();
    cx.simulate_click(bounds.center(), Default::default());
    cx.run_until_parked();
    let connection::Command::Call { reply, .. } = requests.try_recv().unwrap() else {
        panic!("retry")
    };
    reply
        .send_blocking(Ok(
            json!({"oldContents":"old\nrest\n","newContents":"new\nrest\n"}),
        ))
        .unwrap();
    cx.run_until_parked();
    app.read_with(cx, |app, _| {
        assert_eq!(app.diffs.full_file.as_ref().unwrap().2, "new\nrest\n");
        assert!(app.diffs.full_file_error.is_none());
    });
    let bounds = cx.debug_bounds("diff-open-editor").unwrap();
    cx.simulate_click(bounds.center(), Default::default());
    cx.run_until_parked();
    let connection::Command::Call {
        tag,
        payload,
        reply,
    } = requests.try_recv().unwrap()
    else {
        panic!("open editor")
    };
    assert_eq!(tag, "shell.openInEditor");
    assert_eq!(payload, json!({"cwd":"/tmp/fixture/a.rs","editor":"zed"}));
    reply.send_blocking(Ok(Value::Null)).unwrap();
    cx.run_until_parked();
}

#[gpui_kit::test]
fn r13_e_full_file_receipt_is_retired_by_file_navigation(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    cx.update(|_, cx| app.update(cx, |app, cx| app.view_git_diff("working-tree", cx)));
    cx.run_until_parked();
    let connection::Command::Call { reply, .. } = requests.try_recv().unwrap() else {
        panic!("preview")
    };
    reply.send_blocking(Ok(json!({"cwd":"/tmp/fixture","sources":[{"kind":"working-tree","baseRef":"HEAD","headRef":null,"diff":"diff --git a/a b/a\n--- a/a\n+++ b/a\n@@ -1 +1 @@\n-a\n+A\ndiff --git a/b b/b\n--- a/b\n+++ b/b\n@@ -1 +1 @@\n-b\n+B\n"}]}))).unwrap();
    cx.run_until_parked();
    let bounds = cx.debug_bounds("diff-full-file").unwrap();
    cx.simulate_click(bounds.center(), Default::default());
    cx.run_until_parked();
    let connection::Command::Call { reply, .. } = requests.try_recv().unwrap() else {
        panic!("full")
    };
    let bounds = cx.debug_bounds("diff-file-1").unwrap();
    cx.simulate_click(bounds.center(), Default::default());
    cx.run_until_parked();
    reply
        .send_blocking(Ok(
            json!({"oldContents":"wrong file","newContents":"wrong file"}),
        ))
        .unwrap();
    cx.run_until_parked();
    app.read_with(cx, |app, _| {
        assert_eq!(app.diffs.selected_file, 1);
        assert!(app.diffs.full_file.is_none());
        assert!(!app.diffs.full_file_loading);
    });
}

#[test]
fn r13_e_remote_editor_links_encode_hosts_paths_and_windows_drive() {
    use crate::app::git_editors::remote_editor_url as link;
    assert_eq!(
        link("zed", "office.local", "/tmp/a #/é"),
        Some("zed://ssh/office.local/tmp/a%20%23/%C3%A9".into())
    );
    assert_eq!(
        link("vscode", "user@office", "C:\\Users\\Rico"),
        Some("vscode://vscode-remote/ssh-remote+user%40office/C%3A/Users/Rico".into())
    );
    assert_eq!(
        link("zed", "office", "C:\\Users\\Rico"),
        Some("zed://ssh/office/Users/Rico".into())
    );
    assert_eq!(
        link("zed", "office", "/C:/data"),
        Some("zed://ssh/office/C%3A/data".into())
    );
    assert!(link("file-manager", "office", "/tmp").is_none());
    assert!(link("zed", "", "/tmp").is_none());
}

#[gpui_kit::test]
fn r13_e_branch_context_copy_and_title_menu_share_selected_target(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.shell.threads[0].branch = Some("main".into());
            app.open_branch_picker("branch-selector", window, cx);
        })
    });
    cx.run_until_parked();
    let connection::Command::Call {
        tag,
        payload,
        reply,
    } = requests.try_recv().unwrap()
    else {
        panic!("refs")
    };
    assert_eq!(tag, "vcs.listRefs");
    assert_eq!(payload["cwd"], "/tmp/fixture");
    reply
        .send_blocking(Ok(
            json!({"isRepo":true,"refs":[{"name":"main","current":true}],"nextCursor":null}),
        ))
        .unwrap();
    cx.run_until_parked();
    let press = cx.debug_bounds("branch-ref-0").unwrap().center();
    cx.simulate_mouse_down(press, MouseButton::Right, Default::default());
    cx.simulate_mouse_up(press, MouseButton::Right, Default::default());
    cx.run_until_parked();
    let item = point(press.x + px(30.), press.y + px(17.));
    cx.simulate_mouse_move(item, None, Default::default());
    cx.simulate_click(item, Default::default());
    cx.run_until_parked();
    assert_eq!(
        cx.read_from_clipboard().unwrap().text().as_deref(),
        Some("main")
    );
    assert!(requests.is_empty(), "copy must never checkout");
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    let bounds = cx.debug_bounds("header-title-actions").unwrap();
    cx.simulate_click(bounds.center(), Default::default());
    cx.run_until_parked();
    app.read_with(cx, |app, _| {
        assert!(app.thread_menu_open);
        assert!(app.thread_menu_row.is_none());
        assert_eq!(app.selected_thread.as_deref(), Some("t"));
    });
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    let press = cx.debug_bounds("header-project").unwrap().center();
    cx.simulate_mouse_down(press, MouseButton::Right, Default::default());
    cx.simulate_mouse_up(press, MouseButton::Right, Default::default());
    cx.run_until_parked();
    let item = point(press.x + px(30.), press.y + px(17.));
    cx.simulate_mouse_move(item, None, Default::default());
    cx.simulate_click(item, Default::default());
    cx.run_until_parked();
    app.read_with(cx, |app, _| {
        assert_eq!(
            app.sidebar_state.project_settings_target.as_ref(),
            Some(&(app.environment_scope.clone(), "p".into()))
        )
    });
    assert!(requests.is_empty(), "opening settings is not a write");
}

#[gpui_kit::test]
fn r13_e_remote_editor_uses_advertised_host_never_server_exec(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    cx.update(|_,cx| app.update(cx,|app,cx| {
        app.environment_scope="https://remote.example".into();
        app.accept_config_snapshot(json!({"remoteOpenTargets":[{"kind":"tailscale","host":"office.tailnet"}],"providers":[]}),cx);
        app.available_editors=vec!["file-manager".into()];
        assert!(app.editor_unavailable_reason().is_none()); assert_eq!(app.preferred_editor().as_deref(),Some("zed"));
        app.open_in_editor(cx);
    }));
    cx.run_until_parked();
    assert!(
        requests.is_empty(),
        "remote URL is dispatched on the viewing machine, never shell.openInEditor"
    );
    assert_eq!(
        cx.opened_url().as_deref(),
        Some("zed://ssh/office.tailnet/tmp/fixture")
    );
}
