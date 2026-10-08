use super::*;

fn begin(
    app: &Entity<NativeApp>,
    cx: &mut VisualTestContext,
    requests: &Requests,
) -> (String, async_channel::Receiver<Result<(), String>>) {
    let result = cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.shell.synchronized = true;
            app.test_checkout_stop(cx)
        })
    });
    cx.run_until_parked();
    let connection::Command::Subscribe { key, tag, payload } = requests.try_recv().unwrap() else {
        panic!("subscribe before stop");
    };
    assert!(key.starts_with("checkout-stop-"));
    assert_eq!(tag, "orchestration.subscribeThread");
    assert_eq!(
        payload,
        json!({"threadId":"t","reasoningMessages":false,"requestCompletionMarker":true})
    );
    assert!(requests.try_recv().is_err());
    (key, result)
}

fn item(
    app: &Entity<NativeApp>,
    cx: &mut VisualTestContext,
    key: &str,
    value: Result<Value, String>,
) {
    cx.update(|_, cx| {
        app.update(cx, |app, _| {
            app.apply_checkout_stop_subscription(key, value)
        })
    });
    cx.run_until_parked();
}

fn baseline(app: &Entity<NativeApp>, cx: &mut VisualTestContext, key: &str) {
    item(
        app,
        cx,
        key,
        Ok(
            json!({"kind":"snapshot","supportsCorrelatedSessionStops":true,"snapshot":{"snapshotSequence":5,"thread":{"id":"t","messages":[],"activities":[],"session":{"status":"stopped","updatedAt":"old"}}}}),
        ),
    );
    item(app, cx, key, Ok(stopped(6, "old")));
    item(app, cx, key, Ok(json!({"kind":"synchronized"})));
}
fn stopped(sequence: u64, command_id: &str) -> Value {
    json!({"kind":"event","event":{"sequence":sequence,"type":"thread.session-set","occurredAt":"2026-01-01T00:01:00.123Z","payload":{"threadId":"t","session":{"status":"stopped","updatedAt":"2026-01-01T00:01:00.123Z","activeTurnId":null,"stopCommandId":command_id}}}})
}

fn failure(sequence: u64, command_id: &str) -> Value {
    json!({"kind":"event","event":{"sequence":sequence,"type":"thread.activity-appended","occurredAt":"2026-01-01T00:01:00.123Z","payload":{"threadId":"t","activity":{"id":format!("a{sequence}"),"kind":"provider.session.stop.failed","createdAt":"2026-01-01T00:01:00.123Z","sequence":sequence,"payload":{"commandId":command_id}}}}})
}
fn dispatch(
    _app: &Entity<NativeApp>,
    _cx: &mut VisualTestContext,
    _key: &str,
    requests: &Requests,
) -> (
    String,
    connection::MutationGate,
    async_channel::Sender<Result<Value, String>>,
) {
    let connection::Command::CallMutation {
        tag,
        payload,
        gate,
        reply,
    } = requests.try_recv().unwrap()
    else {
        panic!("tracked stop dispatch");
    };
    assert_eq!(tag, "orchestration.dispatchCommand");
    assert_eq!(payload["type"], "thread.session.stop");
    let command_id = payload["commandId"].as_str().unwrap().to_owned();
    assert!(!command_id.is_empty());
    (command_id, gate, reply)
}

#[gpui_kit::test]
fn checkout_stop_waits_for_authoritative_live_correlated_completion(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    let (key, result) = begin(&app, cx, &requests);
    baseline(&app, cx, &key);
    let (at, _, reply) = dispatch(&app, cx, &key, &requests);
    reply.send_blocking(Ok(json!({"sequence":7}))).unwrap();
    cx.run_until_parked();
    assert!(
        result.try_recv().is_err(),
        "persisted acceptance alone is not provider completion"
    );
    item(&app, cx, &key, Ok(stopped(7, "old")));
    assert!(result.try_recv().is_err());
    // An older worker may finish AFTER our acceptance while turn.start and
    // our own stop remain queued. A higher sequence is not causal proof.
    item(&app, cx, &key, Ok(stopped(8, "older-stop-command")));
    assert!(result.try_recv().is_err());
    item(&app, cx, &key, Ok(stopped(9, &at)));
    assert_eq!(result.try_recv().unwrap(), Ok(()));
    assert!(
        matches!(requests.try_recv().unwrap(), connection::Command::Unsubscribe(k) if k == key)
    );
}

