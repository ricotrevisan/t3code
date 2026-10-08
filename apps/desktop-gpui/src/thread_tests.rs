use super::*;

fn snapshot(sequence: u64) -> Value {
    json!({"kind":"snapshot","snapshot":{"snapshotSequence":sequence,"thread":{"id":"t","title":"Thread","messages":[],"checkpoints":[],"activities":[],"proposedPlans":[],"session":null,"latestTurn":null}}})
}
fn delta(sequence: u64, thread_id: &str, text: &str) -> Value {
    json!({"kind":"event","event":{"sequence":sequence,"type":"thread.message-sent","occurredAt":"2026-04-01T00:00:00.000Z","payload":{"threadId":thread_id,"messageId":"m","role":"assistant","text":text,"turnId":"turn-1","streaming":true,"createdAt":"2026-04-01T00:00:00.000Z","updatedAt":"2026-04-01T00:00:00.000Z"}}})
}
#[test]
fn replay_deduplicates_deltas_and_different_thread_cannot_corrupt_selection() {
    let mut detail = ThreadDetail::default();
    detail.apply("t", snapshot(10)).unwrap();
    detail.apply("t", delta(11, "t", "Hello")).unwrap();
    detail.apply("t", delta(11, "t", "Hello")).unwrap();
    detail.apply("t", delta(15, "t", ", world")).unwrap();
    assert_eq!(
        detail.thread.as_ref().unwrap()["messages"][0]["text"],
        "Hello, world"
    );
    assert!(detail.apply("t", delta(16, "wrong", "Corrupt")).is_err());
    assert_eq!(detail.sequence, Some(15));
    assert_eq!(
        detail.thread.as_ref().unwrap()["messages"][0]["text"],
        "Hello, world"
    );
}
#[test]
fn tail_preparation_owner_accumulates_appends_and_invalidates_historical_changes() {
    let mut detail = ThreadDetail::default();
    let mut initial = snapshot(10);
    initial["snapshot"]["thread"]["latestTurn"] =
        json!({"turnId":"turn-1", "state":"running", "startedAt":"2026-04-01T00:00:00.000Z"});
    detail.apply("t", initial).unwrap();
    let base = detail.revision;
    detail.apply("t", delta(11, "t", "first")).unwrap();
    detail.apply("t", delta(12, "t", " second")).unwrap();
    assert_eq!(detail.tail_change, Some((base, "turn-1".into())));
    let mut historical = delta(13, "t", "old");
    historical["event"]["payload"]["turnId"] = json!("old-turn");
    detail.apply("t", historical).unwrap();
    assert!(detail.tail_change.is_none());
    detail.apply("t", snapshot(14)).unwrap();
    assert!(detail.tail_change.is_none());
}

#[test]
fn synchronization_requires_snapshot_and_reconnect_replaces_old_state() {
    let mut detail = ThreadDetail::default();
    assert!(detail.apply("t", json!({"kind":"synchronized"})).is_err());
    assert!(detail.apply("t", delta(1, "t", "before snapshot")).is_err());
    assert!(detail.apply("wrong", snapshot(10)).is_err());
    detail.apply("t", snapshot(10)).unwrap();
    detail.apply("t", delta(11, "t", "Hello")).unwrap();
    detail.apply("t", json!({"kind":"synchronized"})).unwrap();
    assert!(detail.synchronized);
    detail.apply("t", snapshot(20)).unwrap();
    assert!(!detail.synchronized);
    assert!(
        detail.thread.as_ref().unwrap()["messages"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}
#[test]
fn projection_revision_changes_for_same_sequence_snapshots_but_not_duplicate_events() {
    let mut detail = ThreadDetail::default();
    detail.apply("t", snapshot(10)).unwrap();
    let first = detail.revision;
    detail.apply("t", snapshot(10)).unwrap();
    assert!(detail.revision > first);
    detail.apply("t", delta(11, "t", "Hello")).unwrap();
    let before_duplicate = detail.revision;
    detail.apply("t", delta(11, "t", "Hello")).unwrap();
    detail.apply("t", json!({"kind":"synchronized"})).unwrap();
    assert_eq!(detail.revision, before_duplicate);
}

#[test]
fn unsupported_event_is_not_silently_presented_as_current_history() {
    let mut detail = ThreadDetail::default();
    detail.apply("t", snapshot(10)).unwrap();
    assert!(detail.apply("t",json!({"kind":"event","event":{"sequence":11,"type":"thread.future-feature","payload":{"threadId":"t"},"occurredAt":"2026-04-01T00:00:00.000Z"}})).is_err());
    assert_eq!(detail.sequence, Some(10));
}
