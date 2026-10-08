use super::*;
fn click(cx: &mut VisualTestContext, selector: &'static str) {
    let bounds = cx
        .debug_bounds(selector)
        .unwrap_or_else(|| panic!("missing {selector}"));
    cx.simulate_click(bounds.center(), Default::default());
    cx.run_until_parked();
}
fn preview(diff: &str, truncated: bool) -> Value {
    json!({"cwd":"/tmp/fixture","sources":[{"kind":"working-tree","baseRef":"HEAD","headRef":null,"diff":diff,"truncated":truncated,
        "files":[{"path":"a.rs","previousPath":null,"additions":1,"deletions":1},{"path":"renamed.rs","previousPath":"old.rs","additions":3,"deletions":2}]}]})
}
const PATCH: &str =
    "diff --git a/a.rs b/a.rs\n--- a/a.rs\n+++ b/a.rs\n@@ -1 +1 @@\n-fn old() {}\n+fn new() {}\n";
const RENAMED: &str = "diff --git a/old.rs b/renamed.rs\nrename from old.rs\nrename to renamed.rs\n--- a/old.rs\n+++ b/renamed.rs\n@@ -1 +1 @@\n-fn old() {}\n+fn new() {}\n";

#[gpui_kit::test]
fn r16_e_initialize_git_denial_retry_and_receipt_refresh(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    super::r14_e_header::ensure_control(cx, "push-pr");
    click(cx, "push-pr");
    let connection::Command::Call { tag, reply, .. } = requests.try_recv().unwrap() else {
        panic!()
    };
    assert_eq!(tag, "vcs.refreshStatus");
    reply.send_blocking(Ok(json!({"isRepo":false}))).unwrap();
    cx.run_until_parked();
    click(cx, "git-quick-action");
    let connection::Command::Call {
        tag,
        payload,
        reply,
    } = requests.try_recv().unwrap()
    else {
        panic!()
    };
    assert_eq!(tag, "vcs.init");
    assert_eq!(payload, json!({"cwd":"/tmp/fixture"}));
    reply
        .send_blocking(Err("Initialization denied".into()))
        .unwrap();
    cx.run_until_parked();
    assert_eq!(app.read_with(cx, |app, _| app.pending_actions), 0);
    assert_eq!(
        app.read_with(cx, |app, _| app.git_header.error.clone()),
        Some("Initialization denied".into())
    );
    click(cx, "git-quick-action");
    let connection::Command::Call { tag, reply, .. } = requests.try_recv().unwrap() else {
        panic!()
    };
    assert_eq!(tag, "vcs.init");
    reply.send_blocking(Ok(json!({}))).unwrap();
    cx.run_until_parked();
    let connection::Command::Call { tag, reply, .. } = requests.try_recv().unwrap() else {
        panic!()
    };
    assert_eq!(tag, "vcs.refreshStatus");
    reply
        .send_blocking(Ok(
            json!({"isRepo":true,"refName":"main","hasWorkingTreeChanges":false}),
        ))
        .unwrap();
    cx.run_until_parked();
    assert_eq!(
        app.read_with(cx, |app, _| {
            app.git_header.status.clone().unwrap()["isRepo"].clone()
        }),
        true
    );
    assert_eq!(app.read_with(cx, |app, _| app.pending_actions), 0);
}

#[gpui_kit::test]
fn r16_e_commit_new_branch_keeps_message_selected_paths_and_unknown_outcome(
    cx: &mut TestAppContext,
) {
    let (app, cx, requests) = harness(cx);
    super::r14_e_header::ensure_control(cx, "push-pr");
    click(cx, "push-pr");
    let connection::Command::Call { reply, .. } = requests.try_recv().unwrap() else {
        panic!()
    };
    reply.send_blocking(Ok(json!({"isRepo":true,"refName":"feature","hasWorkingTreeChanges":true,"workingTree":{"files":[{"path":"a.rs"},{"path":"b.rs"}]}}))).unwrap();
    cx.run_until_parked();
    click(cx, "git-quick-action");
    cx.simulate_input("Selected commit on feature branch");
    cx.run_until_parked();
    click(cx, "git-file-b.rs");
    click(cx, "git-form-new-branch");
    let connection::Command::CallStreamLive {
        tag,
        payload,
        events,
    } = requests.try_recv().unwrap()
    else {
        panic!()
    };
    assert_eq!(tag, "git.runStackedAction");
    assert_eq!(payload["action"], "commit");
    assert_eq!(payload["featureBranch"], true);
    assert_eq!(payload["filePaths"], json!(["a.rs"]));
    assert_eq!(
        payload["commitMessage"],
        "Selected commit on feature branch"
    );
    assert_eq!(payload["cwd"], "/tmp/fixture");
    assert_eq!(payload["threadId"], "t");
    drop(events);
    cx.run_until_parked();
    assert_eq!(app.read_with(cx, |app, _| app.pending_actions), 0);
    assert!(app.read_with(cx, |app, _| {
        app.git_header
            .error
            .as_ref()
            .is_some_and(|error| error.contains("outcome unknown"))
    }));
    assert!(
        requests.is_empty(),
        "unknown outcome must not auto-replay a commit"
    );
}

