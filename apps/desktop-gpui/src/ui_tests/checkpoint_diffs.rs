use super::*;

fn seed_checkpoints(app: &Entity<NativeApp>, cx: &mut VisualTestContext) {
    cx.update(|_, cx| app.update(cx, |app, cx| {
        app.detail.thread = Some(json!({"id":"t", "messages":[], "activities":[], "proposedPlans":[],
            "checkpoints":[
                {"checkpointTurnCount":1,"turnId":"one","completedAt":"2026-01-01T00:00:00Z","status":"ready","files":[{"path":"a.rs","additions":1,"deletions":1}]},
                {"checkpointTurnCount":2,"turnId":"two","completedAt":"2026-01-01T00:01:00Z","status":"ready","files":[{"path":"b.rs","additions":1,"deletions":1}]}
            ]}));
        app.right_tab = RightTab::Changes;
        cx.notify();
    }));
    cx.run_until_parked();
}

fn click_diff(cx: &mut VisualTestContext, selector: &'static str) {
    let bounds = cx
        .debug_bounds(selector)
        .unwrap_or_else(|| panic!("missing {selector}"));
    cx.simulate_click(bounds.center(), Default::default());
    cx.run_until_parked();
}

fn diff_request(
    requests: &async_channel::Receiver<connection::Command>,
    turn: u64,
) -> async_channel::Sender<Result<Value, String>> {
    let connection::Command::Call {
        tag,
        payload,
        reply,
    } = requests.try_recv().unwrap()
    else {
        panic!("expected checkpoint RPC");
    };
    assert_eq!(tag, "orchestration.getTurnDiff");
    assert_eq!(
        payload,
        json!({"threadId":"t","fromTurnCount":turn-1,"toTurnCount":turn})
    );
    reply
}

#[gpui_kit::test]
fn checkpoint_ranges_files_and_stale_replies(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    seed_checkpoints(&app, cx);
    click_diff(cx, "diff-checkpoint-1");
    assert!(cx.debug_bounds("diff-loading").is_some());
    let first = diff_request(&requests, 1);
    click_diff(cx, "diff-checkpoint-2");
    let second = diff_request(&requests, 2);
    let patch = "diff --git a/b.rs b/b.rs\n--- a/b.rs\n+++ b/b.rs\n@@ -1 +1 @@\n-before-two\n+after-two\ndiff --git a/c.rs b/c.rs\n--- a/c.rs\n+++ b/c.rs\n@@ -1 +1 @@\n-old-c\n+new-c\n";
    second
        .send_blocking(Ok(
            json!({"threadId":"t","fromTurnCount":1,"toTurnCount":2,"diff":patch}),
        ))
        .unwrap();
    cx.run_until_parked();
    assert!(cx.debug_bounds("diff-patch").is_some());
    assert!(app.read_with(cx, |app, _| {
        app.diffs.files[0]
            .patch
            .contains("@@ -1 +1 @@\n-before-two\n+after-two")
    }));
    click_diff(cx, "diff-file-1");
    assert_eq!(
        app.read_with(cx, |app, _| app.diffs.files[app.diffs.selected_file]
            .patch
            .clone()),
        "diff --git a/c.rs b/c.rs\n--- a/c.rs\n+++ b/c.rs\n@@ -1 +1 @@\n-old-c\n+new-c\n"
    );
    // The superseded task may have dropped its reply receiver.
    let _ = first.send_blocking(Ok(
        json!({"threadId":"t","fromTurnCount":0,"toTurnCount":1,"diff":"stale"}),
    ));
    cx.run_until_parked();
    assert_eq!(app.read_with(cx, |app, _| app.diffs.turn_count), Some(2));
    click_diff(cx, "diff-checkpoint-1");
    let first = diff_request(&requests, 1);
    first.send_blocking(Ok(json!({"threadId":"t","fromTurnCount":0,"toTurnCount":1,"diff":"diff --git a/a.rs b/a.rs\n@@ -1 +1 @@\n-before-one\n+after-one\n"}))).unwrap();
    cx.run_until_parked();
    assert!(app.read_with(cx, |app, _| {
        app.diffs.files[0].patch.contains("-before-one\n+after-one")
    }));
    // A different selected thread must hide the old patch and reject its reply.
    click_diff(cx, "diff-checkpoint-2");
    let delayed = diff_request(&requests, 2);
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.selected_thread = Some("other".into());
            cx.notify();
        })
    });
    delayed
        .send_blocking(Ok(
            json!({"threadId":"t","fromTurnCount":1,"toTurnCount":2,"diff":patch}),
        ))
        .unwrap();
    cx.run_until_parked();
    assert!(cx.debug_bounds("diff-patch").is_none());
    assert!(app.read_with(cx, |app, _| app.diffs.files.is_empty()));
}

