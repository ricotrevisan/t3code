use super::*;
use serde_json::Value;

fn reference(number: u64) -> Value {
    json!({"projectId":"p","host":"github.com","repository":"acme/repo","number":number})
}
fn row(number: u64) -> Value {
    json!({"projectId":"p","host":"github.com","repository":"acme/repo","number":number,"title":format!("Change {number}"),"url":format!("https://github.com/acme/repo/pull/{number}"),"state":"open","updatedAt":"2026-10-01"})
}
fn detail(number: u64) -> Value {
    let mut d = row(number);
    d["provider"] = json!("github");
    d["body"] = json!("Description");
    d["headBranch"] = json!("feature");
    d["baseBranch"] = json!("main");
    d["isDraft"] = json!(false);
    d["mergeability"] = json!("mergeable");
    d["capabilities"] = json!({"diff":true,"comment":true,"labels":true,"actions":["merge","close","reopen","ready","draft"],"mergeMethods":["squash"],"viewedFiles":"host","review":{"inlineComment":true,"verdicts":["comment","approve","request-changes"]},"reviewers":{"request":true,"listCandidates":true}});
    d["viewerPermissions"] = json!({"actions":["merge","close","reopen","ready","draft"],"comment":true,"verdicts":["comment","approve","request-changes"],"requestReviewers":true,"labels":true});
    d["mergeCapabilities"] = json!({"squash":true});
    d["labels"] = json!([]);
    d["reviewers"] = json!([]);
    d["checks"] = json!([{"name":"CI","status":"failure","description":"Tests failed","url":"https://github.com/acme/repo/actions/runs/1"}]);
    d
}
fn take(
    requests: &Requests,
    expected: &str,
) -> (Value, async_channel::Sender<Result<Value, String>>) {
    let connection::Command::Call {
        tag,
        payload,
        reply,
    } = requests.try_recv().expect("request")
    else {
        panic!("expected RPC")
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
fn list(app: &Entity<NativeApp>, cx: &mut VisualTestContext, requests: &Requests) {
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.sidebar_state.capabilities = json!({"threadPullRequests":true});
            app.run_palette_item(settings_palette::Target::PullRequests, window, cx);
        })
    });
    cx.run_until_parked();
    let (payload, reply) = take(requests, "pullRequests.list");
    assert_eq!(
        payload,
        json!({"state":"open","involvement":"all","limit":50})
    );
    reply.send_blocking(Ok(json!({"entries":[row(7)],"errors":[],"providers":[],"truncated":false,"nextCursors":{}}))).unwrap();
    cx.run_until_parked();
}
fn opened(app: &Entity<NativeApp>, cx: &mut VisualTestContext, requests: &Requests) {
    list(app, cx, requests);
    click(cx, "pr-row-0");
    finish_detail(cx, requests, 7);
}
fn finish_detail(cx: &mut VisualTestContext, requests: &Requests, number: u64) {
    let (payload, reply) = take(requests, "pullRequests.detail");
    assert_eq!(payload, reference(number));
    let (payload, activity) = take(requests, "pullRequests.activity");
    assert_eq!(payload, reference(number));
    reply.send_blocking(Ok(detail(number))).unwrap();
    activity.send_blocking(Ok(json!({"comments":[{"body":"A real comment","author":{"login":"reviewer"},"kind":"issue-comment"}],"reviewThreads":[],"commits":[],"commentCount":1,"commentsTruncated":false}))).unwrap();
    cx.run_until_parked();
    let (payload, viewed) = take(requests, "pullRequests.filesViewed");
    assert_eq!(payload, reference(number));
    viewed
        .send_blocking(Ok(
            json!({"files":[{"path":"src/a.rs","state":"unviewed"}],"truncated":false}),
        ))
        .unwrap();
    cx.run_until_parked();
}
#[gpui_kit::test]
fn r14_p_palette_list_search_and_detail_checks_use_real_receipts(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    list(&app, cx, &requests);
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.pull_requests
                .query
                .as_ref()
                .unwrap()
                .update(cx, |input, cx| input.set_value("parser", window, cx))
        })
    });
    click(cx, "pr-search");
    let (payload, reply) = take(&requests, "pullRequests.list");
    assert_eq!(payload["query"], "parser");
    reply
        .send_blocking(Ok(
            json!({"entries":[row(7)],"errors":[],"providers":[],"nextCursors":{}}),
        ))
        .unwrap();
    cx.run_until_parked();
    click(cx, "pr-row-0");
    finish_detail(cx, &requests, 7);
    click(cx, "pr-checks");
    assert!(cx.debug_bounds("pr-checks-popover").is_some());
    assert_eq!(
        app.read_with(cx, |app, _| {
            app.pull_requests.activity["comments"][0]["body"].clone()
        }),
        "A real comment"
    );
    click(cx, "pr-checks");
    assert!(cx.debug_bounds("pr-checks-popover").is_none());
}
#[gpui_kit::test]
fn r14_p_comment_failure_keeps_draft_success_consumes_only_submitted_text(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    opened(&app, cx, &requests);
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.pull_requests
                .draft()
                .unwrap()
                .comment
                .update(cx, |input, cx| {
                    input.set_value("Please fix this", window, cx)
                })
        })
    });
    click(cx, "pr-comment-submit");
    let (payload, reply) = take(&requests, "pullRequests.comment");
    assert_eq!(payload["body"], "Please fix this");
    assert_eq!(payload["repository"], "acme/repo");
    click(cx, "pr-comment-submit");
    assert!(requests.is_empty());
    reply.send_blocking(Err("No permission".into())).unwrap();
    cx.run_until_parked();
    assert_eq!(
        app.read_with(cx, |app, cx| app
            .pull_requests
            .draft()
            .unwrap()
            .comment
            .read(cx)
            .value()
            .to_string()),
        "Please fix this"
    );
    click(cx, "pr-comment-submit");
    let (_, reply) = take(&requests, "pullRequests.comment");
    reply.send_blocking(Ok(Value::Null)).unwrap();
    cx.run_until_parked();
    assert!(app.read_with(cx, |app, cx| {
        app.pull_requests
            .draft()
            .unwrap()
            .comment
            .read(cx)
            .value()
            .is_empty()
    }));
    finish_detail(cx, &requests, 7);
}
#[gpui_kit::test]
fn r14_p_review_failure_keeps_inline_positions_and_success_clears_snapshot(
    cx: &mut TestAppContext,
) {
    let (app, cx, requests) = harness(cx);
    opened(&app, cx, &requests);
    let inline = json!({"path":"src/a.rs","position":{"kind":"added","newLine":3},"body":"Handle empty input"});
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            let key = app.pull_requests.key().unwrap();
            let draft = app.pull_requests.drafts.get_mut(&key).unwrap();
            draft.comments.push(inline.clone());
            draft
                .review
                .update(cx, |input, cx| input.set_value("Needs a fix", window, cx));
        })
    });
    cx.run_until_parked();
    click(cx, "pr-verdict-request-changes");
    click(cx, "pr-review-submit");
    let (payload, reply) = take(&requests, "pullRequests.submitReview");
    assert_eq!(payload["comments"], json!([inline]));
    assert_eq!(payload["verdict"], "request-changes");
    reply.send_blocking(Err("Review denied".into())).unwrap();
    cx.run_until_parked();
    assert_eq!(
        app.read_with(cx, |app, _| app
            .pull_requests
            .draft()
            .unwrap()
            .comments
            .len()),
        1
    );
    click(cx, "pr-review-submit");
    let (_, reply) = take(&requests, "pullRequests.submitReview");
    reply.send_blocking(Ok(Value::Null)).unwrap();
    cx.run_until_parked();
    assert!(app.read_with(cx, |app, cx| {
        app.pull_requests.draft().unwrap().comments.is_empty()
            && app
                .pull_requests
                .draft()
                .unwrap()
                .review
                .read(cx)
                .value()
                .is_empty()
    }));
    finish_detail(cx, &requests, 7);
}
#[gpui_kit::test]
fn r14_p_candidate_pick_and_reverse_use_host_ids(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    opened(&app, cx, &requests);
    click(cx, "pr-labels");
    let (payload, reply) = take(&requests, "pullRequests.labelCandidates");
    assert_eq!(payload, reference(7));
    reply
        .send_blocking(Ok(
            json!({"candidates":[{"name":"bug","isApplied":false}],"truncated":false}),
        ))
        .unwrap();
    cx.run_until_parked();
    click(cx, "pr-candidate-0");
    let (payload, reply) = take(&requests, "pullRequests.setLabels");
    assert_eq!(payload["labels"], json!(["bug"]));
    assert_eq!(payload["applied"], true);
    reply.send_blocking(Err("Denied".into())).unwrap();
    cx.run_until_parked();
    cx.update(|_, cx| app.update(cx, |app, cx| app.pr_candidates("reviewers", cx)));
    cx.run_until_parked();
    let (_, reply) = take(&requests, "pullRequests.reviewerCandidates");
    reply.send_blocking(Ok(json!({"candidates":[{"login":"Bob","id":"host-user-42","kind":"user","isRequested":true}],"truncated":false}))).unwrap();
    cx.run_until_parked();
    click(cx, "pr-candidate-0");
    let (payload, reply) = take(&requests, "pullRequests.requestReviewers");
    assert_eq!(
        payload["reviewers"],
        json!([{"id":"host-user-42","kind":"user"}])
    );
    assert_eq!(payload["requested"], false);
    reply.send_blocking(Err("Denied".into())).unwrap();
    cx.run_until_parked();
}
#[gpui_kit::test]
fn r14_p_merge_confirmation_is_retired_on_resource_and_environment_change(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    opened(&app, cx, &requests);
    cx.update(|_, cx| app.update(cx, |app, cx| app.pr_merge(false, cx)));
    assert!(requests.is_empty());
    cx.update(|window, cx| app.update(cx, |app, cx| app.pr_open(reference(8), window, cx)));
    cx.run_until_parked();
    finish_detail(cx, &requests, 8);
    cx.update(|_, cx| app.update(cx, |app, cx| app.pr_merge(true, cx)));
    assert!(requests.is_empty());
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.pr_merge(false, cx);
            app.generation += 1;
            app.pr_merge(true, cx);
        })
    });
    assert!(requests.is_empty());
}
#[gpui_kit::test]
fn r14_p_stale_detail_cannot_replace_new_pr_projection(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    list(&app, cx, &requests);
    click(cx, "pr-row-0");
    let (_, old) = take(&requests, "pullRequests.detail");
    let (_, activity) = take(&requests, "pullRequests.activity");
    cx.update(|window, cx| app.update(cx, |app, cx| app.pr_open(reference(8), window, cx)));
    cx.run_until_parked();
    finish_detail(cx, &requests, 8);
    old.send_blocking(Ok(detail(7))).unwrap();
    activity
        .send_blocking(Ok(json!({"comments":[{"body":"stale"}]})))
        .unwrap();
    cx.run_until_parked();
    assert_eq!(
        app.read_with(cx, |app, _| app.pull_requests.detail["number"].clone()),
        8
    );
    assert!(requests.is_empty());
}
#[gpui_kit::test]
fn r14_p_link_and_unlink_dispatch_correct_thread_and_host(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    opened(&app, cx, &requests);
    click(cx, "pr-link");
    let (payload, reply) = take(&requests, "orchestration.dispatchCommand");
    assert_eq!(payload["type"], "thread.pull-request.link");
    assert_eq!(payload["threadId"], "t");
    assert_eq!(payload["host"], "github.com");
    assert_eq!(payload["source"], "manual");
    assert_eq!(payload["url"], "https://github.com/acme/repo/pull/7");
    reply.send_blocking(Err("Link refused".into())).unwrap();
    cx.run_until_parked();
    assert!(app.read_with(cx, |app, _| {
        app.pull_requests.errors.contains_key("mutation")
    }));
    cx.update(|_, cx| app.update(cx, |app, cx| app.pr_link(true, cx)));
    cx.run_until_parked();
    let (payload, reply) = take(&requests, "orchestration.dispatchCommand");
    assert_eq!(payload["type"], "thread.pull-request.unlink");
    assert!(payload.get("source").is_none());
    reply.send_blocking(Err("Unlink refused".into())).unwrap();
    cx.run_until_parked();
}

