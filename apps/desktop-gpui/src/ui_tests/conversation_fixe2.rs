use super::*;

#[gpui_kit::test]
fn conversation_fixe2_tail_append_preserves_same_role_boundary(cx: &mut TestAppContext) {
    let (app, cx, _) = harness(cx);
    cx.update(|_, cx| app.update(cx, |app, cx| {
        let mut item = snapshot(vec![
            json!({"id":"u0", "turnId":"old", "role":"user", "text":"old", "createdAt":"2026-04-01T00:00:00Z"}),
            json!({"id":"u1", "turnId":"tail", "role":"user", "text":"latest", "createdAt":"2026-04-01T00:01:00Z"}),
        ], "running");
        for key in ["activities", "checkpoints", "proposedPlans"] {
            item["snapshot"]["thread"][key] = json!([]);
        }
        item["snapshot"]["thread"]["latestTurn"] = json!({"turnId":"tail", "state":"running", "startedAt":"2026-04-01T00:01:00Z"});
        app.apply_thread_update("t", item, cx).unwrap();
        cx.notify();
    }));
    cx.run_until_parked();
    let before = app.read_with(cx, |app, _| app.conversation_presentation.prepared_turns);
    cx.update(|_, cx| app.update(cx, |app, cx| {
        let sequence = app.detail.sequence.unwrap() + 1;
        app.apply_thread_update("t", json!({"kind":"event", "event":{"sequence":sequence, "type":"thread.message-sent", "occurredAt":"2026-04-01T00:01:02Z", "payload":{"threadId":"t", "messageId":"a1", "turnId":"tail", "role":"assistant", "text":"append", "streaming":true, "createdAt":"2026-04-01T00:01:02Z", "updatedAt":"2026-04-01T00:01:02Z"}}}), cx).unwrap();
        cx.notify();
    }));
    cx.run_until_parked();
    app.read_with(cx, |app, _| {
        assert_eq!(app.conversation_presentation.prepared_turns, before + 1);
        assert_eq!(
            *app.conversation_rows,
            t3code_desktop_gpui::conversation::timeline_rows(
                app.detail.thread.as_ref().unwrap(),
                &app.activity_expanded
            )
        );
    });
}

#[gpui_kit::test]
fn conversation_fixe2_folded_tail_append_recomputes_history_disclosures(cx: &mut TestAppContext) {
    let (app, cx, _) = harness(cx);
    cx.update(|_, cx| app.update(cx, |app, cx| {
        let mut item = snapshot(vec![
            json!({"id":"u0", "turnId":"old", "role":"user", "text":"old", "createdAt":"2026-04-01T00:00:00Z"}),
            json!({"id":"u1", "turnId":"tail", "role":"user", "text":"latest", "createdAt":"2026-04-01T00:01:00Z"}),
        ], "running");
        item["snapshot"]["thread"]["checkpoints"] = json!([]);
        item["snapshot"]["thread"]["proposedPlans"] = json!([]);
        item["snapshot"]["thread"]["activities"] = json!([{"id":"old-work", "turnId":"old", "kind":"log", "summary":"previous work", "createdAt":"2026-04-01T00:00:01Z"}]);
        item["snapshot"]["thread"]["latestTurn"] = json!({"turnId":"tail", "state":"running", "startedAt":"2026-04-01T00:01:00Z"});
        app.activity_expanded.insert("fold:tail".into(), true);
        app.apply_thread_update("t", item, cx).unwrap();
        cx.notify();
    }));
    cx.run_until_parked();
    let before = app.read_with(cx, |app, _| app.conversation_presentation.prepared_turns);
    cx.update(|_, cx| app.update(cx, |app, cx| {
        let sequence = app.detail.sequence.unwrap() + 1;
        app.apply_thread_update("t", json!({"kind":"event", "event":{"sequence":sequence, "type":"thread.activity-appended", "occurredAt":"2026-04-01T00:01:02Z", "payload":{"threadId":"t", "activity":{"id":"tail-work", "turnId":"tail", "kind":"log", "summary":"new work", "createdAt":"2026-04-01T00:01:02Z"}}}}), cx).unwrap();
        cx.notify();
    }));
    cx.run_until_parked();
    app.read_with(cx, |app, _| {
        assert_eq!(app.conversation_presentation.prepared_turns, before + 2);
        assert_eq!(
            *app.conversation_rows,
            t3code_desktop_gpui::conversation::timeline_rows(
                app.detail.thread.as_ref().unwrap(),
                &app.activity_expanded
            )
        );
    });
}

