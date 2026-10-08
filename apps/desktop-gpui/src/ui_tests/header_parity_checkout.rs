use super::*;

#[gpui_kit::test]
fn verified_next_checkout_label_uses_authoritative_branch(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.choose_checkout("requested".into(), Some("/tmp/verified".into()), window, cx)
        })
    });
    cx.run_until_parked();
    let connection::Command::Call {
        tag,
        payload,
        reply,
    } = requests.try_recv().unwrap()
    else {
        panic!("expected verification")
    };
    assert_eq!(tag, "vcs.listRefs");
    assert_eq!(payload["cwd"], "/tmp/verified");
    reply
        .send_blocking(Ok(
            json!({"isRepo":true,"refs":[{"name":"actual","current":true}],"nextCursor":null}),
        ))
        .unwrap();
    cx.run_until_parked();
    app.read_with(cx, |app, _| {
        assert_eq!(
            app.next_checkout_label(),
            "Next new thread: actual · /tmp/verified"
        );
        assert_eq!(app.selected_thread.as_deref(), Some("t"));
    });
}

#[gpui_kit::test]
fn next_checkout_identity_survives_current_picker_refresh(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.refs = vec![t3code_desktop_gpui::workspaces::Ref {
                name: "feature".into(),
                current: false,
                is_default: false,
                worktree_path: Some("/tmp/feature".into()),
            }];
            app.choose_checkout("feature".into(), Some("/tmp/feature".into()), window, cx);
            assert_eq!(
                app.next_checkout_label(),
                "Next new thread: feature · /tmp/feature"
            );
            app.open_branch_picker("branch-selector", window, cx);
        })
    });
    cx.run_until_parked();
    let connection::Command::Call { tag, reply, .. } = requests.try_recv().unwrap() else {
        panic!("expected refs")
    };
    assert_eq!(tag, "vcs.listRefs");
    reply.send_blocking(Ok(json!({"isRepo":true,"refs":[{"name":"feature","current":false,"worktreePath":"/tmp/feature"}],"nextCursor":null}))).unwrap();
    cx.run_until_parked();
    app.read_with(cx, |app, _| {
        assert!(app.is_next_checkout(&app.refs[0]));
        assert_eq!(
            app.next_checkout_label(),
            "Next new thread: feature · /tmp/feature"
        );
    });
}

#[gpui_kit::test]
fn new_worktree_draft_reloads_visible_inspector_refs(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.inspector_open = true;
            app.inspector_section = Some(InspectorSection::Worktrees);
            app.new_thread_checkout =
                Some(("p".into(), "feature".into(), Some("/tmp/feature".into())));
            app.new_thread(window, cx);
        })
    });
    cx.run_until_parked();
    assert!(matches!(
        requests.try_recv().unwrap(),
        connection::Command::Subscribe { .. }
    ));
    assert!(matches!(
        requests.try_recv().unwrap(),
        connection::Command::SelectThread(None)
    ));
    let connection::Command::Call {
        tag,
        payload,
        reply,
    } = requests.try_recv().unwrap()
    else {
        panic!("expected reload")
    };
    assert_eq!(tag, "vcs.listRefs");
    assert_eq!(payload["cwd"], "/tmp/feature");
    reply
        .send_blocking(Ok(
            json!({"isRepo":true,"refs":[{"name":"feature","current":true}],"nextCursor":null}),
        ))
        .unwrap();
    cx.run_until_parked();
    app.read_with(cx, |app, _| assert_eq!(app.refs[0].name, "feature"));
}