#[gpui_kit::test]
fn r14_p_accepted_comment_consumes_captured_draft_after_navigation(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    opened(&app, cx, &requests);
    let key = app.read_with(cx, |app, _| app.pull_requests.key().unwrap());
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.pull_requests
                .draft()
                .unwrap()
                .comment
                .update(cx, |input, cx| {
                    input.set_value("Captured seven", window, cx)
                });
            app.pr_comment(window, cx);
        })
    });
    cx.run_until_parked();
    let (_, reply) = take(&requests, "pullRequests.comment");
    cx.update(|window, cx| app.update(cx, |app, cx| app.pr_open(reference(8), window, cx)));
    cx.run_until_parked();
    finish_detail(cx, &requests, 8);
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.pull_requests
                .draft()
                .unwrap()
                .comment
                .update(cx, |input, cx| input.set_value("Eight remains", window, cx))
        })
    });
    reply.send_blocking(Ok(Value::Null)).unwrap();
    cx.run_until_parked();
    app.read_with(cx, |app, cx| {
        assert!(
            app.pull_requests.drafts[&key]
                .comment
                .read(cx)
                .value()
                .is_empty()
        );
        assert_eq!(
            app.pull_requests
                .draft()
                .unwrap()
                .comment
                .read(cx)
                .value()
                .as_str(),
            "Eight remains"
        );
    });
    assert!(requests.is_empty());
}
#[gpui_kit::test]
fn r14_p_merge_and_viewed_mutations_are_receipt_backed(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    opened(&app, cx, &requests);
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.pr_merge(false, cx);
            app.pr_merge(true, cx);
        })
    });
    cx.run_until_parked();
    let (payload, reply) = take(&requests, "pullRequests.runAction");
    assert_eq!(
        payload,
        json!({"projectId":"p","host":"github.com","repository":"acme/repo","number":7,"action":"merge","mergeMethod":"squash"})
    );
    reply
        .send_blocking(Err("Required checks have not passed".into()))
        .unwrap();
    cx.run_until_parked();
    assert_eq!(
        app.read_with(cx, |app, _| app.pull_requests.detail["state"].clone()),
        "open"
    );
    cx.update(|_, cx| app.update(cx, |app, cx| app.pr_viewed("src/a.rs".into(), true, cx)));
    cx.run_until_parked();
    let (payload, reply) = take(&requests, "pullRequests.setFilesViewed");
    assert_eq!(payload["files"], json!([{"path":"src/a.rs","viewed":true}]));
    assert_eq!(
        app.read_with(cx, |app, _| app.pull_requests.viewed["files"][0]["state"]
            .clone()),
        "unviewed"
    );
    reply
        .send_blocking(Err("Host refused viewed mark".into()))
        .unwrap();
    cx.run_until_parked();
    assert_eq!(
        app.read_with(cx, |app, _| app.pull_requests.viewed["files"][0]["state"]
            .clone()),
        "unviewed"
    );
}
#[gpui_kit::test]
fn r14_p_pr_handoff_uses_owned_context_and_preserves_rejected_turn(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    opened(&app, cx, &requests);
    cx.update(|_, cx| {
        app.update(cx, |app, _| {
            app.provider_catalog = vec![super::composer_parity::provider_fixture(
                "codex", "Personal",
            )]
        })
    });
    click(cx, "pr-handoff");
    cx.update(|window, cx| app.update(cx, |app, cx| app.send_prompt(window, cx)));
    cx.run_until_parked();
    let (payload, reply) = take(&requests, "orchestration.dispatchCommand");
    assert_eq!(payload["type"], "thread.turn.start");
    let context = &payload["message"]["context"]["records"][0];
    assert_eq!(context["kind"], "review-comment");
    assert_eq!(context["sectionId"], "pull-request:7");
    assert_eq!(context["pullRequest"], json!({"number":7,"title":"Change 7","url":"https://github.com/acme/repo/pull/7","headBranch":"feature","baseBranch":"main","state":"open","isDraft":false}));
    assert!(
        context["text"]
            .as_str()
            .unwrap()
            .contains("https://github.com/acme/repo/pull/7")
    );
    assert!(context["text"].as_str().unwrap().contains("untrusted data"));
    reply.send_blocking(Err("Turn rejected".into())).unwrap();
    cx.run_until_parked();
    assert_eq!(
        app.read_with(cx, |app, _| app.draft_context_records().len()),
        1
    );
}