#[gpui_kit::test]
fn conversation_fixe2_tail_checkpoint_removal_restores_previous_controls(cx: &mut TestAppContext) {
    let (app, cx, _) = harness(cx);
    cx.update(|_, cx| app.update(cx, |app, cx| {
        let mut item = snapshot(vec![
            json!({"id":"u0", "turnId":"old", "role":"user", "text":"old", "createdAt":"2026-04-01T00:00:00Z"}),
            json!({"id":"u1", "turnId":"tail", "role":"user", "text":"latest", "createdAt":"2026-04-01T00:01:00Z"}),
        ], "running");
        item["snapshot"]["thread"]["latestTurn"] = json!({"turnId":"tail", "state":"running", "startedAt":"2026-04-01T00:01:00Z"});
        item["snapshot"]["thread"]["checkpoints"] = json!([
            {"turnId":"old", "checkpointTurnCount":1, "completedAt":"2026-04-01T00:00:01Z", "files":[{"path":"old.rs", "additions":1, "deletions":0}]},
            {"turnId":"tail", "checkpointTurnCount":2, "completedAt":"2026-04-01T00:01:01Z", "files":[{"path":"tail.rs", "additions":1, "deletions":0}]},
        ]);
        app.apply_thread_update("t", item, cx).unwrap();
        cx.notify();
    }));
    cx.run_until_parked();
    let before = app.read_with(cx, |app, _| app.conversation_presentation.prepared_turns);
    cx.update(|_, cx| app.update(cx, |app, cx| {
        std::sync::Arc::make_mut(app.detail.thread.as_mut().unwrap())["checkpoints"].as_array_mut().unwrap().pop();
        let sequence = app.detail.sequence.unwrap() + 1;
        app.apply_thread_update("t", json!({"kind":"event", "event":{"sequence":sequence, "type":"thread.message-sent", "occurredAt":"2026-04-01T00:01:02Z", "payload":{"threadId":"t", "messageId":"a1", "turnId":"tail", "role":"assistant", "text":"append", "streaming":true, "createdAt":"2026-04-01T00:01:02Z", "updatedAt":"2026-04-01T00:01:02Z"}}}), cx).unwrap();
        cx.notify();
    }));
    cx.run_until_parked();
    app.read_with(cx, |app, _| {
        assert_eq!(app.conversation_presentation.prepared_turns, before + 2);
        assert_eq!(*app.conversation_rows, t3code_desktop_gpui::conversation::timeline_rows(app.detail.thread.as_ref().unwrap(), &app.activity_expanded));
        assert!(app.conversation_rows.iter().any(|row| matches!(&row.entry,
            t3code_desktop_gpui::conversation::TimelineEntry::Checkpoint { turn, latest: true, .. } if turn == "old"
        )));
    });
}

