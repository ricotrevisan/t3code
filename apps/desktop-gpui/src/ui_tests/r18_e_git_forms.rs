use super::*;

fn click(cx: &mut VisualTestContext, selector: &'static str) {
    let bounds = cx
        .debug_bounds(selector)
        .unwrap_or_else(|| panic!("missing {selector}"));
    cx.simulate_click(bounds.center(), Default::default());
    cx.run_until_parked();
}
fn open_form(cx: &mut VisualTestContext, requests: &super::r13_int_requests::Requests) {
    super::r14_e_header::ensure_control(cx, "push-pr");
    click(cx, "push-pr");
    let connection::Command::Call { tag, reply, .. } = requests.try_recv().unwrap() else {
        panic!()
    };
    assert_eq!(tag, "vcs.refreshStatus");
    reply.send_blocking(Ok(json!({"isRepo":true,"refName":"feature","hasWorkingTreeChanges":true,"workingTree":{"files":[{"path":"a.rs","insertions":3,"deletions":1},{"path":"b.rs","insertions":2,"deletions":0}]}}))).unwrap();
    cx.run_until_parked();
    click(cx, "git-quick-action");
}

#[gpui_kit::test]
fn r18_e_multiline_message_selection_and_hook_denial_do_not_replay(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    open_form(cx, &requests);
    cx.simulate_input("Summary 🦀");
    cx.simulate_keystrokes("enter");
    cx.simulate_keystrokes("enter");
    cx.simulate_input("Body with details");
    cx.run_until_parked();
    assert!(
        requests.is_empty(),
        "Enter edits the message; it never commits"
    );
    click(cx, "git-files-all");
    click(cx, "git-form-submit");
    assert!(requests.is_empty(), "No files selected cannot commit");
    click(cx, "git-file-a.rs");
    click(cx, "git-form-submit");
    let connection::Command::CallStreamLive {
        tag,
        payload,
        events,
    } = requests.try_recv().unwrap()
    else {
        panic!()
    };
    assert_eq!(tag, "git.runStackedAction");
    assert_eq!(payload["commitMessage"], "Summary 🦀\n\nBody with details");
    assert_eq!(payload["filePaths"], json!(["a.rs"]));
    assert_eq!(payload["cwd"], "/tmp/fixture");
    events
        .send_blocking(Err("Hook rejected the commit".into()))
        .unwrap();
    cx.run_until_parked();
    app.read_with(cx, |app, _| {
        assert_eq!(app.pending_actions, 0);
        assert!(
            app.git_header
                .error
                .as_ref()
                .unwrap()
                .contains("Hook rejected")
        );
    });
    assert!(requests.is_empty(), "Denied mutation must not auto-replay");
}

#[gpui_kit::test]
fn r18_e_changed_file_editor_denial_preserves_form_and_cancel_reopens_clean(
    cx: &mut TestAppContext,
) {
    let (app, cx, requests) = harness(cx);
    cx.update(|_, cx| app.update(cx, |app, _| app.available_editors = vec!["zed".into()]));
    open_form(cx, &requests);
    cx.simulate_input("Keep this message");
    click(cx, "git-file-b.rs");
    click(cx, "git-file-editor-a.rs");
    let connection::Command::Call {
        tag,
        payload,
        reply,
    } = requests.try_recv().unwrap()
    else {
        panic!()
    };
    assert_eq!(tag, "shell.openInEditor");
    assert_eq!(payload, json!({"cwd":"/tmp/fixture/a.rs","editor":"zed"}));
    reply
        .send_blocking(Err("Editor unavailable".into()))
        .unwrap();
    cx.run_until_parked();
    app.read_with(cx, |app, cx| {
        let form = app.git_header.form.as_ref().unwrap();
        assert_eq!(
            form.message.as_ref().unwrap().read(cx).value(),
            "Keep this message"
        );
        assert_eq!(form.excluded, vec!["b.rs"]);
    });
    click(cx, "git-form-cancel");
    assert!(app.read_with(cx, |app, _| app.git_header.form.is_none()));
    click(cx, "git-quick-action");
    app.read_with(cx, |app, cx| {
        let form = app.git_header.form.as_ref().unwrap();
        assert!(form.message.as_ref().unwrap().read(cx).value().is_empty());
        assert!(form.excluded.is_empty());
    });
    assert!(requests.is_empty());
}

#[gpui_kit::test]
fn r18_e_message_contract_limit_and_stale_file_target_preserve_draft(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    open_form(cx, &requests);
    let message = "🦀".repeat(5001);
    cx.simulate_input(&message);
    cx.run_until_parked();
    click(cx, "git-form-submit");
    assert!(requests.is_empty());
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.submit_git_form(cx);
            assert!(app.git_header.form.is_some());
            assert!(app.git_header.error.as_ref().unwrap().contains("10,000"));
            app.selected_thread = None;
            app.open_git_form_file_editor("a.rs", cx);
        })
    });
    cx.run_until_parked();
    assert!(
        requests.is_empty(),
        "A stale form cannot open a file in the new checkout"
    );
    app.read_with(cx, |app, cx| {
        assert_eq!(
            app.git_header
                .form
                .as_ref()
                .unwrap()
                .message
                .as_ref()
                .unwrap()
                .read(cx)
                .value(),
            message
        )
    });
}

#[gpui_kit::test]
fn r18_e_changed_file_editor_rejects_checkout_escape(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.available_editors = vec!["zed".into()];
            for path in ["../secret", "/etc/passwd", "\\secret", "C:\\secret"] {
                app.open_checkout_file_editor(path, cx);
            }
        })
    });
    cx.run_until_parked();
    assert!(requests.is_empty());
}