#[gpui_kit::test]
async fn r14_p_code_http_receipt_line_capture_review_and_viewed_failure(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    let patch = "diff --git a/src/a.rs b/src/a.rs\n--- a/src/a.rs\n+++ b/src/a.rs\n@@ -1,2 +1,2 @@\n context\n-old\n+new\n";
    let body = json!({"patch":patch,"nextCursor":null,"truncated":false}).to_string();
    let (origin, server) = runtime_pull_requests::http_tests::endpoint("200 OK", &body);
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.server
                .update(cx, |input, cx| input.set_value(origin.clone(), window, cx));
            app.environment_scope = origin.clone();
            app.access = Some(auth::Access {
                origin: origin.clone(),
                token: "fixture-token".into(),
                proof_key: None,
            });
        })
    });
    opened(&app, cx, &requests);
    let (receipt_tx, receipt_rx) = async_channel::bounded(1);
    let _receipt_subscription = cx.update(|_, cx| {
        app.update(cx, |_, cx| {
            cx.subscribe(
                &app,
                move |_, _, receipt: &runtime_pull_requests::DiffReceipt, _| {
                    receipt_tx.try_send(receipt.clone()).unwrap();
                },
            )
        })
    });
    click(cx, "pr-code-tab");
    let receipt = receipt_rx.recv().await.unwrap();
    assert!(receipt.succeeded);
    assert_eq!(receipt.reference, reference(7));
    cx.run_until_parked();
    let request = server.join().unwrap();
    assert!(request.starts_with("POST /api/pull-requests/diff HTTP/1.1"));
    assert_eq!(
        serde_json::from_str::<Value>(request.split_once("\r\n\r\n").unwrap().1).unwrap(),
        reference(7)
    );
    click(cx, "pr-line-6");
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.pull_requests
                .inline_body
                .as_ref()
                .unwrap()
                .update(cx, |input, cx| {
                    input.set_value("Guard this new line", window, cx)
                })
        })
    });
    cx.run_until_parked();
    click(cx, "pr-inline-add");
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.pull_requests
                .draft()
                .unwrap()
                .review
                .update(cx, |input, cx| {
                    input.set_value("Please address this", window, cx)
                })
        })
    });
    cx.run_until_parked();
    click(cx, "pr-review-submit");
    let (payload, reply) = take(&requests, "pullRequests.submitReview");
    assert_eq!(
        payload["comments"],
        json!([{"path":"src/a.rs","position":{"kind":"added","newLine":2},"body":"Guard this new line"}])
    );
    reply.send_blocking(Ok(Value::Null)).unwrap();
    cx.run_until_parked();
    finish_detail(cx, &requests, 7);
    click(cx, "pr-file-viewed");
    let (payload, reply) = take(&requests, "pullRequests.setFilesViewed");
    assert_eq!(payload["files"], json!([{"path":"src/a.rs","viewed":true}]));
    reply.send_blocking(Err("Viewed denied".into())).unwrap();
    cx.run_until_parked();
    assert_eq!(
        app.read_with(cx, |app, _| app.pull_requests.viewed["files"][0]["state"]
            .clone()),
        "unviewed"
    );
}
