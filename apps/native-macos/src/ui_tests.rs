//! Headless GPUI interactions: real input dispatch and asynchronous receipts, not markup assertions.
use super::*;
use gpui::{AppContext, TestAppContext, VisualTestContext};

fn harness(
    cx: &mut TestAppContext,
) -> (
    Entity<NativeApp>,
    &mut VisualTestContext,
    async_channel::Receiver<connection::Command>,
) {
    cx.update(gpui_component::init);
    let mut app = None;
    let (_, cx) = cx.add_window_view(|window, cx| {
        let entity = cx.new(|cx| NativeApp::new(window, cx, false));
        app = Some(entity.clone());
        Root::new(entity, window, cx)
    });
    cx.simulate_resize(size(px(1280.), px(1000.)));
    let app = app.unwrap();
    let (sender, requests) = async_channel::bounded(16);
    cx.update(|window,cx| app.update(cx,|app,cx| {
        app.shell.apply(serde_json::from_value(json!({"kind":"snapshot","snapshot":{"snapshotSequence":0,"projects":[{"id":"p","title":"Fixture","workspaceRoot":"/tmp/fixture"}],"threads":[{"id":"t","projectId":"p","title":"Thread","modelSelection":{"instanceId":"codex","model":"gpt-5.4"},"runtimeMode":"full-access","interactionMode":"default","archivedAt":null}]}})).unwrap()).unwrap();
        app.commands = Some(sender);
        app.select_thread("t",window,cx);
        app.composer.update(cx,|input,cx| input.focus(window,cx));
        cx.notify();
    }));
    // Selection request belongs to setup, not the action under test.
    assert!(
        matches!(requests.try_recv().unwrap(),connection::Command::SelectThread(Some(id)) if id == "t")
    );
    cx.run_until_parked();
    (app, cx, requests)
}

#[gpui::test]
fn unicode_multiline_prompt_clears_only_after_accepted_receipt(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    cx.simulate_input("Hello 🌍\n世界");
    let before = app.read_with(cx, |app, cx| app.composer.read(cx).value().to_string());
    assert_eq!(before, "Hello 🌍\n世界");
    let bounds = cx.debug_bounds("send").expect("visible Send button");
    cx.simulate_click(bounds.center(), Default::default());
    let connection::Command::Call {
        tag,
        payload,
        reply,
    } = requests.try_recv().unwrap()
    else {
        panic!("expected turn start");
    };
    assert_eq!(tag, "orchestration.dispatchCommand");
    assert_eq!(payload["message"]["text"], before);
    assert_eq!(
        app.read_with(cx, |app, cx| app.composer.read(cx).value().to_string()),
        before
    );
    reply.send_blocking(Ok(json!({"sequence":1}))).unwrap();
    cx.run_until_parked();
    assert_eq!(
        app.read_with(cx, |app, cx| app.composer.read(cx).value().to_string()),
        ""
    );
}

#[gpui::test]
fn rejected_send_preserves_user_prompt(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    cx.simulate_input("Do not lose this prompt");
    let bounds = cx.debug_bounds("send").unwrap();
    cx.simulate_click(bounds.center(), Default::default());
    let connection::Command::Call { reply, .. } = requests.try_recv().unwrap() else {
        panic!("expected call");
    };
    reply
        .send_blocking(Err("Provider rejected request".into()))
        .unwrap();
    cx.run_until_parked();
    assert_eq!(
        app.read_with(cx, |app, cx| app.composer.read(cx).value().to_string()),
        "Do not lose this prompt"
    );
    assert_eq!(app.read_with(cx, |app, _| app.pending_actions), 0);
    assert_eq!(
        app.read_with(cx, |app, _| app.status.clone()),
        "Provider rejected request"
    );
}

#[gpui::test]
fn editing_while_send_is_pending_does_not_get_erased(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    cx.simulate_input("First prompt");
    let bounds = cx.debug_bounds("send").unwrap();
    cx.simulate_click(bounds.center(), Default::default());
    let connection::Command::Call { reply, .. } = requests.try_recv().unwrap() else {
        panic!("expected call");
    };
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.composer.update(cx, |input, cx| input.focus(window, cx))
        })
    });
    cx.simulate_input(" plus a later edit");
    reply.send_blocking(Ok(json!({"sequence":1}))).unwrap();
    cx.run_until_parked();
    assert_eq!(
        app.read_with(cx, |app, cx| app.composer.read(cx).value().to_string()),
        "First prompt plus a later edit"
    );
}