#[gpui_kit::test]
fn checkpoint_error_retry_and_empty_patch(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    seed_checkpoints(&app, cx);
    click_diff(cx, "diff-checkpoint-1");
    diff_request(&requests, 1)
        .send_blocking(Err("Checkpoint unavailable".into()))
        .unwrap();
    cx.run_until_parked();
    assert_eq!(
        app.read_with(cx, |app, _| app.diffs.error.clone())
            .as_deref(),
        Some("Checkpoint unavailable")
    );
    click_diff(cx, "diff-retry");
    assert!(cx.debug_bounds("diff-loading").is_some());
    diff_request(&requests, 1)
        .send_blocking(Ok(
            json!({"threadId":"t","fromTurnCount":0,"toTurnCount":1,"diff":""}),
        ))
        .unwrap();
    cx.run_until_parked();
    assert!(cx.debug_bounds("diff-empty").is_some());
    assert!(cx.debug_bounds("diff-retry").is_none());
    assert!(cx.debug_bounds("diff-loading").is_none());
}

#[gpui_kit::test]
fn timeline_changes_opens_same_panel_and_header_preserves_tab(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    seed_checkpoints(&app, cx);
    click_diff(cx, "view-checkpoint-2");
    assert_eq!(app.read_with(cx, |app, _| app.right_tab), RightTab::Changes);
    diff_request(&requests, 2).send_blocking(Ok(json!({"threadId":"t","fromTurnCount":1,"toTurnCount":2,"diff":"diff --git a/b.rs b/b.rs\n@@ -1 +1 @@\n-old\n+new\n"}))).unwrap();
    cx.run_until_parked();
    assert!(cx.debug_bounds("diff-patch").is_some());
    click_diff(cx, "toggle-inspector");
    assert!(!app.read_with(cx, |app, _| app.inspector_open));
    click_diff(cx, "toggle-inspector");
    assert_eq!(app.read_with(cx, |app, _| app.right_tab), RightTab::Changes);
    assert!(cx.debug_bounds("diff-patch").is_some());
    click_diff(cx, "copy-diff");
    assert!(cx.read_from_clipboard().unwrap().text().unwrap().contains("-old\n+new\n"));
    assert!(requests.try_recv().is_err());
}

#[gpui_kit::test]
fn checkout_selection_retires_pending_and_completed_patches(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    seed_second_thread(&app, cx, false);
    seed_checkpoints(&app, cx);
    click_diff(cx, "diff-checkpoint-1");
    let delayed = diff_request(&requests, 1);
    cx.update(|window, cx| app.update(cx, |app, cx| {
        app.select_thread("u", window, cx);
        app.select_thread("t", window, cx);
    }));
    cx.run_until_parked();
    while requests.try_recv().is_ok() {}
    let _ = delayed.send_blocking(Ok(json!({"threadId":"t","fromTurnCount":0,"toTurnCount":1,"diff":"obsolete patch"})));
    cx.run_until_parked();
    assert!(app.read_with(cx, |app, _| app.diffs.files.is_empty() && !app.diffs.loading && app.diffs.turn_count.is_none()));
    seed_checkpoints(&app, cx);
    click_diff(cx, "diff-checkpoint-1");
    diff_request(&requests, 1).send_blocking(Ok(json!({"threadId":"t","fromTurnCount":0,"toTurnCount":1,"diff":"current patch"}))).unwrap();
    cx.run_until_parked();
    assert!(cx.debug_bounds("diff-patch").is_some());
    cx.update(|_, cx| app.update(cx, |app, cx| app.invalidate_checkout_context(cx)));
    cx.run_until_parked();
    assert!(cx.debug_bounds("diff-patch").is_none());
    assert!(app.read_with(cx, |app, _| app.diffs.files.is_empty()));
}

#[gpui_kit::test]
fn checkpoint_reply_must_match_the_requested_thread_and_range(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    seed_checkpoints(&app, cx);
    for reply in [
        json!({"threadId":"other","fromTurnCount":0,"toTurnCount":1,"diff":"wrong thread"}),
        json!({"threadId":"t","fromTurnCount":0,"toTurnCount":2,"diff":"wrong range"}),
        json!({"threadId":"t","diff":"malformed"}),
    ] {
        click_diff(cx, "diff-checkpoint-1");
        diff_request(&requests, 1).send_blocking(Ok(reply)).unwrap();
        cx.run_until_parked();
        assert!(app.read_with(cx, |app, _| app.diffs.error.is_some() && app.diffs.files.is_empty()));
        assert!(cx.debug_bounds("diff-retry").is_some());
    }
}
