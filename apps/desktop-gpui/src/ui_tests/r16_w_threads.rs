use super::*;
use serde_json::Value;

#[gpui_kit::test]
fn r16_w_reply_draft_identity_survives_reordered_reference_and_cancel(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    open(&app, cx, &requests);
    click(cx, "pr-thread-reply-t");
    type_reply(&app, cx, "Retained conversation draft");
    click(cx, "pr-thread-cancel-t");
    assert!(cx.debug_bounds("pr-thread-send-t").is_none());
    click(cx, "pr-thread-reply-t");
    assert_eq!(reply_text(&app, cx, 7), "Retained conversation draft");
    // The list adds host after repository/number; a header or caller may order it first.
    cx.update(|window, cx| app.update(cx, |app, cx| app.pr_open(reference(7), window, cx)));
    cx.run_until_parked();
    finish(cx, &requests, 7, false, true);
    assert_eq!(
        app.read_with(cx, |app, _| app.pull_requests.drafts.len()),
        1
    );
    click(cx, "pr-thread-reply-t");
    assert_eq!(reply_text(&app, cx, 7), "Retained conversation draft");
    assert!(requests.is_empty());
}

#[gpui_kit::test]
fn r16_w_reactions_denied_add_remove_and_description_subject(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    open(&app, cx, &requests);
    click(cx, "pr-reactions-comment-c1");
    click(cx, "pr-reaction-comment-c1-heart");
    let (payload, reply) = take(&requests, "pullRequests.setReaction");
    let mut expected = reference(7);
    expected["subjectId"] = json!("c1");
    expected["content"] = json!("heart");
    expected["reacted"] = json!(true);
    assert_eq!(payload, expected);
    reply.send_blocking(Err("Reaction denied".into())).unwrap();
    cx.run_until_parked();
    assert!(cx.debug_bounds("pr-reaction-comment-c1-heart").is_none());
    click(cx, "pr-reactions-comment-c1");
    click(cx, "pr-reaction-comment-c1-heart");
    let (_, reply) = take(&requests, "pullRequests.setReaction");
    reply.send_blocking(Ok(Value::Null)).unwrap();
    cx.run_until_parked();
    let (_, detail_reply) = take(&requests, "pullRequests.detail");
    let (_, activity_reply) = take(&requests, "pullRequests.activity");
    detail_reply.send_blocking(Ok(detail(7, true))).unwrap();
    activity_reply.send_blocking(Ok(json!({"comments":[],"commits":[],"reviewThreads":[{"id":"t","path":"src/a.rs","line":2,"isResolved":false,"comments":[{"id":"c1","body":"Please explain","author":{"login":"reviewer"},"reactions":[{"content":"heart","count":2,"actors":["reviewer"],"viewerHasReacted":true}]}]}]}))).unwrap();
    cx.run_until_parked();
    click(cx, "pr-reaction-comment-c1-heart");
    let (payload, reply) = take(&requests, "pullRequests.setReaction");
    assert_eq!(payload["reacted"], false);
    assert_eq!(payload["subjectId"], "c1");
    reply.send_blocking(Ok(Value::Null)).unwrap();
    cx.run_until_parked();
    finish(cx, &requests, 7, false, true);
    click(cx, "pr-reactions-description");
    click(cx, "pr-reaction-description-thumbs-up");
    let (payload, reply) = take(&requests, "pullRequests.setReaction");
    assert!(payload.get("subjectId").is_none());
    assert_eq!(payload["content"], "thumbs-up");
    reply.send_blocking(Err("Denied".into())).unwrap();
    cx.run_until_parked();
    // A subject that disappeared from refreshed activity cannot be mutated by a stale action.
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.pr_react(Some("missing".into()), "heart", cx)
        })
    });
    cx.run_until_parked();
    assert!(requests.is_empty());
}