#[gpui::test]
fn delete_requires_confirmation_before_any_request(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    let bounds = cx.debug_bounds("delete").unwrap();
    cx.simulate_click(bounds.center(), Default::default());
    assert!(requests.try_recv().is_err());
    assert_eq!(
        app.read_with(cx, |app, _| app.confirm_delete.clone()),
        Some("t".into())
    );
    cx.simulate_click(bounds.center(), Default::default());
    let connection::Command::Call { payload, reply, .. } = requests.try_recv().unwrap() else {
        panic!("expected delete request");
    };
    assert_eq!(payload["type"], "thread.delete");
    reply
        .send_blocking(Err("Fixture refuses deletion".into()))
        .unwrap();
    cx.run_until_parked();
}

#[gpui::test]
fn clipboard_roundtrip_preserves_multiline_unicode(cx: &mut TestAppContext) {
    let (app, cx, _requests) = harness(cx);
    cx.write_to_clipboard(gpui::ClipboardItem::new_string("Hello 🌍\n世界".into()));
    cx.simulate_keystrokes("cmd-v");
    assert_eq!(
        app.read_with(cx, |app, cx| app.composer.read(cx).value().to_string()),
        "Hello 🌍\n世界"
    );
    cx.simulate_keystrokes("cmd-a cmd-c");
    assert_eq!(
        cx.read_from_clipboard().unwrap().text().unwrap(),
        "Hello 🌍\n世界"
    );
    cx.simulate_keystrokes("backspace");
    assert_eq!(
        app.read_with(cx, |app, cx| app.composer.read(cx).value().to_string()),
        ""
    );
}

#[gpui::test]
fn stashing_and_restoring_preserves_prompt_without_overwriting_edits(cx: &mut TestAppContext) {
    let (app, cx, _requests) = harness(cx);
    cx.simulate_input("Saved prompt");
    let stash = cx.debug_bounds("stash").unwrap();
    cx.simulate_click(stash.center(), Default::default());
    assert_eq!(
        app.read_with(cx, |app, cx| app.composer.read(cx).value().to_string()),
        ""
    );
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.composer.update(cx, |input, cx| input.focus(window, cx))
        })
    });
    cx.simulate_input("Keep this edit");
    let restore = cx.debug_bounds("restore-stash").unwrap();
    cx.simulate_click(restore.center(), Default::default());
    assert_eq!(
        app.read_with(cx, |app, cx| app.composer.read(cx).value().to_string()),
        "Keep this edit"
    );
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.composer
                .update(cx, |input, cx| input.set_value("", window, cx))
        })
    });
    cx.simulate_click(restore.center(), Default::default());
    assert_eq!(
        app.read_with(cx, |app, cx| app.composer.read(cx).value().to_string()),
        "Saved prompt"
    );
}

#[gpui::test]
fn thread_selection_keeps_independent_drafts(cx: &mut TestAppContext) {
    let (app, cx, _requests) = harness(cx);
    cx.simulate_input("Draft for first thread");
    cx.update(|window, cx| {
        app.update(cx, |app, cx| app.select_thread("second-thread", window, cx))
    });
    assert_eq!(
        app.read_with(cx, |app, cx| app.composer.read(cx).value().to_string()),
        ""
    );
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.composer.update(cx, |input, cx| input.focus(window, cx))
        })
    });
    cx.simulate_input("Draft for second thread");
    cx.update(|window, cx| app.update(cx, |app, cx| app.select_thread("t", window, cx)));
    assert_eq!(
        app.read_with(cx, |app, cx| app.composer.read(cx).value().to_string()),
        "Draft for first thread"
    );
    cx.update(|window, cx| {
        app.update(cx, |app, cx| app.select_thread("second-thread", window, cx))
    });
    assert_eq!(
        app.read_with(cx, |app, cx| app.composer.read(cx).value().to_string()),
        "Draft for second thread"
    );
}