#[gpui_kit::test]
fn checkout_stop_capable_host_unmarked_intermediate_does_not_release_or_fail(
    cx: &mut TestAppContext,
) {
    let (app, cx, requests) = harness(cx);
    let (key, result) = begin(&app, cx, &requests);
    baseline(&app, cx, &key);
    let (command_id, _, reply) = dispatch(&app, cx, &key, &requests);
    reply.send_blocking(Ok(json!({"sequence":7}))).unwrap();
    let mut intermediate = stopped(8, "unused");
    intermediate["event"]["payload"]["session"]
        .as_object_mut()
        .unwrap()
        .remove("stopCommandId");
    item(&app, cx, &key, Ok(intermediate));
    assert!(result.try_recv().is_err());
    let mut unmarked_failure = failure(9, "unused");
    unmarked_failure["event"]["payload"]["activity"]["payload"] = json!({});
    item(&app, cx, &key, Ok(unmarked_failure));
    assert!(result.try_recv().is_err());
    item(&app, cx, &key, Ok(stopped(10, &command_id)));
    assert_eq!(result.try_recv().unwrap(), Ok(()));
}

#[gpui_kit::test]
fn checkout_stop_requires_capability_before_dispatch(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    for capability in [None, Some(false)] {
        let (key, result) = begin(&app, cx, &requests);
        let mut snapshot = json!({"kind":"snapshot","snapshot":{"snapshotSequence":5,"thread":{"id":"t","messages":[],"activities":[],"session":{"status":"running"}}}});
        if let Some(value) = capability {
            snapshot["supportsCorrelatedSessionStops"] = json!(value);
        }
        item(&app, cx, &key, Ok(snapshot));
        let error = result.try_recv().unwrap().unwrap_err();
        assert!(error.contains("Update the host server"), "{error}");
        // No dispatch (and therefore no checkout mutation) on older hosts,
        // even if a later synchronization marker arrives for this key.
        item(&app, cx, &key, Ok(json!({"kind":"synchronized"})));
        let mut unsubscribed = false;
        while let Ok(command) = requests.try_recv() {
            assert!(matches!(command, connection::Command::Unsubscribe(k) if k == key));
            unsubscribed = true;
        }
        assert!(unsubscribed);
    }
}

#[gpui_kit::test]
fn checkout_stop_completion_before_acceptance_is_retained(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    let (key, result) = begin(&app, cx, &requests);
    baseline(&app, cx, &key);
    let (at, _, reply) = dispatch(&app, cx, &key, &requests);
    item(&app, cx, &key, Ok(stopped(8, &at)));
    assert!(result.try_recv().is_err());
    reply.send_blocking(Ok(json!({"sequence":7}))).unwrap();
    cx.run_until_parked();
    assert_eq!(result.try_recv().unwrap(), Ok(()));
}

#[gpui_kit::test]
fn checkout_stop_old_failure_is_ignored_owned_failure_rejects(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    let (key, result) = begin(&app, cx, &requests);
    baseline(&app, cx, &key);
    let (at, _, reply) = dispatch(&app, cx, &key, &requests);
    reply.send_blocking(Ok(json!({"sequence":7}))).unwrap();
    item(&app, cx, &key, Ok(failure(7, &at)));
    assert!(result.try_recv().is_err());
    item(&app, cx, &key, Ok(failure(8, "older-stop-command")));
    assert!(result.try_recv().is_err());
    item(&app, cx, &key, Ok(failure(9, &at)));
    assert!(
        result
            .try_recv()
            .unwrap()
            .unwrap_err()
            .contains("stop failed")
    );
}