#[gpui_kit::test]
fn r16_e_ten_thousand_file_headings_only_prepare_visible_buttons_and_cache(
    cx: &mut TestAppContext,
) {
    let (app, cx, requests) = harness(cx);
    cx.update(|_, cx| app.update(cx, |app, cx| app.view_git_diff("working-tree", cx)));
    cx.run_until_parked();
    let connection::Command::Call { reply, .. } = requests.try_recv().unwrap() else {
        panic!()
    };
    let mut value = preview(PATCH, true);
    value["sources"][0]["files"] = (0..10000).map(|index| json!({"path":format!("file-{index}.rs"),"previousPath":null,"additions":1,"deletions":0})).collect::<Vec<_>>().into();
    reply.send_blocking(Ok(value)).unwrap();
    cx.run_until_parked();
    let connection::Command::Call { reply, .. } = requests.try_recv().unwrap() else {
        panic!()
    };
    reply.send_blocking(Err("Patch denied".into())).unwrap();
    cx.run_until_parked();
    let list = app.read_with(cx, |app, _| app.diffs.file_list.get().unwrap().clone());
    let count = list.read_with(cx, |list, _| list.constructions.get());
    assert!(count > 0 && count < 100, "only viewport headings: {count}");
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.composer.update(cx, |input, cx| {
                input.set_value("Unrelated composer edit", window, cx)
            });
            cx.notify();
        })
    });
    cx.run_until_parked();
    assert_eq!(
        list.read_with(cx, |list, _| list.constructions.get()),
        count
    );
    assert_eq!(app.read_with(cx, |app, _| app.diffs.files.len()), 10000);
}

#[gpui_kit::test]
fn r16_e_manifest_omitted_file_retry_full_contents_and_stale_navigation(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    cx.update(|_, cx| app.update(cx, |app, cx| app.view_git_diff("working-tree", cx)));
    cx.run_until_parked();
    let connection::Command::Call { tag, reply, .. } = requests.try_recv().unwrap() else {
        panic!()
    };
    assert_eq!(tag, "review.getDiffPreview");
    reply.send_blocking(Ok(preview(PATCH, true))).unwrap();
    cx.run_until_parked();
    let connection::Command::Call { payload, reply, .. } = requests.try_recv().unwrap() else {
        panic!()
    };
    assert_eq!(
        payload["file"],
        json!({"path":"a.rs","previousPath":null,"sourceKind":"working-tree"})
    );
    reply
        .send_blocking(Err("Denied file patch".into()))
        .unwrap();
    cx.run_until_parked();
    assert_eq!(app.read_with(cx, |app, _| app.diffs.files.len()), 2);
    assert!(
        cx.debug_bounds("diff-file-1").is_some(),
        "omitted file is navigable"
    );
    click(cx, "diff-file-retry");
    let connection::Command::Call { reply: stale, .. } = requests.try_recv().unwrap() else {
        panic!()
    };
    click(cx, "diff-file-1");
    let connection::Command::Call {
        tag,
        payload,
        reply,
        ..
    } = requests.try_recv().unwrap()
    else {
        panic!()
    };
    assert_eq!(tag, "review.getDiffPreview");
    assert_eq!(
        payload["file"],
        json!({"path":"renamed.rs","previousPath":"old.rs","sourceKind":"working-tree"})
    );
    assert_eq!(payload["baseRef"], "HEAD");
    let _ = stale.send_blocking(Ok(preview(PATCH, false)));
    reply.send_blocking(Ok(preview(RENAMED, false))).unwrap();
    cx.run_until_parked();
    app.read_with(cx, |app, _| {
        assert_eq!(app.diffs.selected_file, 1);
        assert_eq!(app.diffs.files[1].patch, RENAMED);
        assert!(!app.diffs.files[0].loaded);
        assert!(app.diffs.patch_error.is_none());
    });
    click(cx, "diff-full-file");
    let connection::Command::Call {
        tag,
        payload,
        reply,
    } = requests.try_recv().unwrap()
    else {
        panic!()
    };
    assert_eq!(tag, "review.getDiffFileContents");
    assert_eq!(payload["changeType"], "rename-changed");
    assert_eq!(payload["oldPath"], "old.rs");
    assert_eq!(payload["newPath"], "renamed.rs");
    reply
        .send_blocking(Ok(
            json!({"oldContents":"fn old() {}\n","newContents":"fn new() {}\n"}),
        ))
        .unwrap();
    cx.run_until_parked();
    click(cx, "copy-full-1");
    assert_eq!(
        cx.update(|_, cx| cx.read_from_clipboard().unwrap().text().unwrap()),
        "fn new() {}\n"
    );
    click(cx, "diff-close-full");
    assert!(app.read_with(cx, |app, _| app.diffs.full_file_view.is_none()));
}

