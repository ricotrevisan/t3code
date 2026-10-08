use super::*;
fn status(changes: bool) -> Value {
    json!({"isRepo":true,"refName":"feature","isDefaultRef":false,"hasPrimaryRemote":true,"hasUpstream":true,"aheadCount":1,"behindCount":0,"hasWorkingTreeChanges":changes,"pr":{"state":"open"},"workingTree":{"files":[{"path":"a.rs"},{"path":"b.rs"}]}})
}
fn open_git(
    app: &Entity<NativeApp>,
    cx: &mut VisualTestContext,
    requests: &Requests,
    value: Value,
) {
    let bounds = cx.debug_bounds("push-pr").unwrap();
    cx.simulate_click(bounds.center(), Default::default());
    cx.run_until_parked();
    let connection::Command::Call {
        tag,
        payload,
        reply,
    } = requests.try_recv().unwrap()
    else {
        panic!("status request")
    };
    assert_eq!(tag, "vcs.refreshStatus");
    assert_eq!(payload["cwd"], "/tmp/fixture");
    reply.send_blocking(Ok(value)).unwrap();
    cx.run_until_parked();
    app.read_with(cx, |app, _| assert!(app.git_header.open));
}
#[gpui_kit::test]
fn header_pol_git_is_anchored_and_dismisses_escape_and_outside(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    let before = cx.debug_bounds("thread-header").unwrap();
    open_git(&app, cx, &requests, status(false));
    let after = cx.debug_bounds("thread-header").unwrap();
    assert_eq!(
        before.size.height, after.size.height,
        "menu must not push header down"
    );
    assert!(cx.debug_bounds("git-menu-popover").is_some());
    let trigger = cx.debug_bounds("push-pr").unwrap();
    assert!(
        cx.debug_bounds("git-menu-popover").unwrap().origin.y >= trigger.bottom(),
        "menu is below its trigger"
    );
    cx.simulate_click(trigger.center(), Default::default());
    cx.run_until_parked();
    app.read_with(cx, |app, _| {
        assert!(!app.git_header.open, "trigger toggles closed")
    });
    open_git(&app, cx, &requests, status(false));
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    app.read_with(cx, |app, _| assert!(!app.git_header.open));
    open_git(&app, cx, &requests, status(false));
    cx.simulate_click(point(px(8.), px(950.)), Default::default());
    cx.run_until_parked();
    app.read_with(cx, |app, _| assert!(!app.git_header.open));
    assert!(requests.try_recv().is_err());
}
#[gpui_kit::test]
fn header_pol_split_push_shows_live_phases_and_hook_output_before_receipt(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    open_git(&app, cx, &requests, status(false));
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    let bounds = cx.debug_bounds("git-primary-action").unwrap();
    cx.simulate_click(bounds.center(), Default::default());
    cx.run_until_parked();
    let connection::Command::CallStreamLive {
        tag,
        payload,
        events,
    } = requests.try_recv().unwrap()
    else {
        panic!("live stream")
    };
    assert_eq!(tag, "git.runStackedAction");
    assert_eq!(payload["action"], "push");
    assert_eq!(payload["cwd"], "/tmp/fixture");
    events
        .send_blocking(Ok(Some(json!({"kind":"phase_started","label":"Pushing"}))))
        .unwrap();
    events
        .send_blocking(Ok(Some(
            json!({"kind":"hook_output","stream":"stderr","text":"running checks"}),
        )))
        .unwrap();
    cx.run_until_parked();
    app.read_with(cx, |app, _| {
        assert_eq!(app.pending_actions, 1);
        assert_eq!(
            app.git_header.progress,
            ["Pushing", "stderr: running checks"]
        );
    });
    assert!(cx.debug_bounds("git-menu-popover").is_some());
    events
        .send_blocking(Ok(Some(
            json!({"kind":"action_finished","result":{"toast":{"title":"Pushed"}}}),
        )))
        .unwrap();
    events.send_blocking(Ok(None)).unwrap();
    cx.run_until_parked();
    app.read_with(cx, |app, _| assert_eq!(app.pending_actions, 0));
}
#[gpui_kit::test]
fn header_pol_commit_form_sends_message_and_only_selected_files(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    open_git(&app, cx, &requests, status(true));
    let bounds = cx.debug_bounds("git-quick-action").unwrap();
    cx.simulate_click(bounds.center(), Default::default());
    cx.run_until_parked();
    assert!(
        requests.try_recv().is_err(),
        "opening a dialog must not commit"
    );
    cx.simulate_input("Selected change");
    cx.run_until_parked();
    let file = cx.debug_bounds("git-file-b.rs").unwrap();
    cx.simulate_click(file.center(), Default::default());
    cx.run_until_parked();
    let remaining = cx.debug_bounds("git-file-a.rs").unwrap();
    cx.simulate_click(remaining.center(), Default::default());
    cx.run_until_parked();
    let bounds = cx.debug_bounds("git-form-submit").unwrap();
    cx.simulate_click(bounds.center(), Default::default());
    cx.run_until_parked();
    assert!(
        requests.try_recv().is_err(),
        "cannot commit an empty file selection"
    );
    cx.simulate_click(remaining.center(), Default::default());
    cx.run_until_parked();
    cx.simulate_click(bounds.center(), Default::default());
    cx.run_until_parked();
    let connection::Command::CallStreamLive {
        payload, events, ..
    } = requests.try_recv().unwrap()
    else {
        panic!("commit stream")
    };
    assert_eq!(payload["commitMessage"], "Selected change");
    assert_eq!(payload["filePaths"], json!(["a.rs"]));
    events.send_blocking(Err("hook rejected".into())).unwrap();
    cx.run_until_parked();
    app.read_with(cx, |app, _| assert_eq!(app.pending_actions, 0));
}
#[gpui_kit::test]
fn header_pol_publish_discovers_provider_and_uses_real_private_publish_rpc(
    cx: &mut TestAppContext,
) {
    let (app, cx, requests) = harness(cx);
    let mut value = status(false);
    value["hasPrimaryRemote"] = json!(false);
    value["hasUpstream"] = json!(false);
    value["pr"] = Value::Null;
    open_git(&app, cx, &requests, value);
    let bounds = cx.debug_bounds("git-quick-action").unwrap();
    cx.simulate_click(bounds.center(), Default::default());
    cx.run_until_parked();
    let connection::Command::Call { tag, reply, .. } = requests.try_recv().unwrap() else {
        panic!("discovery")
    };
    assert_eq!(tag, "server.discoverSourceControl");
    reply.send_blocking(Ok(json!({"sourceControlProviders":[{"kind":"github","status":"available","auth":{"status":"authenticated"}}]}))).unwrap();
    cx.run_until_parked();
    cx.simulate_input("rico/new-repo");
    cx.run_until_parked();
    let bounds = cx.debug_bounds("git-form-submit").unwrap();
    cx.simulate_click(bounds.center(), Default::default());
    cx.run_until_parked();
    let connection::Command::Call {
        tag,
        payload,
        reply,
    } = requests.try_recv().unwrap()
    else {
        panic!("publish")
    };
    assert_eq!(tag, "sourceControl.publishRepository");
    assert_eq!(
        payload,
        json!({"cwd":"/tmp/fixture","provider":"github","repository":"rico/new-repo","visibility":"private","remoteName":"origin","protocol":"ssh"})
    );
    reply
        .send_blocking(Ok(json!({"status":"remote_added"})))
        .unwrap();
    cx.run_until_parked();
    app.read_with(cx, |app, _| {
        assert!(app.git_header.form.is_none());
        assert!(app.git_header.progress[0].contains("Remote added"));
    });
}
#[gpui_kit::test]
fn header_pol_default_branch_split_action_opens_confirmation_before_mutation(
    cx: &mut TestAppContext,
) {
    let (app, cx, requests) = harness(cx);
    let mut value = status(false);
    value["isDefaultRef"] = json!(true);
    open_git(&app, cx, &requests, value);
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    let bounds = cx.debug_bounds("git-primary-action").unwrap();
    cx.simulate_click(bounds.center(), Default::default());
    cx.run_until_parked();
    assert!(
        requests.try_recv().is_err(),
        "default-branch write must await confirmation"
    );
    let confirm = cx
        .debug_bounds("git-default-confirm")
        .expect("split action must expose its confirmation");
    cx.simulate_click(confirm.center(), Default::default());
    cx.run_until_parked();
    let connection::Command::CallStreamLive {
        payload, events, ..
    } = requests.try_recv().unwrap()
    else {
        panic!("confirmed action")
    };
    assert_eq!(payload["action"], "commit_push");
    events
        .send_blocking(Err("fixture canceled".into()))
        .unwrap();
    cx.run_until_parked();
}
#[gpui_kit::test]
fn header_pol_late_progress_is_fenced_and_disconnect_never_replays(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    open_git(&app, cx, &requests, status(false));
    let bounds = cx.debug_bounds("git-quick-action").unwrap();
    cx.simulate_click(bounds.center(), Default::default());
    cx.run_until_parked();
    let connection::Command::CallStreamLive { events, .. } = requests.try_recv().unwrap() else {
        panic!("stream")
    };
    cx.update(|_, cx| app.update(cx, |app, _| app.generation += 1));
    events
        .send_blocking(Ok(Some(
            json!({"kind":"hook_output","text":"late","stream":"stdout"}),
        )))
        .unwrap();
    cx.run_until_parked();
    app.read_with(cx, |app, _| assert!(app.git_header.progress.is_empty()));
    drop(events);
    cx.run_until_parked();
    app.read_with(cx, |app, _| assert_eq!(app.pending_actions, 0));
    assert!(
        requests.try_recv().is_err(),
        "unknown mutation outcomes are never auto-replayed"
    );
}
#[gpui_kit::test]
fn header_pol_publish_without_ready_provider_never_mutates(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    let mut value = status(false);
    value["hasPrimaryRemote"] = json!(false);
    value["hasUpstream"] = json!(false);
    value["pr"] = Value::Null;
    open_git(&app, cx, &requests, value);
    let bounds = cx.debug_bounds("git-quick-action").unwrap();
    cx.simulate_click(bounds.center(), Default::default());
    cx.run_until_parked();
    let connection::Command::Call { reply, .. } = requests.try_recv().unwrap() else {
        panic!("discovery")
    };
    reply.send_blocking(Ok(json!({"sourceControlProviders":[{"kind":"github","status":"available","auth":{"status":"unauthenticated"}},{"kind":"gitlab","status":"missing","auth":{"status":"unknown"}}]}))).unwrap();
    cx.run_until_parked();
    cx.simulate_input("rico/new-repo");
    cx.run_until_parked();
    let bounds = cx.debug_bounds("git-form-submit").unwrap();
    cx.simulate_click(bounds.center(), Default::default());
    cx.run_until_parked();
    assert!(requests.try_recv().is_err());
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    app.read_with(cx, |app, _| {
        assert!(!app.git_header.open);
        assert!(app.git_header.form.is_none());
    });
}