fn reference(number: u64) -> Value {
    json!({"projectId":"p","host":"github.com","repository":"acme/repo","number":number})
}
fn take(
    requests: &Requests,
    expected: &str,
) -> (Value, async_channel::Sender<Result<Value, String>>) {
    let connection::Command::Call {
        tag,
        payload,
        reply,
    } = requests.try_recv().expect("RPC")
    else {
        panic!("call")
    };
    assert_eq!(tag, expected);
    (payload, reply)
}
fn click(cx: &mut VisualTestContext, id: &'static str) {
    let bounds = cx
        .debug_bounds(id)
        .unwrap_or_else(|| panic!("missing {id}"));
    cx.simulate_click(bounds.center(), Default::default());
    cx.run_until_parked();
}
fn detail(number: u64, allowed: bool) -> Value {
    let mut d = reference(number);
    d["title"] = json!("Conversation change");
    d["state"] = json!("open");
    d["url"] = json!(format!("https://github.com/acme/repo/pull/{number}"));
    d["headBranch"] = json!("feature");
    d["baseBranch"] = json!("main");
    d["capabilities"] =
        json!({"reactions":true,"review":{"reply":true,"resolve":true,"verdicts":[]}});
    d["viewerPermissions"] = json!({"comment":allowed,"resolve":allowed,"verdicts":[]});
    d
}
fn finish(
    cx: &mut VisualTestContext,
    requests: &Requests,
    number: u64,
    resolved: bool,
    allowed: bool,
) {
    let (payload, reply) = take(requests, "pullRequests.detail");
    assert_eq!(payload, reference(number));
    let (_, activity) = take(requests, "pullRequests.activity");
    reply.send_blocking(Ok(detail(number, allowed))).unwrap();
    activity.send_blocking(Ok(json!({"comments":[],"commits":[],"commentCount":0,"commentsTruncated":false,"reviewThreads":[{"id":"t","path":"src/a.rs","line":2,"side":"right","isResolved":resolved,"isOutdated":false,"comments":[{"id":"c1","author":{"login":"reviewer"},"body":"Please explain","createdAt":"2026-10-01"}],"commentCount":3,"nextCommentsCursor":"page2"}]}))).unwrap();
    cx.run_until_parked();
}
fn open(app: &Entity<NativeApp>, cx: &mut VisualTestContext, requests: &Requests) {
    cx.update(|window, cx| app.update(cx, |app, cx| app.open_pull_requests(window, cx)));
    cx.run_until_parked();
    let (_, reply) = take(requests, "pullRequests.list");
    let mut row = detail(7, true);
    row["updatedAt"] = json!("2026-10-01");
    reply
        .send_blocking(Ok(json!({"entries":[row],"nextCursors":{},"errors":[]})))
        .unwrap();
    cx.run_until_parked();
    click(cx, "pr-row-0");
    finish(cx, requests, 7, false, true);
}
fn type_reply(app: &Entity<NativeApp>, cx: &mut VisualTestContext, text: &str) {
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.pull_requests.draft().unwrap().replies["t"]
                .input
                .update(cx, |input, cx| input.set_value(text, window, cx));
        })
    });
    cx.run_until_parked();
}
fn reply_text(app: &Entity<NativeApp>, cx: &VisualTestContext, number: u64) -> String {
    app.read_with(cx, |app, cx| {
        let key = app.pull_requests.resource_key(&reference(number));
        app.pull_requests.drafts[&key].replies["t"]
            .input
            .read(cx)
            .value()
            .to_string()
    })
}

#[gpui_kit::test]
fn r16_w_reply_denial_preserves_text_acceptance_consumes_only_captured_pr(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    open(&app, cx, &requests);
    click(cx, "pr-thread-reply-t");
    cx.simulate_input("Unicode reply café 🙂");
    cx.run_until_parked();
    click(cx, "pr-thread-send-t");
    let (payload, reply) = take(&requests, "pullRequests.replyToThread");
    let mut expected = reference(7);
    expected["threadId"] = json!("t");
    expected["body"] = json!("Unicode reply café 🙂");
    assert_eq!(payload, expected);
    click(cx, "pr-thread-send-t");
    assert!(requests.is_empty());
    reply
        .send_blocking(Err("Host denied reply".into()))
        .unwrap();
    cx.run_until_parked();
    assert_eq!(reply_text(&app, cx, 7), "Unicode reply café 🙂");
    assert!(app.read_with(cx, |app, _| {
        app.pull_requests.errors["mutation"].contains("denied")
    }));
    click(cx, "pr-thread-send-t");
    let (_, reply) = take(&requests, "pullRequests.replyToThread");
    // Navigate to a distinct resource with the same host thread ID while the write is in flight.
    cx.update(|window, cx| app.update(cx, |app, cx| app.pr_open(reference(8), window, cx)));
    cx.run_until_parked();
    finish(cx, &requests, 8, false, true);
    click(cx, "pr-thread-reply-t");
    type_reply(&app, cx, "PR eight owns this");
    reply.send_blocking(Ok(Value::Null)).unwrap();
    cx.run_until_parked();
    assert_eq!(reply_text(&app, cx, 7), "");
    assert_eq!(reply_text(&app, cx, 8), "PR eight owns this");
    assert!(requests.is_empty());
}