#[gpui_kit::test]
fn conversation_fixe2_direct_projection_changes_replace_cached_rows(cx: &mut TestAppContext) {
    let (app, cx, _) = harness(cx);
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.apply_thread_update("t", snapshot(vec![], "idle"), cx)
                .unwrap();
            cx.notify();
        })
    });
    cx.run_until_parked();
    let replacements = [
        (
            "messages",
            json!([{"id":"m", "turnId":"turn", "role":"user", "text":"updated", "createdAt":"2026-04-01T00:00:00Z"}]),
        ),
        (
            "activities",
            json!([{"id":"a", "turnId":"turn", "kind":"approval.requested", "summary":"Approve", "createdAt":"2026-04-01T00:00:01Z", "payload":{"requestId":"req", "options":[{"decision":"accept", "label":"Allow"}]}}]),
        ),
        (
            "checkpoints",
            json!([{"turnId":"turn", "checkpointTurnCount":1, "completedAt":"2026-04-01T00:00:02Z", "files":[{"path":"src/main.rs", "additions":1, "deletions":0}]}]),
        ),
        (
            "proposedPlans",
            json!([{"id":"p", "turnId":"turn", "createdAt":"2026-04-01T00:00:03Z", "planMarkdown":"# Plan"}]),
        ),
        (
            "latestTurn",
            json!({"turnId":"turn", "state":"error", "startedAt":"2026-04-01T00:00:00Z"}),
        ),
        ("session", json!({"lastError":"provider failed"})),
        ("activities", json!([])),
    ];
    for (field, value) in replacements {
        cx.update(|_, cx| {
            app.update(cx, |app, cx| {
                // Deliberately retain the revision: public projection data can be
                // replaced/mutated without going through the stream event helper.
                std::sync::Arc::make_mut(app.detail.thread.as_mut().unwrap())[field] = value;
                cx.notify();
            })
        });
        cx.run_until_parked();
        app.read_with(cx, |app, _| {
            assert_eq!(
                *app.conversation_rows,
                t3code_desktop_gpui::conversation::timeline_rows(
                    app.detail.thread.as_ref().unwrap(),
                    &app.activity_expanded
                ),
                "stale rows after {field} replacement"
            )
        });
        let before = app.read_with(cx, |app, _| app.conversation_presentation.preparations);
        cx.update(|_, cx| app.update(cx, |_, cx| cx.notify()));
        cx.run_until_parked();
        assert_eq!(
            app.read_with(cx, |app, _| app.conversation_presentation.preparations),
            before,
            "unchanged notification must reuse the rows"
        );
    }
}

#[gpui_kit::test]
fn conversation_fixe2_untracked_history_change_disqualifies_tail_hint(cx: &mut TestAppContext) {
    let (app, cx, _) = harness(cx);
    cx.update(|_, cx| app.update(cx, |app, cx| {
        let mut item = snapshot(vec![
            json!({"id":"u0", "turnId":"old", "role":"user", "text":"before", "createdAt":"2026-04-01T00:00:00Z"}),
            json!({"id":"u1", "turnId":"tail", "role":"user", "text":"latest", "createdAt":"2026-04-01T00:01:00Z"}),
        ], "running");
        item["snapshot"]["thread"]["checkpoints"] = json!([]);
        item["snapshot"]["thread"]["latestTurn"] = json!({"turnId":"tail", "state":"running", "startedAt":"2026-04-01T00:01:00Z"});
        app.apply_thread_update("t", item, cx).unwrap();
        cx.notify();
    }));
    cx.run_until_parked();
    let before = app.read_with(cx, |app, _| app.conversation_presentation.prepared_turns);
    cx.update(|_, cx| app.update(cx, |app, cx| {
        std::sync::Arc::make_mut(app.detail.thread.as_mut().unwrap())["messages"][0]["text"] = json!("historical replacement");
        let sequence = app.detail.sequence.unwrap() + 1;
        app.apply_thread_update("t", json!({"kind":"event", "event":{"sequence":sequence, "type":"thread.message-sent", "occurredAt":"2026-04-01T00:01:01Z", "payload":{"threadId":"t", "messageId":"a1", "turnId":"tail", "role":"assistant", "text":"appended", "streaming":true, "createdAt":"2026-04-01T00:01:01Z", "updatedAt":"2026-04-01T00:01:01Z"}}}), cx).unwrap();
        cx.notify();
    }));
    cx.run_until_parked();
    app.read_with(cx, |app, _| {
        assert_eq!(
            app.conversation_presentation.prepared_turns,
            before + 2,
            "must rebuild history, not trust the tail hint"
        );
        assert_eq!(
            *app.conversation_rows,
            t3code_desktop_gpui::conversation::timeline_rows(
                app.detail.thread.as_ref().unwrap(),
                &app.activity_expanded
            )
        );
    });
}
