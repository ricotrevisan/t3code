use super::*;
use serde_json::json;
#[test]
fn fixa_roster_is_keyed_and_revision_cached_not_elapsed_refolded() {
    let rows = (0..2000)
        .map(|i| {
            activity(
                "task.started",
                json!({"taskId":format!("agent-{i}"), "agentKind":"agent"}),
                i,
            )
        })
        .collect();
    let thread = thread(rows);
    let mut cache = RosterCache::default();
    let first = cache.get(1, 10, &thread);
    assert_eq!(first.len(), 100);
    assert!(cache.live());
    let again = cache.get(1, 10, &thread);
    assert!(std::rc::Rc::ptr_eq(&first, &again));
    let next = cache.get(1, 11, &thread);
    assert!(!std::rc::Rc::ptr_eq(&first, &next));
    let reconnect = cache.get(2, 11, &thread);
    assert!(!std::rc::Rc::ptr_eq(&next, &reconnect));
}

fn activity(kind: &str, p: Value, second: u32) -> Value {
    json!({"id":format!("row-{second}"),"kind":kind,"createdAt":format!("2026-01-01T00:00:{second:02}Z"),"payload":p})
}
fn thread(rows: Vec<Value>) -> Value {
    json!({"id":"t","activities":rows,"session":{"status":"running"}})
}
#[test]
fn only_server_stamped_agents_join_and_membership_is_sticky() {
    let a = fold(&thread(vec![
        activity(
            "task.started",
            json!({"taskId":"shell","taskType":"subagent","title":"Run shell"}),
            0,
        ),
        activity(
            "task.started",
            json!({"taskId":"agent","agentKind":"agent","title":"Research"}),
            1,
        ),
        activity(
            "task.completed",
            json!({"taskId":"agent","summary":"Found it"}),
            2,
        ),
    ]));
    assert_eq!(a.len(), 1);
    assert_eq!(a[0].id, "agent");
    assert_eq!(a[0].status, Status::Completed);
    assert_eq!(a[0].activity(), Some("Found it"));
}
#[test]
fn completion_before_late_start_stays_terminal_and_enriches() {
    let a = fold(&thread(vec![
        activity(
            "task.updated",
            json!({"taskId":"a","agentKind":"agent","status":"failed","endedAt":"2026-01-01T00:00:00Z"}),
            1,
        ),
        activity(
            "task.started",
            json!({"taskId":"a","title":"Late title"}),
            2,
        ),
        activity(
            "task.completed",
            json!({"taskId":"a","summary":"Problem","typedUsage":{"totalTokens":400}}),
            3,
        ),
        activity(
            "task.completed",
            json!({"taskId":"a","summary":"Duplicate"}),
            4,
        ),
    ]));
    assert_eq!(a[0].status, Status::Failed);
    assert_eq!(a[0].title, "Late title");
    assert_eq!(a[0].error.as_deref(), Some("Problem"));
    assert_eq!(a[0].completed.as_deref(), Some("2026-01-01T00:00:00Z"));
    assert_eq!(a[0].usage["totalTokens"], 400.);
}
#[test]
fn explicit_reactivation_clears_previous_result_and_counts_one_run() {
    let a = fold(&thread(vec![
        activity("task.started", json!({"taskId":"a","agentKind":"agent"}), 0),
        activity("task.completed", json!({"taskId":"a","summary":"old"}), 1),
        activity(
            "task.updated",
            json!({"taskId":"a","status":"running","attempt":2}),
            2,
        ),
        activity("task.progress", json!({"taskId":"a","summary":"new"}), 3),
    ]));
    assert_eq!(a[0].status, Status::Running);
    assert_eq!(a[0].activations, 2);
    assert!(a[0].result.is_none());
    assert!(a[0].completed.is_none());
    assert_eq!(a[0].started.as_deref(), Some("2026-01-01T00:00:02Z"));
}
#[test]
fn idle_usage_snapshots_do_not_reactivate_and_max_merge_keeps_breakdown() {
    let a = fold(&thread(vec![
        activity("task.started", json!({"taskId":"a","agentKind":"agent"}), 0),
        activity(
            "task.progress",
            json!({"taskId":"a","typedUsage":{"totalTokens":100,"inputTokens":70,"outputTokens":30}}),
            1,
        ),
        activity("task.updated", json!({"taskId":"a","status":"idle"}), 2),
        activity(
            "task.progress",
            json!({"taskId":"a","usageSnapshot":true,"typedUsage":{"totalTokens":80}}),
            3,
        ),
    ]));
    assert_eq!(a[0].status, Status::Idle);
    assert_eq!(a[0].usage["totalTokens"], 100.);
    assert_eq!(a[0].usage["inputTokens"], 70.);
    assert_eq!(a[0].activations, 1);
}
#[test]
fn coordinator_completion_cascades_and_totals_do_not_double_count() {
    let a = fold(&thread(vec![
        activity(
            "task.started",
            json!({"taskId":"w","agentKind":"agent","taskType":"local_workflow","phases":[{"index":0,"title":"Build"}],"runHandles":{"scriptPath":"/tmp/workflow.js"}}),
            0,
        ),
        activity(
            "task.started",
            json!({"taskId":"m","agentKind":"agent","parentAgentId":"w","phaseIndex":0}),
            1,
        ),
        activity(
            "task.progress",
            json!({"taskId":"m","typedUsage":{"totalTokens":50}}),
            2,
        ),
        activity(
            "task.completed",
            json!({"taskId":"w","typedUsage":{"totalTokens":50}}),
            3,
        ),
    ]));
    assert_eq!(a[0].kind, "workflow");
    assert_eq!(a[0].phases, vec![(0, "Build".into())]);
    assert_eq!(a[0].script.as_deref(), Some("/tmp/workflow.js"));
    assert_eq!(a[1].status, Status::Completed);
    assert_eq!(totals(&a), (0, 0, 1, 50.));
}
#[test]
fn stopped_session_interrupts_active_but_preserves_idle() {
    let mut t = thread(vec![
        activity("task.started", json!({"taskId":"a","agentKind":"agent"}), 0),
        activity("task.started", json!({"taskId":"b","agentKind":"agent"}), 1),
        activity("task.updated", json!({"taskId":"b","status":"idle"}), 2),
    ]);
    t["session"] = json!({"status":"stopped"});
    let a = fold(&t);
    assert_eq!(a[0].status, Status::Interrupted);
    assert_eq!(a[1].status, Status::Idle);
}
#[test]
fn elapsed_freezes_at_completion_and_live_tool_progress_has_timeline_identity() {
    let a = fold(&thread(vec![
        activity("task.started", json!({"taskId":"a","agentKind":"agent"}), 0),
        activity("tool.progress", json!({"taskId":"a","toolName":"Read"}), 8),
    ]));
    assert_eq!(a[0].activity(), Some("▸ Read"));
    assert_eq!(a[0].activity_id, "row-8");
    assert_eq!(
        a[0].elapsed("2026-01-01T00:01:01Z").as_deref(),
        Some("1m 01s")
    );
    let a = fold(&thread(vec![
        activity("task.started", json!({"taskId":"a","agentKind":"agent"}), 0),
        activity("task.completed", json!({"taskId":"a"}), 8),
    ]));
    assert_eq!(a[0].elapsed("2026-01-01T01:00:00Z").as_deref(), Some("8s"));
}
#[test]
fn authoritative_empty_background_signal_interrupts_but_legacy_absence_does_not() {
    let mut t = thread(vec![activity(
        "task.started",
        json!({"taskId":"a","agentKind":"agent"}),
        0,
    )]);
    t["session"]["status"] = json!("ready");
    assert_eq!(fold(&t)[0].status, Status::Running);
    t["backgroundLiveness"] = Value::Null;
    assert_eq!(fold(&t)[0].status, Status::Interrupted);
    t["backgroundLiveness"] = json!("monitoring");
    assert_eq!(fold(&t)[0].status, Status::Running);
    t["session"] = Value::Null;
    assert_eq!(fold(&t)[0].status, Status::Interrupted);
}
#[test]
fn retention_prefers_live_without_reordering_survivors() {
    let mut rows = vec![activity(
        "task.started",
        json!({"taskId":"live","agentKind":"agent"}),
        0,
    )];
    for i in 0..130 {
        rows.push(activity(
            "task.completed",
            json!({"taskId":format!("done-{i:03}"),"agentKind":"agent"}),
            i % 60,
        ));
    }
    let a = fold(&thread(rows));
    assert_eq!(a.len(), 100);
    assert!(a.iter().any(|a| a.id == "live"));
    assert!(
        a.windows(2)
            .all(|w| (&w[0].first_seen, &w[0].id) <= (&w[1].first_seen, &w[1].id))
    );
}