#[gpui_kit::test]
fn r16_e_comparison_search_remote_selection_and_automatic_reverse(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    cx.update(|_, cx| app.update(cx, |app, cx| app.view_git_diff("branch-range", cx)));
    cx.run_until_parked();
    let connection::Command::Call { reply, .. } = requests.try_recv().unwrap() else {
        panic!()
    };
    reply.send_blocking(Ok(json!({"cwd":"/tmp/fixture","sources":[{"kind":"branch-range","baseRef":"main","headRef":"feature","diff":PATCH}]}))).unwrap();
    cx.run_until_parked();
    click(cx, "diff-base-ref");
    let connection::Command::Call {
        tag,
        payload,
        reply,
    } = requests.try_recv().unwrap()
    else {
        panic!()
    };
    assert_eq!(tag, "vcs.listRefs");
    assert_eq!(payload["refKind"], "all");
    assert_eq!(payload["includeMatchingRemoteRefs"], true);
    assert_eq!(payload["cwd"], "/tmp/fixture");
    reply
        .send_blocking(Ok(json!({"refs":[],"nextCursor":null})))
        .unwrap();
    cx.run_until_parked();
    cx.simulate_input("release");
    cx.run_until_parked();
    let connection::Command::Call { payload, reply, .. } = requests.try_recv().unwrap() else {
        panic!()
    };
    assert_eq!(payload["query"], "release");
    reply.send_blocking(Ok(json!({"refs":[{"name":"release","isRemote":false},{"name":"origin/release","isRemote":true}],"nextCursor":null}))).unwrap();
    cx.run_until_parked();
    click(cx, "diff-ref-row-1");
    let connection::Command::Call {
        tag,
        payload,
        reply,
    } = requests.try_recv().unwrap()
    else {
        panic!()
    };
    assert_eq!(tag, "review.getDiffPreview");
    assert_eq!(payload["baseRef"], "origin/release");
    reply.send_blocking(Ok(json!({"cwd":"/tmp/fixture","sources":[{"kind":"branch-range","baseRef":"origin/release","headRef":"feature","diff":PATCH}]}))).unwrap();
    cx.run_until_parked();
    click(cx, "diff-base-ref");
    let connection::Command::Call { reply, .. } = requests.try_recv().unwrap() else {
        panic!()
    };
    reply
        .send_blocking(Ok(json!({"refs":[],"nextCursor":null})))
        .unwrap();
    cx.run_until_parked();
    click(cx, "diff-ref-automatic");
    let connection::Command::Call { payload, reply, .. } = requests.try_recv().unwrap() else {
        panic!()
    };
    assert!(payload.get("baseRef").is_none());
    reply
        .send_blocking(Err("Comparison denied".into()))
        .unwrap();
    cx.run_until_parked();
    assert_eq!(
        app.read_with(cx, |app, _| app.diffs.error.clone()),
        Some("Comparison denied".into())
    );
    assert!(cx.debug_bounds("diff-retry").is_some());
}

#[gpui_kit::test]
fn r16_e_title_pending_click_double_click_blur_and_keyboard(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    click(cx, "header-title");
    assert!(!app.read_with(cx, |app, _| app.thread_menu_open));
    let position = cx.debug_bounds("header-title").unwrap().center();
    cx.simulate_event(gpui_kit::MouseDownEvent {
        position,
        modifiers: Default::default(),
        button: MouseButton::Left,
        click_count: 2,
        first_mouse: false,
    });
    cx.simulate_event(gpui_kit::MouseUpEvent {
        position,
        modifiers: Default::default(),
        button: MouseButton::Left,
        click_count: 2,
    });
    cx.run_until_parked();
    assert!(app.read_with(cx, |app, _| app.header_rename.input.is_some()));
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(600));
    cx.run_until_parked();
    assert!(!app.read_with(cx, |app, _| app.thread_menu_open));
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    assert!(requests.is_empty());
    click(cx, "header-title");
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.composer.update(cx, |input, cx| input.focus(window, cx))
        })
    });
    cx.run_until_parked();
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(600));
    cx.run_until_parked();
    assert!(!app.read_with(cx, |app, _| app.thread_menu_open));
    click(cx, "header-title");
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    assert!(app.read_with(cx, |app, _| app.thread_menu_open));
    click(cx, "header-title-actions");
    assert!(!app.read_with(cx, |app, _| app.thread_menu_open));
    click(cx, "header-title");
    assert!(!app.read_with(cx, |app, _| app.thread_menu_open));
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(500));
    cx.run_until_parked();
    assert!(
        app.read_with(cx, |app, _| app.thread_menu_open),
        "single click opens after the native double-click window"
    );
    assert!(requests.is_empty());
}
