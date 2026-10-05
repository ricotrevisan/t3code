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
fn unsupported_event_is_not_silently_presented_as_current_history() {
    let mut detail = ThreadDetail::default();
    detail.apply("t", snapshot(10)).unwrap();
    assert!(detail.apply("t",json!({"kind":"event","event":{"sequence":11,"type":"thread.future-feature","payload":{"threadId":"t"},"occurredAt":"2026-04-01T00:00:00.000Z"}})).is_err());
    assert_eq!(detail.sequence, Some(10));
}
