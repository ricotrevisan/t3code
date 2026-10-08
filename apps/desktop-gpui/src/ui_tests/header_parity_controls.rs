use super::*;

#[gpui_kit::test]
fn header_git_refresh_is_checkout_scoped_and_uses_real_status_rpc(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    cx.update(|_, cx| app.update(cx, |app, cx| app.toggle_git_header(cx)));
    cx.run_until_parked();
    let connection::Command::Call {
        tag,
        payload,
        reply,
    } = requests.try_recv().unwrap()
    else {
        panic!("expected status")
    };
    assert_eq!(tag, "vcs.refreshStatus");
    assert_eq!(payload, json!({"cwd":"/tmp/fixture"}));
    cx.update(|_, cx| app.update(cx, |app, _| app.generation += 1));
    reply.send_blocking(Ok(json!({"refName":"stale"}))).unwrap();
    cx.run_until_parked();
    app.read_with(cx, |app, _| assert!(app.git_header.status.is_none()));
}

#[gpui_kit::test]
fn header_picker_arrow_and_enter_select_current_row_without_losing_next_plan(
    cx: &mut TestAppContext,
) {
    let (app, cx, requests) = harness(cx);
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.shell.threads[0].branch = Some("main".into());
            app.new_thread_checkout =
                Some(("p".into(), "feature".into(), Some("/tmp/feature".into())));
            app.open_branch_picker("branch-selector", window, cx);
        })
    });
    cx.run_until_parked();
    let connection::Command::Call { reply, .. } = requests.try_recv().unwrap() else {
        panic!("expected refs")
    };
    reply.send_blocking(Ok(json!({"isRepo":true,"refs":[{"name":"main","current":true},{"name":"feature","current":false,"worktreePath":"/tmp/feature"}],"nextCursor":null}))).unwrap();
    cx.run_until_parked();
    cx.simulate_keystrokes("down");
    app.read_with(cx, |app, _| assert_eq!(app.git_picker.index, 1));
    cx.simulate_keystrokes("up");
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    app.read_with(cx, |app, _| {
        assert!(app.branch_picker.is_none());
        assert_eq!(app.new_thread_checkout.as_ref().unwrap().1, "feature");
    });
    assert!(requests.try_recv().is_err());
}

#[gpui_kit::test]
fn header_remote_editor_never_launches_on_server_and_local_choice_is_preferred(
    cx: &mut TestAppContext,
) {
    let (app, cx, requests) = harness(cx);
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.available_editors = vec!["zed".into(), "vscode".into(), "file-manager".into()];
            app.environment_scope = "https://remote.tailnet.example".into();
            app.open_in_editor(cx);
        })
    });
    cx.run_until_parked();
    assert!(requests.try_recv().is_err());
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.environment_scope = "http://127.0.0.1:3773".into();
            app.select_editor("file-manager".into(), cx);
        })
    });
    cx.run_until_parked();
    let connection::Command::Call {
        tag,
        payload,
        reply,
    } = requests.try_recv().unwrap()
    else {
        panic!("expected editor")
    };
    assert_eq!(tag, "shell.openInEditor");
    assert_eq!(payload["editor"], "file-manager");
    reply.send_blocking(Ok(json!({}))).unwrap();
    cx.run_until_parked();
    app.read_with(cx, |app, _| {
        assert_eq!(app.preferred_editor().as_deref(), Some("file-manager"))
    });
}

#[gpui_kit::test]
fn header_inline_rename_escape_cancels_and_enter_dispatches_title(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.begin_header_rename("Thread".into(), window, cx);
            app.header_rename
                .input
                .as_ref()
                .unwrap()
                .update(cx, |input, cx| input.set_value("Cancelled", window, cx));
        })
    });
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    assert!(requests.try_recv().is_err());
    app.read_with(cx, |app, _| assert!(app.header_rename.input.is_none()));
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.begin_header_rename("Thread".into(), window, cx);
            app.header_rename
                .input
                .as_ref()
                .unwrap()
                .update(cx, |input, cx| input.set_value("Renamed", window, cx));
        })
    });
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    let connection::Command::Call { payload, reply, .. } = requests.try_recv().unwrap() else {
        panic!("expected rename")
    };
    assert_eq!(payload["title"], "Renamed");
    reply.send_blocking(Ok(json!({}))).unwrap();
    cx.run_until_parked();
}

#[gpui_kit::test]
fn header_working_diff_scope_retains_real_patch_and_exposes_navigation(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    cx.update(|_, cx| app.update(cx, |app, cx| app.view_git_diff("working-tree", cx)));
    cx.run_until_parked();
    let connection::Command::Call {
        tag,
        payload,
        reply,
    } = requests.try_recv().unwrap()
    else {
        panic!("expected preview")
    };
    assert_eq!(tag, "review.getDiffPreview");
    assert_eq!(payload["cwd"], "/tmp/fixture");
    reply.send_blocking(Ok(json!({"cwd":"/tmp/fixture","sources":[{"kind":"working-tree","diff":"diff --git a/a b/a\n@@ -1 +1 @@\n-old\n+new\n","truncated":true}]}))).unwrap();
    cx.run_until_parked();
    app.read_with(cx, |app, _| {
        assert_eq!(app.diffs.files[0].path, "a");
        assert!(app.diffs.truncated);
        assert_eq!(app.diffs.turn_count, None);
    });
    for control in ["diff-collapse-all", "diff-expand-all", "diff-wrap"] {
        let bounds = cx.debug_bounds(control).expect("diff control is painted");
        cx.simulate_click(bounds.center(), Default::default());
        cx.run_until_parked();
        app.read_with(cx, |app, _| match control {
            "diff-collapse-all" => assert!(app.diffs.collapsed.contains(&0)),
            "diff-expand-all" => assert!(app.diffs.collapsed.is_empty()),
            _ => assert!(app.diffs.wrap),
        });
    }
}