#[gpui_kit::test]
fn r16_w_reply_edited_during_receipt_and_permission_revocation_are_safe(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    open(&app, cx, &requests);
    click(cx, "pr-thread-reply-t");
    type_reply(&app, cx, "captured");
    click(cx, "pr-thread-send-t");
    let (_, reply) = take(&requests, "pullRequests.replyToThread");
    type_reply(&app, cx, "newer text");
    reply.send_blocking(Ok(Value::Null)).unwrap();
    cx.run_until_parked();
    finish(cx, &requests, 7, false, false);
    assert_eq!(reply_text(&app, cx, 7), "newer text");
    click(cx, "pr-thread-send-t");
    click(cx, "pr-thread-resolve-t");
    assert!(requests.is_empty());
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.pr_reply("t".into(), window, cx);
            app.pr_resolve("t".into(), true, cx);
        })
    });
    cx.run_until_parked();
    assert!(requests.is_empty());
}

#[gpui_kit::test]
fn r16_w_resolution_reverse_and_denied_receipt_do_not_optimistically_change_status(
    cx: &mut TestAppContext,
) {
    let (app, cx, requests) = harness(cx);
    open(&app, cx, &requests);
    click(cx, "pr-thread-resolve-t");
    let (payload, reply) = take(&requests, "pullRequests.setThreadResolution");
    let mut expected = reference(7);
    expected["threadId"] = json!("t");
    expected["resolved"] = json!(true);
    assert_eq!(payload, expected);
    assert_eq!(
        app.read_with(cx, |app, _| app.pull_requests.activity["reviewThreads"][0]
            ["isResolved"]
            .clone()),
        false
    );
    reply.send_blocking(Err("Cannot resolve".into())).unwrap();
    cx.run_until_parked();
    assert_eq!(
        app.read_with(cx, |app, _| app.pull_requests.activity["reviewThreads"][0]
            ["isResolved"]
            .clone()),
        false
    );
    click(cx, "pr-thread-resolve-t");
    let (_, reply) = take(&requests, "pullRequests.setThreadResolution");
    reply.send_blocking(Ok(Value::Null)).unwrap();
    cx.run_until_parked();
    finish(cx, &requests, 7, true, true);
    click(cx, "pr-thread-resolve-t");
    let (payload, reply) = take(&requests, "pullRequests.setThreadResolution");
    assert_eq!(payload["resolved"], false);
    reply.send_blocking(Ok(Value::Null)).unwrap();
    cx.run_until_parked();
    finish(cx, &requests, 7, false, true);
}

#[gpui_kit::test]
fn r16_w_thread_pagination_retry_dedup_and_refresh_fence(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    open(&app, cx, &requests);
    click(cx, "pr-thread-more-t");
    let (payload, reply) = take(&requests, "pullRequests.threadComments");
    let mut expected = reference(7);
    expected["threadId"] = json!("t");
    expected["cursor"] = json!("page2");
    assert_eq!(payload, expected);
    reply.send_blocking(Err("Host unavailable".into())).unwrap();
    cx.run_until_parked();
    click(cx, "pr-thread-more-t");
    let (_, reply) = take(&requests, "pullRequests.threadComments");
    reply.send_blocking(Ok(json!({"comments":[{"id":"c1","body":"duplicate"},{"id":"c2","body":"new comment"}],"nextCursor":null}))).unwrap();
    cx.run_until_parked();
    assert_eq!(
        app.read_with(cx, |app, _| app.pull_requests.activity["reviewThreads"][0]
            ["comments"]
            .as_array()
            .unwrap()
            .len()),
        2
    );
    assert!(cx.debug_bounds("pr-thread-more-t").is_none());
    cx.update(|_, cx| app.update(cx, |app, cx| app.pr_refresh(cx)));
    cx.run_until_parked();
    finish(cx, &requests, 7, false, true);
    click(cx, "pr-thread-more-t");
    let (_, late) = take(&requests, "pullRequests.threadComments");
    cx.update(|_, cx| app.update(cx, |app, cx| app.pr_refresh(cx)));
    cx.run_until_parked();
    finish(cx, &requests, 7, true, true);
    late.send_blocking(Ok(
        json!({"comments":[{"id":"obsolete","body":"must not append"}],"nextCursor":null}),
    ))
    .unwrap();
    cx.run_until_parked();
    assert_eq!(
        app.read_with(cx, |app, _| app.pull_requests.activity["reviewThreads"][0]
            ["comments"]
            .as_array()
            .unwrap()
            .len()),
        1
    );
    assert!(app.read_with(cx, |app, _| {
        !app.pull_requests.loading.contains("thread-comments")
    }));
}