#[gpui_kit::test]
fn checkout_stop_invalid_disconnected_resynchronized_and_stale_streams_fail_closed(
    cx: &mut TestAppContext,
) {
    let (app, cx, requests) = harness(cx);
    for mode in 0..5 {
        while requests.try_recv().is_ok() {}
        let (key, result) = begin(&app, cx, &requests);
        baseline(&app, cx, &key);
        let (at, gate, reply) = dispatch(&app, cx, &key, &requests);
        // A replaced key cannot affect the current waiter.
        item(&app, cx, "checkout-stop-stale", Err("stale".into()));
        assert!(result.try_recv().is_err());
        match mode {
            0 => item(&app, cx, &key, Ok(json!({"kind":"invalid"}))),
            1 => item(&app, cx, &key, Err("Disconnected".into())),
            2 => item(&app, cx, &key, Ok(json!({"kind":"snapshot"}))),
            3 => {
                cx.update(|_, cx| app.update(cx, |app, _| app.generation += 1));
                item(&app, cx, &key, Ok(stopped(8, &at)));
            }
            _ => {
                // Even buffered success cannot survive context invalidation.
                item(&app, cx, &key, Ok(stopped(8, &at)));
                cx.update(|_, cx| app.update(cx, |app, _| app.clear_checkout_stops()));
            }
        }
        assert!(
            !gate.may_have_been_sent(),
            "mock never dispatched the transport gate"
        );
        reply.send_blocking(Ok(json!({"sequence":7}))).unwrap();
        cx.run_until_parked();
        assert!(result.try_recv().unwrap().is_err());
    }
}

#[gpui_kit::test]
fn checkout_stop_buffered_completion_is_invalidated_by_reengagement(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    let (key, result) = begin(&app, cx, &requests);
    baseline(&app, cx, &key);
    let (at, _, reply) = dispatch(&app, cx, &key, &requests);
    item(&app, cx, &key, Ok(stopped(8, &at)));
    let mut running = stopped(9, &at);
    running["event"]["payload"]["session"]["status"] = json!("running");
    running["event"]["payload"]["session"]
        .as_object_mut()
        .unwrap()
        .remove("stopCommandId");
    item(&app, cx, &key, Ok(running));
    reply.send_blocking(Ok(json!({"sequence":7}))).unwrap();
    cx.run_until_parked();
    assert!(
        result.try_recv().is_err(),
        "a newer active session must not permit checkout"
    );
    item(&app, cx, &key, Ok(stopped(10, &at)));
    assert_eq!(result.try_recv().unwrap(), Ok(()));
}

#[gpui_kit::test]
fn checkout_stop_completion_must_follow_owned_receipt_floor(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    let (key, result) = begin(&app, cx, &requests);
    baseline(&app, cx, &key);
    let (at, _, reply) = dispatch(&app, cx, &key, &requests);
    item(&app, cx, &key, Ok(stopped(8, &at)));
    reply.send_blocking(Ok(json!({"sequence":8}))).unwrap();
    cx.run_until_parked();
    assert!(result.try_recv().is_err());
    item(&app, cx, &key, Ok(stopped(9, &at)));
    assert_eq!(result.try_recv().unwrap(), Ok(()));
}

#[gpui_kit::test]
fn checkout_stop_canceled_before_observation_never_enqueues_mutation(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    let (key, result) = begin(&app, cx, &requests);
    cx.update(|_, cx| app.update(cx, |app, _| app.clear_checkout_stops()));
    cx.run_until_parked();
    assert!(result.try_recv().unwrap().is_err());
    while let Ok(command) = requests.try_recv() {
        assert!(matches!(command, connection::Command::Unsubscribe(_)));
    }
    item(&app, cx, &key, Ok(stopped(8, "stale")));
    assert!(requests.try_recv().is_err());
}
