use super::*;
use gpui_kit::Modifiers;

#[gpui_kit::test]
fn conversation_deferred_message_jump_is_retired_on_generation_change(cx: &mut TestAppContext) {
    let (app, cx, _) = harness(cx);
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.apply_thread_update("t", snapshot((0..60).map(message).collect(), "idle"), cx)
                .unwrap();
            app.scroll_to_message("not-loaded-yet");
            cx.notify();
        })
    });
    cx.run_until_parked();
    assert!(app.read_with(cx, |app, _| app.conversation_presentation.jump.is_some()));
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.generation += 1;
            cx.notify();
        })
    });
    cx.run_until_parked();
    assert!(app.read_with(cx, |app, _| app.conversation_presentation.jump.is_none()));
    assert!(app.read_with(cx, |app, _| app.conversation_scroll.is_following_tail()));
}

#[gpui_kit::test]
fn conversation_wheel_reveals_return_button_without_a_projection_change(cx: &mut TestAppContext) {
    let (app, cx, _) = harness(cx);
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.apply_thread_update("t", snapshot((0..60).map(message).collect(), "idle"), cx)
                .unwrap();
            cx.notify();
        })
    });
    cx.run_until_parked();
    let before = app.read_with(cx, |app, _| app.conversation_presentation.preparations);
    let bounds = cx.debug_bounds("message-m59").unwrap();
    cx.simulate_event(gpui_kit::ScrollWheelEvent {
        position: bounds.center(),
        delta: gpui_kit::ScrollDelta::Pixels(point(px(0.), px(500.))),
        modifiers: Default::default(),
        touch_phase: gpui_kit::TouchPhase::Moved,
    });
    cx.run_until_parked();
    assert!(cx.debug_bounds("return-to-latest").is_some());
    assert_eq!(
        app.read_with(cx, |app, _| app.conversation_presentation.preparations),
        before
    );
}

#[gpui_kit::test]
fn conversation_fold_restores_work_and_search_unfolds_the_target(cx: &mut TestAppContext) {
    let (app, cx, _) = harness(cx);
    let mut messages: Vec<_> = (0..40).map(message).collect();
    for message in &mut messages {
        message["turnId"] = json!("turn");
    }
    messages[0]["role"] = json!("user");
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.apply_thread_update("t", snapshot(messages, "idle"), cx)
                .unwrap();
            app.scroll_to_message("m0");
            cx.notify();
        })
    });
    cx.run_until_parked();
    let bounds = cx.debug_bounds("fold-turn-turn").unwrap();
    cx.simulate_click(bounds.center(), Modifiers::none());
    cx.run_until_parked();
    assert_eq!(app.read_with(cx, |app, _| app.conversation_rows.len()), 1);
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.sidebar_state.message_anchor = Some(sidebar::MessageAnchor {
                environment: app.environment_scope.clone(),
                thread_id: "t".into(),
                source: "assistant".into(),
                message_created_at: "2026-10-06T10:00:00Z".into(),
            });
            cx.notify();
        })
    });
    cx.run_until_parked();
    assert_eq!(app.read_with(cx, |app, _| app.conversation_rows.len()), 40);
    assert!(app.read_with(cx, |app, _| app.sidebar_state.message_anchor.is_none()));
    assert!(cx.debug_bounds("message-m1").is_some());
}

#[gpui_kit::test]
fn conversation_copy_writes_the_message_and_records_visible_feedback(cx: &mut TestAppContext) {
    let (app, cx, _) = harness(cx);
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.apply_thread_update(
                "t",
                snapshot(
                    vec![json!({"id":"a","role":"assistant","text":"Copy the whole message"})],
                    "idle",
                ),
                cx,
            )
            .unwrap();
            cx.notify();
        })
    });
    cx.run_until_parked();
    let message = cx.debug_bounds("message-a").unwrap();
    cx.simulate_mouse_move(message.center(), None, Modifiers::none());
    cx.run_until_parked();
    let copy = cx.debug_bounds("copy-a").unwrap();
    cx.simulate_click(copy.center(), Modifiers::none());
    cx.run_until_parked();
    assert_eq!(
        cx.read_from_clipboard().unwrap().text().unwrap(),
        "Copy the whole message"
    );
    assert_eq!(
        app.read_with(cx, |app, _| app.conversation_presentation.copied.clone()),
        Some("copy-a".into())
    );
}
