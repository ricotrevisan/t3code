use super::*;

pub(super) fn ensure_overflow(cx: &mut VisualTestContext) {
    if cx.debug_bounds("header-overflow-panel").is_none() {
        let trigger = cx
            .debug_bounds("header-overflow")
            .expect("header overflow trigger");
        cx.simulate_click(trigger.center(), Default::default());
        cx.run_until_parked();
    }
}

pub(super) fn ensure_control(cx: &mut VisualTestContext, selector: &'static str) {
    if cx.debug_bounds(selector).is_none() {
        ensure_overflow(cx);
    }
}

#[gpui_kit::test]
fn r14_e_large_patch_and_full_file_are_virtualized_and_isolated(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    cx.update(|_, cx| app.update(cx, |app, cx| app.view_git_diff("working-tree", cx)));
    cx.run_until_parked();
    let connection::Command::Call {
        tag,
        payload,
        reply,
    } = requests.try_recv().unwrap()
    else {
        panic!("preview request")
    };
    assert_eq!(tag, "review.getDiffPreview");
    assert_eq!(payload["cwd"], "/tmp/fixture");
    let mut patch =
        String::from("diff --git a/a.rs b/a.rs\n--- a/a.rs\n+++ b/a.rs\n@@ -1,20000 +1,20000 @@\n");
    for index in 0..20000 {
        patch.push_str(&format!("-old {index}\n+new {index}\n"));
    }
    reply.send_blocking(Ok(json!({"cwd":"/tmp/fixture","sources":[{"kind":"working-tree","baseRef":"HEAD","headRef":null,"diff":patch}]}))).unwrap();
    cx.run_until_parked();
    let entity = app.read_with(cx, |app, _| app.diffs.files[0].view.get().unwrap().clone());
    let initial = entity.read_with(cx, |view, _| view.constructions.get());
    assert!(initial > 0 && initial < 200, "visible rows only: {initial}");
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
        entity.read_with(cx, |view, _| view.constructions.get()),
        initial,
        "cached patch subtree"
    );
    let split = cx.debug_bounds("diff-split").unwrap();
    cx.simulate_click(split.center(), Default::default());
    cx.run_until_parked();
    let split_count = entity.read_with(cx, |view, _| view.constructions.get());
    assert!(split_count > initial && split_count < 400);
    let full = cx.debug_bounds("diff-full-file").unwrap();
    cx.simulate_click(full.center(), Default::default());
    cx.run_until_parked();
    let connection::Command::Call {
        tag,
        payload,
        reply,
    } = requests.try_recv().unwrap()
    else {
        panic!("file request")
    };
    assert_eq!(tag, "review.getDiffFileContents");
    assert_eq!(payload["newPath"], "a.rs");
    let body = (0..30000)
        .map(|i| format!("complete {i}\n"))
        .collect::<String>();
    reply
        .send_blocking(Ok(json!({"oldContents":body,"newContents":body})))
        .unwrap();
    cx.run_until_parked();
    let full = app.read_with(cx, |app, _| {
        app.diffs.full_file_view.as_ref().unwrap().clone()
    });
    let initial = full.read_with(cx, |view, _| view.constructions.get());
    assert!(
        initial > 0 && initial < 200,
        "complete-file visible rows: {initial}"
    );
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.composer.update(cx, |input, cx| {
                input.set_value("Another unrelated edit", window, cx)
            });
            cx.notify();
        })
    });
    cx.run_until_parked();
    assert_eq!(
        full.read_with(cx, |view, _| view.constructions.get()),
        initial
    );
    let copy = cx.debug_bounds("copy-full-1").expect("complete-file copy");
    cx.simulate_click(copy.center(), Default::default());
    cx.run_until_parked();
    assert_eq!(
        cx.update(|_, cx| cx.read_from_clipboard().unwrap().text().unwrap()),
        body
    );
    let close = cx.debug_bounds("diff-close-full").unwrap();
    cx.simulate_click(close.center(), Default::default());
    cx.run_until_parked();
    app.read_with(cx, |app, _| {
        assert!(app.diffs.full_file.is_none());
        assert!(app.diffs.full_file_view.is_none());
    });
    let refresh = cx.debug_bounds("diff-refresh").unwrap();
    cx.simulate_click(refresh.center(), Default::default());
    cx.run_until_parked();
    let connection::Command::Call { tag, reply, .. } = requests.try_recv().unwrap() else {
        panic!("new revision request")
    };
    assert_eq!(tag, "review.getDiffPreview");
    reply.send_blocking(Ok(json!({"cwd":"/tmp/fixture","sources":[{"kind":"working-tree","baseRef":"HEAD","headRef":null,"diff":"diff --git a/a.rs b/a.rs\n--- a/a.rs\n+++ b/a.rs\n@@ -1 +1 @@\n-old revision\n+new revision\n"}]}))).unwrap();
    cx.run_until_parked();
    let replacement = app.read_with(cx, |app, _| app.diffs.files[0].view.get().unwrap().clone());
    assert_ne!(
        replacement.entity_id(),
        entity.entity_id(),
        "new receipt retires previous revision cache"
    );
    assert!(replacement.read_with(cx, |view, _| view.constructions.get()) > 0);
}

#[gpui_kit::test]
fn r14_e_header_narrow_overflow_dismisses_and_preserves_identity(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    cx.simulate_resize(size(px(960.), px(1000.)));
    cx.run_until_parked();
    let header = cx.debug_bounds("thread-header").unwrap();
    assert!(header.size.height <= px(48.), "one compact row: {header:?}");
    let title = cx.debug_bounds("header-title").unwrap();
    let overflow = cx.debug_bounds("header-overflow").unwrap();
    assert!(title.right() <= overflow.left());
    ensure_overflow(cx);
    assert!(cx.debug_bounds("open-editor").is_some());
    assert!(cx.debug_bounds("push-pr").is_some());
    assert!(cx.debug_bounds("toggle-inspector").is_some());
    let bounds = cx.debug_bounds("header-overflow-panel").unwrap();
    cx.simulate_click(
        point(bounds.left() + px(4.), bounds.top() + px(4.)),
        Default::default(),
    );
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    assert!(cx.debug_bounds("header-overflow-panel").is_none());
    ensure_overflow(cx);
    cx.simulate_click(point(px(8.), px(950.)), Default::default());
    cx.run_until_parked();
    assert!(cx.debug_bounds("header-overflow-panel").is_none());
    assert!(requests.is_empty());
    app.read_with(cx, |app, _| {
        assert_eq!(app.selected_thread.as_deref(), Some("t"))
    });
}
