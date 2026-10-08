use super::*;
fn initial(duplicate: bool) -> Value {
    let mut messages = (0..10000)
        .map(
            |i| json!({"id":format!("old-{i}"),"role":"assistant","turnId":"old","text":"history"}),
        )
        .collect::<Vec<_>>();
    if duplicate {
        messages[0]["id"] = json!("answer");
    }
    messages.push(json!({"id":"answer","role":"assistant","turnId":"tail","text":"first","createdAt":"2026-10-01T00:00:00Z"}));
    json!({"kind":"snapshot","snapshot":{"snapshotSequence":0,"thread":{"id":"t","messages":messages,"activities":[],"checkpoints":[],"proposedPlans":[],"latestTurn":{"turnId":"tail","state":"running","startedAt":"2026-10-01T00:00:00Z"},"session":{"status":"running","activeTurnId":"tail"}}}})
}
fn event(sequence: u64) -> Value {
    json!({"kind":"event","event":{"sequence":sequence,"type":"thread.message-sent","occurredAt":"2026-10-01T00:00:00Z","payload":{"threadId":"t","messageId":"answer","role":"assistant","turnId":"tail","text":" chunk","streaming":true,"createdAt":"2026-10-01T00:00:00Z"}}})
}
#[test]
fn r13_d2_sealed_tail_evidence_chains_but_external_mutation_and_duplicates_fail_closed() {
    let mut detail = ThreadDetail::default();
    detail.apply("t", initial(false)).unwrap();
    let base = detail.thread.clone().unwrap();
    detail.apply("t", event(1)).unwrap();
    detail.apply("t", event(2)).unwrap();
    assert!(detail.verified_message_tail(&base, "tail"));
    let disclosures = std::collections::HashMap::new();
    let old_rows = crate::conversation::timeline_rows(&base, &disclosures);
    if let Some((start, mut template)) =
        crate::conversation::message_tail_template(&base, &old_rows)
    {
        let rows = crate::conversation::timeline_verified_message_rows(
            detail.thread.as_ref().unwrap(),
            &mut template,
            start,
            &disclosures,
            &old_rows,
        )
        .unwrap();
        assert_eq!(
            rows,
            crate::conversation::timeline_rows(detail.thread.as_ref().unwrap(), &disclosures)
        );
        assert!(template["messages"].as_array().unwrap().len() < 10);
    } else {
        panic!("latest-turn cache");
    }
    assert!(!detail.verified_message_tail(&base, "other"));
    std::sync::Arc::make_mut(detail.thread.as_mut().unwrap())["messages"][0]["text"] =
        json!("changed history");
    assert!(!detail.verified_message_tail(&base, "tail"));
    detail.apply("t", event(3)).unwrap();
    assert!(!detail.verified_message_tail(&base, "tail"));
    detail.apply("t", initial(true)).unwrap();
    let duplicate = detail.thread.clone().unwrap();
    detail.apply("t", event(4)).unwrap();
    assert!(!detail.verified_message_tail(&duplicate, "tail"));
}
