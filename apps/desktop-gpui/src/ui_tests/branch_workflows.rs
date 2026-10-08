//! Receipt-driven product workflow tests; no server, Git, provider, or timers.
use super::*;
use crate::app::branch_workflows::BranchWorkflow;

fn call(
    requests: &Requests,
    tag: &str,
) -> (Value, async_channel::Sender<Result<Value, String>>) {
    let (actual, payload, reply) = loop {
        match requests.try_recv().expect("expected RPC") {
            connection::Command::Call {
                tag,
                payload,
                reply,
            }
            | connection::Command::CallMutation {
                tag,
                payload,
                reply,
                ..
            } => break (tag, payload, reply),
            connection::Command::Unsubscribe(key) if key.starts_with("checkout-stop-") => continue,
            _ => panic!("expected {tag}"),
        }
    };
    assert_eq!(actual, tag);
    (payload, reply)
}
pub(super) fn stop_observation(
    app: &Entity<NativeApp>,
    cx: &mut VisualTestContext,
    requests: &Requests,
) -> String {
    let connection::Command::Subscribe { key, tag, .. } = requests
        .try_recv()
        .expect("stop observation before dispatch")
    else {
        panic!("expected checkout stop subscription");
    };
    assert_eq!(tag, "orchestration.subscribeThread");
    assert!(key.starts_with("checkout-stop-"));
    cx.update(|_, cx| app.update(cx, |app, _| {
        let session = app.detail.thread.as_ref().and_then(|t| t.get("session")).cloned()
            .unwrap_or_else(|| app.shell.threads.iter().find(|t| t.id == "t")
                .and_then(|t| t.session.as_ref()).map_or(Value::Null, |s| json!({"status":s.status})));
        let latest_turn = app.detail.thread.as_ref().map(|t| t["latestTurn"].clone()).unwrap_or(Value::Null);
        app.apply_checkout_stop_subscription(&key, Ok(json!({"kind":"snapshot","supportsCorrelatedSessionStops":true,"snapshot":{"snapshotSequence":5,"thread":{"id":"t","messages":[],"activities":[],"session":session,"latestTurn":latest_turn}}})));
        app.apply_checkout_stop_subscription(&key, Ok(json!({"kind":"synchronized"})));
    }));
    cx.run_until_parked();
    key
}

pub(super) fn complete_stop(
    app: &Entity<NativeApp>,
    cx: &mut VisualTestContext,
    key: &str,
    command_id: &Value,
) {
    cx.update(|_, cx| app.update(cx, |app, _| app.apply_checkout_stop_subscription(key, Ok(json!({"kind":"event","event":{"sequence":13,"type":"thread.session-set","occurredAt":"now","payload":{"threadId":"t","session":{"status":"stopped","activeTurnId":null,"stopCommandId":command_id}}}})))));
    cx.run_until_parked();
}

pub(super) fn finish_owned_stop(
    app: &Entity<NativeApp>,
    cx: &mut VisualTestContext,
    requests: &Requests,
) {
    let key = stop_observation(app, cx, requests);
    let (payload, receipt) = call(requests, "orchestration.dispatchCommand");
    assert_eq!(payload["type"], "thread.session.stop");
    receipt.send_blocking(Ok(json!({"sequence":10}))).unwrap();
    cx.run_until_parked();
    assert!(
        requests.try_recv().is_err(),
        "acceptance cannot authorize Git"
    );
    complete_stop(app, cx, &key, &payload["commandId"]);
}

pub(super) fn refresh_refs(
    cx: &mut VisualTestContext,
    requests: &Requests,
) {
    let (payload, reply) = call(requests, "vcs.listRefs");
    assert_eq!(payload["refresh"], true);
    reply
        .send_blocking(Ok(
            json!({"isRepo":true,"refs":[{"name":"actual","current":true}],"nextCursor":null}),
        ))
        .unwrap();
    cx.run_until_parked();
}

fn enter(app: &Entity<NativeApp>, cx: &mut VisualTestContext, page: BranchWorkflow, value: &str) {
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.branch_picker = Some(if app.selected_thread_branch().is_some() {
                "branch-selector"
            } else {
                "checkout-selector"
            });
            app.show_branch_workflow(page, window, cx);
            app.branch_workflow_input
                .update(cx, |input, cx| input.set_value(value, window, cx));
            app.composer.update(cx, |input, cx| {
                input.set_value("keep my message", window, cx)
            });
        })
    });
    cx.run_until_parked();
}
fn run(app: &Entity<NativeApp>, cx: &mut VisualTestContext, worktree: bool) {
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.shell.synchronized = true;
            app.run_branch_workflow(worktree, window, cx);
        })
    });
    cx.run_until_parked();
}

#[gpui_kit::test]
fn accepted_queued_start_without_session_waits_for_own_stop_marker(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    enter(&app, cx, BranchWorkflow::Create, "queued-checkout");
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.shell.threads[0].session = Some(shell::Session {
                status: "stopped".into(),
                last_error: None,
            });
            let id = app.queues.push(
                "t",
                queue::QueuedMessage {
                    id: 0,
                    text: "queued prompt".into(),
                    attachments: vec![],
                    model_selection: json!({"provider":"codex","model":"gpt-5.3-codex"}),
                    runtime_mode: "full-access".into(),
                    interaction_mode: "default".into(),
                },
                false,
            );
            app.send_queued("t", id, cx);
        })
    });
    cx.run_until_parked();
    let (start, receipt) = call(&requests, "orchestration.dispatchCommand");
    assert_eq!(start["type"], "thread.turn.start");
    run(&app, cx, false);
    assert!(
        requests.try_recv().is_err(),
        "in-flight start fences checkout before subscription"
    );
    app.read_with(cx, |app, cx| {
        assert!(app.checkout_operations.is_empty());
        assert!(app.queues.dispatching("t"));
        assert_eq!(app.queues.messages("t")[0].text, "queued prompt");
        assert_eq!(app.composer.read(cx).value().as_str(), "keep my message");
    });
    receipt.send_blocking(Ok(json!({"sequence":4}))).unwrap();
    cx.run_until_parked();
    cx.update(|_, cx| {
        app.update(cx, |app, _| {
            app.shell.threads[0].session = None;
            app.detail.thread =
                Some(json!({"id":"t","session":null,"latestTurn":{"status":"queued"}}).into());
        })
    });
    run(&app, cx, false);
    let key = stop_observation(&app, cx, &requests);
    let (stop, receipt) = call(&requests, "orchestration.dispatchCommand");
    assert_eq!(stop["type"], "thread.session.stop");
    receipt.send_blocking(Ok(json!({"sequence":10}))).unwrap();
    cx.run_until_parked();
    assert!(requests.try_recv().is_err());
    for (index, marker) in [Value::Null, json!("older-stop-command")]
        .into_iter()
        .enumerate()
    {
        cx.update(|_, cx| app.update(cx, |app, _| {
            app.apply_checkout_stop_subscription(&key, Ok(json!({"kind":"event","event":{"sequence":11 + index,"type":"thread.session-set","occurredAt":"now","payload":{"threadId":"t","session":{"status":"stopped","stopCommandId":marker}}}})));
        }));
        cx.run_until_parked();
        assert!(requests.try_recv().is_err(), "no Git without our marker");
    }
    complete_stop(&app, cx, &key, &stop["commandId"]);
    let (_, create) = call(&requests, "vcs.createRef");
    create.send_blocking(Err("not transmitted".into())).unwrap();
    cx.run_until_parked();
    assert!(requests.try_recv().is_err());
}

#[gpui_kit::test]
fn pending_action_receipt_fences_picker_without_losing_prompt(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    enter(&app, cx, BranchWorkflow::Create, "retained");
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.pending_actions = 1;
            app.pick_checkout("feature".into(), None, window, cx);
            assert!(app.checkout_operations.is_empty());
            assert_eq!(app.composer.read(cx).value().as_str(), "keep my message");
            assert!(app.status.contains("pending server receipt"));
        })
    });
    cx.run_until_parked();
    assert!(requests.try_recv().is_err());
}

#[gpui_kit::test]
fn other_active_root_session_blocks_shared_mutation_but_not_separate_pr_worktree(
    cx: &mut TestAppContext,
) {
    let (app, cx, requests) = harness(cx);
    cx.update(|_, cx| {
        app.update(cx, |app, _| {
            let mut other = app.shell.threads[0].clone();
            other.id = "other".into();
            other.session = Some(shell::Session {
                status: "ready".into(),
                last_error: None,
            });
            app.shell.threads.push(other);
        })
    });
    for page in [BranchWorkflow::Create, BranchWorkflow::PullRequest] {
        enter(&app, cx, page, "42");
        run(&app, cx, false);
        assert!(requests.try_recv().is_err());
        assert!(app.read_with(cx, |app, _| {
            app.branch_workflow_error
                .as_deref()
                .is_some_and(|e| e.contains("Stop it first or use a separate worktree"))
        }));
    }
    // Preparing a distinct worktree does not switch the shared project root.
    run(&app, cx, true);
    let (_, resolve) = call(&requests, "git.resolvePullRequest");
    resolve
        .send_blocking(Ok(json!({"pullRequest":{"number":42}})))
        .unwrap();
    cx.run_until_parked();
    finish_owned_stop(&app, cx, &requests);
    let (payload, prepare) = call(&requests, "git.preparePullRequestThread");
    assert_eq!(payload["mode"], "worktree");
    prepare
        .send_blocking(Err("not transmitted".into()))
        .unwrap();
    cx.run_until_parked();
    assert!(requests.try_recv().is_err());
}

#[gpui_kit::test]
fn other_root_in_flight_send_fences_mutation_until_its_receipt(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    enter(&app, cx, BranchWorkflow::Create, "after-other-send");
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            let mut other = app.shell.threads[0].clone();
            other.id = "other".into();
            app.shell.threads.push(other);
            let id = app.queues.push(
                "other",
                queue::QueuedMessage {
                    id: 0,
                    text: "other root prompt".into(),
                    attachments: vec![],
                    model_selection: json!({"instanceId":"codex","model":"gpt-5.4"}),
                    runtime_mode: "full-access".into(),
                    interaction_mode: "default".into(),
                },
                false,
            );
            let message = app.queues.take("other", id).unwrap();
            app.dispatch_queued("other".into(), message, cx);
        })
    });
    cx.run_until_parked();
    let (payload, send) = call(&requests, "orchestration.dispatchCommand");
    assert_eq!(payload["threadId"], "other");
    run(&app, cx, false);
    assert!(requests.try_recv().is_err());
    assert!(
        app.read_with(cx, |app, _| app.checkout_operations.is_empty()
            && app.queues.dispatching("other")
            && app.queues.messages("other")[0].text == "other root prompt")
    );
    send.send_blocking(Err("send rejected".into())).unwrap();
    cx.run_until_parked();
    let connection::Command::Subscribe { key, tag, payload } = requests.try_recv().unwrap() else {
        panic!("retained queue has its own detail interest");
    };
    assert!(key.starts_with("queue-detail:"));
    assert_eq!(tag, "orchestration.subscribeThread");
    assert_eq!(payload["threadId"], "other");
    assert!(app.read_with(cx, |app, _| !app.queues.dispatching("other")
        && app.queues.messages("other")[0].text == "other root prompt"));
    run(&app, cx, false);
    finish_owned_stop(&app, cx, &requests);
    let (_, create) = call(&requests, "vcs.createRef");
    create.send_blocking(Err("not transmitted".into())).unwrap();
    cx.run_until_parked();
    assert!(requests.try_recv().is_err());
}

#[gpui_kit::test]
fn root_session_projected_during_pr_resolution_prevents_mutation(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    enter(&app, cx, BranchWorkflow::PullRequest, "42");
    run(&app, cx, false);
    let (_, resolve) = call(&requests, "git.resolvePullRequest");
    cx.update(|_, cx| {
        app.update(cx, |app, _| {
            let mut other = app.shell.threads[0].clone();
            other.id = "other".into();
            other.session = Some(shell::Session {
                status: "ready".into(),
                last_error: None,
            });
            app.shell.threads.push(other);
        })
    });
    resolve
        .send_blocking(Ok(json!({"pullRequest":{"number":42}})))
        .unwrap();
    cx.run_until_parked();
    assert!(
        requests.try_recv().is_err(),
        "no stop or Git after shared root becomes active"
    );
    assert!(app.read_with(cx, |app, _| {
        app.checkout_operations.is_empty()
            && app
                .branch_workflow_error
                .as_deref()
                .is_some_and(|e| e.contains("use a separate worktree"))
    }));
}

#[gpui_kit::test]
fn branchless_threads_and_drafts_can_reach_checkout_workflows(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    for draft in [false, true] {
        cx.update(|window, cx| {
            app.update(cx, |app, cx| {
                if draft {
                    app.new_thread(window, cx);
                }
                app.shell.threads[0].branch = None;
                if let Some(detail) = &mut app.detail.thread {
                    std::sync::Arc::make_mut(detail)["branch"] = Value::Null;
                }
                app.composer.update(cx, |input, cx| {
                    input.set_value("keep this prompt", window, cx)
                });
                cx.notify();
            })
        });
        cx.run_until_parked();
        while requests.try_recv().is_ok() {}
        assert!(
            cx.debug_bounds("branch-selector").is_none(),
            "no invented branch label"
        );
        let checkout = cx
            .debug_bounds("checkout-selector")
            .expect("branchless checkout entry point");
        cx.simulate_click(checkout.center(), Default::default());
        cx.run_until_parked();
        let (payload, reply) = call(&requests, "vcs.listRefs");
        assert_eq!(payload["cwd"], "/tmp/fixture");
        reply.send_blocking(Ok(json!({"isRepo":true,"nextCursor":null,"refs":[{"name":"main","current":true,"isDefault":true}]}))).unwrap();
        cx.run_until_parked();
        let new_branch = cx
            .debug_bounds("branch-picker-new")
            .expect("product workflow row");
        cx.simulate_click(new_branch.center(), Default::default());
        cx.run_until_parked();
        assert!(app.read_with(cx, |app, _| app.branch_workflow == BranchWorkflow::Create));
        assert!(cx.debug_bounds("branch-workflow-create").is_some());
        cx.simulate_keystrokes("escape");
        cx.run_until_parked();
        assert!(app.read_with(cx, |app, _| app.branch_picker.is_none()));
        assert_eq!(
            app.read_with(cx, |app, cx| app.composer.read(cx).value().to_string()),
            "keep this prompt"
        );
        assert!(
            requests.try_recv().is_err(),
            "viewing/canceling must not mutate a checkout"
        );
    }
}

#[gpui_kit::test]
fn invalid_selected_thread_never_falls_back_to_mutating_the_project_root(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    enter(&app, cx, BranchWorkflow::Create, "must-not-create");
    cx.update(|_, cx| app.update(cx, |app, _| app.selected_thread = Some("missing".into())));
    run(&app, cx, false);
    assert!(requests.try_recv().is_err());
    assert!(app.read_with(cx, |app, _| {
        app.branch_workflow_error
            .as_deref()
            .is_some_and(|error| error.contains("unavailable"))
    }));
}

#[gpui_kit::test]
fn new_branch_row_creates_once_then_applies_normalized_metadata(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    let reply = open_branch_picker(cx, &requests, "checkout-selector");
    reply
        .send_blocking(Ok(
            json!({"refs":[{"name":"main","current":true}],"isRepo":true,"nextCursor":null}),
        ))
        .unwrap();
    cx.run_until_parked();
    let row = cx.debug_bounds("branch-picker-new").unwrap();
    cx.simulate_click(row.center(), Default::default());
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.shell.synchronized = true;
            app.branch_workflow_input
                .update(cx, |input, cx| input.set_value("new branch", window, cx));
            app.composer.update(cx, |input, cx| {
                input.set_value("keep my message", window, cx)
            });
        })
    });
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    finish_owned_stop(&app, cx, &requests);
    let (payload, receipt) = call(&requests, "vcs.createRef");
    assert_eq!(
        payload,
        json!({"cwd":"/tmp/fixture","refName":"new-branch","switchRef":true})
    );
    assert!(requests.try_recv().is_err());
    receipt
        .send_blocking(Ok(json!({"refName":"normalized"})))
        .unwrap();
    cx.run_until_parked();
    let (payload, receipt) = call(&requests, "orchestration.dispatchCommand");
    assert_eq!(payload["type"], "thread.meta.update");
    assert_eq!(payload["branch"], "normalized");
    assert_eq!(payload.get("worktreePath"), Some(&Value::Null));
    assert!(app.read_with(cx, |app, _| app.checkout_transition_pending()));
    cx.update(|window, cx| app.update(cx, |app, cx| app.send_prompt(window, cx)));
    cx.run_until_parked();
    assert!(
        requests.try_recv().is_err(),
        "metadata adoption fences send"
    );
    assert_eq!(
        app.read_with(cx, |app, cx| app.composer.read(cx).value().to_string()),
        "keep my message"
    );
    receipt.send_blocking(Ok(json!({}))).unwrap();
    cx.run_until_parked();
    refresh_refs(cx, &requests);
    app.read_with(cx, |app, cx| {
        assert_eq!(app.shell.threads[0].branch.as_deref(), Some("normalized"));
        assert_eq!(app.composer.read(cx).value().as_str(), "keep my message");
        assert!(!app.branch_workflow_busy && !app.worktree_busy);
        assert!(app.branch_picker.is_none());
    });
    assert!(requests.try_recv().is_err());
}

#[gpui_kit::test]
fn pr_resolution_stop_prepare_metadata_are_receipt_ordered(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    enter(&app, cx, BranchWorkflow::PullRequest, "#42");
    cx.update(|_, cx| {
        app.update(cx, |app, _| {
            app.detail.thread = Some(json!({"id":"t","session":{"status":"ready"}}).into())
        })
    });
    run(&app, cx, true);
    let (payload, receipt) = call(&requests, "git.resolvePullRequest");
    assert_eq!(payload, json!({"cwd":"/tmp/fixture","reference":"42"}));
    assert!(requests.try_recv().is_err());
    receipt
        .send_blocking(Ok(json!({"pullRequest":{"number":42}})))
        .unwrap();
    cx.run_until_parked();
    let key = stop_observation(&app, cx, &requests);
    let (payload, receipt) = call(&requests, "orchestration.dispatchCommand");
    assert_eq!(payload["type"], "thread.session.stop");
    assert!(requests.try_recv().is_err());
    receipt.send_blocking(Ok(json!({"sequence":10}))).unwrap();
    cx.run_until_parked();
    assert!(
        requests.try_recv().is_err(),
        "acceptance is not provider shutdown"
    );
    complete_stop(&app, cx, &key, &payload["commandId"]);
    let (payload, receipt) = call(&requests, "git.preparePullRequestThread");
    assert_eq!(
        payload,
        json!({"cwd":"/tmp/fixture","reference":"42","mode":"worktree","threadId":"t"})
    );
    receipt.send_blocking(Ok(json!({"branch":"pr-normalized","worktreePath":"/tmp/pr-tree","isOnPullRequestHead":false}))).unwrap();
    cx.run_until_parked();
    let (payload, receipt) = call(&requests, "orchestration.dispatchCommand");
    assert_eq!(payload["type"], "thread.meta.update");
    assert_eq!(payload["branch"], "pr-normalized");
    assert_eq!(payload["worktreePath"], "/tmp/pr-tree");
    receipt.send_blocking(Ok(json!({}))).unwrap();
    cx.run_until_parked();
    assert!(app.read_with(cx, |app, _| app.status.contains("not on PR head")));
    refresh_refs(cx, &requests);
    app.read_with(cx, |app, cx| {
        assert_eq!(app.selected_workspace().as_deref(), Some("/tmp/pr-tree"));
        assert_eq!(app.composer.read(cx).value().as_str(), "keep my message");
    });
    assert!(requests.try_recv().is_err());
}

#[gpui_kit::test]
fn pre_send_rejection_retry_and_back_preserve_selection_and_draft(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    enter(&app, cx, BranchWorkflow::Create, "bad..name");
    run(&app, cx, false);
    assert!(requests.try_recv().is_err());
    assert!(app.read_with(cx, |app, _| app.branch_workflow_error.is_some()));
    enter(&app, cx, BranchWorkflow::Create, "valid");
    run(&app, cx, false);
    finish_owned_stop(&app, cx, &requests);
    let (_, receipt) = call(&requests, "vcs.createRef");
    receipt
        .send_blocking(Err("Disconnected before send".into()))
        .unwrap();
    cx.run_until_parked();
    assert!(app.read_with(cx, |app, _| !app.worktree_busy
        && app.branch_workflow_error.as_deref() == Some("Disconnected before send")));
    run(&app, cx, false);
    finish_owned_stop(&app, cx, &requests);
    let (_, old) = call(&requests, "vcs.createRef");
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.show_branch_workflow(BranchWorkflow::Browse, window, cx)
        })
    });
    let _ = old.send_blocking(Ok(json!({"refName":"late"})));
    cx.run_until_parked();
    refresh_refs(cx, &requests);
    app.read_with(cx, |app, cx| {
        assert!(app.checkout_recovery().is_some());
        assert_eq!(
            app.checkout_recovery()
                .unwrap()
                .destination
                .as_ref()
                .unwrap()
                .branch
                .as_deref(),
            Some("late")
        );
        assert_eq!(app.selected_thread.as_deref(), Some("t"));
        assert_ne!(app.shell.threads[0].branch.as_deref(), Some("late"));
        assert_eq!(app.composer.read(cx).value().as_str(), "keep my message");
        assert!(!app.branch_workflow_busy && !app.worktree_busy);
    });
}

#[gpui_kit::test]
fn escape_during_pr_resolution_cannot_prepare_or_mutate(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    enter(&app, cx, BranchWorkflow::PullRequest, "42");
    run(&app, cx, false);
    let (_, old) = call(&requests, "git.resolvePullRequest");
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    let _ = old.send_blocking(Ok(json!({"pullRequest":{"number":42}})));
    cx.run_until_parked();
    assert!(requests.try_recv().is_err());
    app.read_with(cx, |app, cx| {
        assert!(app.branch_picker.is_none());
        assert!(!app.branch_workflow_busy && !app.worktree_busy);
        assert_eq!(app.composer.read(cx).value().as_str(), "keep my message");
    });
}

#[gpui_kit::test]
fn environment_mode_is_local_draft_state_not_a_materialized_thread_rpc(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    enter(&app, cx, BranchWorkflow::Environment, "main");
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.select_branch_environment(true, window, cx)
        })
    });
    assert!(requests.try_recv().is_err());
    assert!(app.read_with(cx, |app, _| {
        app.branch_workflow_error
            .as_deref()
            .is_some_and(|e| e.contains("unsent drafts"))
    }));
    cx.update(|window, cx| app.update(cx, |app, cx| app.new_thread(window, cx)));
    // Selection subscription glue is not a mutation RPC.
    assert!(
        matches!(requests.try_recv().unwrap(), connection::Command::Subscribe { tag, .. } if tag == "subscribeWorktreeSetup")
    );
    assert!(matches!(
        requests.try_recv().unwrap(),
        connection::Command::SelectThread(None)
    ));
    assert!(requests.try_recv().is_err());
    enter(&app, cx, BranchWorkflow::Environment, "main");
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.select_branch_environment(true, window, cx)
        })
    });
    assert!(requests.try_recv().is_err());
    app.read_with(cx, |app, cx| {
        let draft = app
            .pending_threads
            .iter()
            .find(|d| Some(&d.thread.id) == app.selected_thread.as_ref())
            .unwrap();
        assert_eq!(
            draft.prepare_worktree,
            Some(json!({"projectCwd":"/tmp/fixture","baseBranch":"main","startFromOrigin":false}))
        );
        assert!(draft.thread.worktree_path.is_none());
        assert_eq!(app.composer.read(cx).value().as_str(), "keep my message");
    });
    enter(&app, cx, BranchWorkflow::Create, "not-a-base");
    run(&app, cx, false);
    assert!(requests.try_recv().is_err());
    assert!(app.read_with(cx, |app, _| {
        app.branch_workflow_error
            .as_deref()
            .is_some_and(|e| e.contains("base branch"))
    }));
    enter(&app, cx, BranchWorkflow::Environment, "main");
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.select_branch_environment(false, window, cx)
        })
    });
    app.read_with(cx, |app, cx| {
        let draft = app
            .pending_threads
            .iter()
            .find(|d| Some(&d.thread.id) == app.selected_thread.as_ref())
            .unwrap();
        assert!(draft.prepare_worktree.is_none());
        assert!(
            draft.thread.branch.is_none(),
            "Local cannot advertise arbitrary base text as a physical root branch"
        );
        assert!(!app.prepare_new_worktree);
        assert!(app.worktree_ref.read(cx).value().is_empty());
        assert!(draft.thread.worktree_path.is_none());
        assert_eq!(app.composer.read(cx).value().as_str(), "keep my message");
    });
    assert!(requests.try_recv().is_err());
}

#[gpui_kit::test]
fn dirty_file_and_archived_selection_reject_branch_workflows(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    enter(&app, cx, BranchWorkflow::Create, "valid");
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.file_editor.loaded(
                "/tmp/fixture".into(),
                serde_json::from_value(
                    json!({"relativePath":"owned.txt","contents":"saved","truncated":false}),
                )
                .unwrap(),
            );
            app.file_contents
                .update(cx, |input, cx| input.set_value("unsaved", window, cx));
        })
    });
    run(&app, cx, false);
    assert!(requests.try_recv().is_err());
    app.read_with(cx, |app, cx| {
        assert_eq!(app.file_contents.read(cx).value().as_str(), "unsaved");
        assert!(
            app.branch_workflow_error
                .as_deref()
                .is_some_and(|e| e.contains("Save or Discard"))
        );
    });
    cx.update(|_, cx| {
        app.update(cx, |app, _| {
            app.archived_shell.threads.push(app.shell.threads.remove(0));
        })
    });
    enter(&app, cx, BranchWorkflow::PullRequest, "42");
    run(&app, cx, true);
    assert!(requests.try_recv().is_err());
    app.read_with(cx, |app, cx| {
        assert!(
            app.branch_workflow_error
                .as_deref()
                .is_some_and(|e| e.contains("Restore"))
        );
        assert_eq!(app.selected_thread.as_deref(), Some("t"));
        assert_eq!(app.composer.read(cx).value().as_str(), "keep my message");
    });
}

#[gpui_kit::test]
fn accepted_create_failed_metadata_retries_only_metadata_through_visible_recovery(
    cx: &mut TestAppContext,
) {
    let (app, cx, requests) = harness(cx);
    enter(&app, cx, BranchWorkflow::Create, "feature");
    run(&app, cx, false);
    finish_owned_stop(&app, cx, &requests);
    let (_, create) = call(&requests, "vcs.createRef");
    create
        .send_blocking(Ok(json!({"refName":"actual-feature"})))
        .unwrap();
    cx.run_until_parked();
    let (_, metadata) = call(&requests, "orchestration.dispatchCommand");
    metadata
        .send_blocking(Err("metadata rejected".into()))
        .unwrap();
    cx.run_until_parked();
    refresh_refs(cx, &requests);
    assert!(cx.debug_bounds("checkout-recovery-status").is_some());
    assert!(app.read_with(cx, |app, _| app.checkout_transition_pending()));
    let retry = cx.debug_bounds("checkout-recovery-retry").unwrap();
    cx.simulate_click(retry.center(), Default::default());
    cx.run_until_parked();
    let (payload, metadata) = call(&requests, "orchestration.dispatchCommand");
    assert_eq!(payload["type"], "thread.meta.update");
    assert_eq!(payload["branch"], "actual-feature");
    assert!(requests.try_recv().is_err(), "no duplicate physical create");
    metadata.send_blocking(Ok(json!({"sequence":20}))).unwrap();
    cx.run_until_parked();
    refresh_refs(cx, &requests);
    app.read_with(cx, |app, cx| {
        assert!(!app.checkout_transition_pending());
        assert_eq!(
            app.shell.threads[0].branch.as_deref(),
            Some("actual-feature")
        );
        assert_eq!(app.composer.read(cx).value().as_str(), "keep my message");
    });
}

#[gpui_kit::test]
fn delayed_pr_preparation_fences_first_send_and_preserves_dirty_file_target(
    cx: &mut TestAppContext,
) {
    let (app, cx, requests) = harness(cx);
    cx.update(|window, cx| app.update(cx, |app, cx| app.new_thread(window, cx)));
    while requests.try_recv().is_ok() {}
    enter(&app, cx, BranchWorkflow::PullRequest, "42");
    run(&app, cx, true);
    let (_, resolve) = call(&requests, "git.resolvePullRequest");
    resolve
        .send_blocking(Ok(json!({"pullRequest":{"number":42}})))
        .unwrap();
    cx.run_until_parked();
    let (_, prepare) = call(&requests, "git.preparePullRequestThread");
    cx.update(|window, cx| app.update(cx, |app, cx| app.send_prompt(window, cx)));
    cx.run_until_parked();
    assert!(
        requests.try_recv().is_err(),
        "first send cannot materialize the old draft checkout"
    );
    assert_eq!(
        app.read_with(cx, |app, cx| app.composer.read(cx).value().to_string()),
        "keep my message"
    );
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.file_editor.loaded(
                "/tmp/fixture".into(),
                serde_json::from_value(
                    json!({"relativePath":"owned.txt","contents":"saved","truncated":false}),
                )
                .unwrap(),
            );
            app.file_environment = app.environment_scope.clone();
            app.file_contents
                .update(cx, |input, cx| input.set_value("dirty text", window, cx));
        })
    });
    prepare
        .send_blocking(Ok(json!({"branch":"pr-42","worktreePath":"/tmp/pr-42"})))
        .unwrap();
    cx.run_until_parked();
    refresh_refs(cx, &requests);
    app.read_with(cx, |app, cx| {
        assert_eq!(
            app.checkout_recovery()
                .unwrap()
                .destination
                .as_ref()
                .unwrap()
                .path
                .as_deref(),
            Some("/tmp/pr-42")
        );
        assert_eq!(app.file_contents.read(cx).value().as_str(), "dirty text");
        assert!(app.checkout_transition_pending());
        assert_eq!(app.selected_workspace().as_deref(), Some("/tmp/fixture"));
    });
    cx.update(|window, cx| app.update(cx, |app, cx| app.file_operation(true, window, cx)));
    cx.run_until_parked();
    let (write, saved) = call(&requests, "projects.writeFile");
    assert_eq!(write["cwd"], "/tmp/fixture");
    assert_eq!(write["relativePath"], "owned.txt");
    assert_eq!(write["contents"], "dirty text");
    saved
        .send_blocking(Ok(
            json!({"relativePath":"owned.txt","contents":"dirty text","truncated":false}),
        ))
        .unwrap();
    cx.run_until_parked();
    cx.update(|window, cx| app.update(cx, |app, cx| app.retry_checkout_metadata(window, cx)));
    cx.run_until_parked();
    refresh_refs(cx, &requests);
    assert!(
        requests.try_recv().is_err(),
        "draft adoption needs no duplicate prepare nor metadata RPC"
    );
    assert_eq!(
        app.read_with(cx, |app, _| app.selected_workspace()),
        Some("/tmp/pr-42".into())
    );
}

#[gpui_kit::test]
fn transmitted_create_result_survives_a_to_b_and_environment_return(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    enter(&app, cx, BranchWorkflow::Create, "feature");
    run(&app, cx, false);
    finish_owned_stop(&app, cx, &requests);
    let (_, create) = call(&requests, "vcs.createRef");
    let original_scope = app.read_with(cx, |app, _| app.environment_scope.clone());
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.cancel_branch_workflow(cx);
            app.environment_scope = "another-environment".into();
            app.generation += 1;
            app.selected_thread = Some("b".into());
            let mut b = app.shell.threads[0].clone();
            b.id = "b".into();
            b.branch = Some("b-branch".into());
            app.shell.threads.push(b);
        })
    });
    create
        .send_blocking(Ok(json!({"refName":"a-feature"})))
        .unwrap();
    cx.run_until_parked();
    assert!(requests.try_recv().is_err());
    app.read_with(cx, |app, _| {
        assert!(
            !app.checkout_transition_pending(),
            "fence is environment-scoped"
        );
        assert_eq!(app.shell.threads[1].branch.as_deref(), Some("b-branch"));
        assert_eq!(
            app.checkout_operations[0]
                .destination
                .as_ref()
                .unwrap()
                .branch
                .as_deref(),
            Some("a-feature")
        );
    });
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.environment_scope = original_scope;
            app.selected_thread = Some("t".into());
            app.retry_checkout_metadata(window, cx);
        })
    });
    cx.run_until_parked();
    let (payload, metadata) = call(&requests, "orchestration.dispatchCommand");
    assert_eq!(payload["threadId"], "t");
    metadata.send_blocking(Ok(json!({"sequence":22}))).unwrap();
    cx.run_until_parked();
    refresh_refs(cx, &requests);
    assert_eq!(
        app.read_with(cx, |app, _| app.shell.threads[1].branch.clone()),
        Some("b-branch".into())
    );
}

#[gpui_kit::test]
fn saved_environment_mode_base_and_inspector_update_stay_in_sync(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    cx.update(|window, cx| app.update(cx, |app, cx| app.new_thread(window, cx)));
    while requests.try_recv().is_ok() {}
    enter(&app, cx, BranchWorkflow::Environment, "main");
    let worktree = cx.debug_bounds("branch-workflow-worktree").unwrap();
    cx.simulate_click(worktree.center(), Default::default());
    cx.run_until_parked();
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.branch_picker = Some("checkout-selector");
            app.show_branch_workflow(BranchWorkflow::Environment, window, cx);
        })
    });
    cx.run_until_parked();
    assert!(cx.debug_bounds("draft-environment-mode").is_some());
    app.read_with(cx, |app, cx| {
        assert!(app.prepare_new_worktree);
        assert_eq!(app.branch_workflow_input.read(cx).value().as_str(), "main");
        assert_eq!(app.worktree_ref.read(cx).value().as_str(), "main");
    });
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.pick_checkout("develop".into(), None, window, cx);
            app.update_draft_checkout(cx);
        })
    });
    assert!(
        requests.try_recv().is_err(),
        "planning a base remains declarative"
    );
    assert_eq!(
        app.read_with(cx, |app, _| app
            .pending_threads
            .last()
            .unwrap()
            .prepare_worktree
            .as_ref()
            .unwrap()["baseBranch"]
            .clone()),
        json!("develop")
    );
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.branch_picker = Some("checkout-selector");
            app.show_branch_workflow(BranchWorkflow::Environment, window, cx);
            app.branch_workflow_input.update(cx, |input, cx| {
                input.set_value("invented-branch", window, cx)
            });
        })
    });
    cx.run_until_parked();
    let local = cx.debug_bounds("branch-workflow-local").unwrap();
    cx.simulate_click(local.center(), Default::default());
    cx.run_until_parked();
    app.read_with(cx, |app, _| {
        assert!(!app.prepare_new_worktree && !app.start_from_origin);
        assert!(
            app.pending_threads
                .last()
                .unwrap()
                .prepare_worktree
                .is_none()
        );
        assert!(app.pending_threads.last().unwrap().thread.branch.is_none());
    });
    cx.update(|window, cx| app.update(cx, |app, cx| app.new_thread(window, cx)));
    assert!(app.read_with(cx, |app, _| {
        app.pending_threads
            .last()
            .unwrap()
            .prepare_worktree
            .is_none()
    }));
}

#[gpui_kit::test]
fn empty_materialized_root_uses_public_prepare_bootstrap_without_recreate(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    cx.update(|_, cx| app.update(cx, |app, _| {
        app.detail.synchronized = true;
        app.detail.thread = Some(json!({"id":"t","messages":[],"activities":[],"session":null,"worktreePath":null,"latestTurn":null}).into());
    }));
    enter(&app, cx, BranchWorkflow::Environment, "main");
    let worktree = cx.debug_bounds("branch-workflow-worktree").unwrap();
    cx.simulate_click(worktree.center(), Default::default());
    cx.run_until_parked();
    assert!(
        requests.try_recv().is_err(),
        "mode override is declarative until first send"
    );
    app.read_with(cx, |app, _| {
        let bootstrap = app.materialized_checkout_bootstrap("t").unwrap().unwrap();
        assert!(bootstrap.get("createThread").is_none());
        assert_eq!(bootstrap["prepareWorktree"]["baseBranch"], "main");
    });
    cx.update(|window, cx| app.update(cx, |app, cx| app.send_prompt(window, cx)));
    cx.run_until_parked();
    let (payload, _) = call(&requests, "orchestration.dispatchCommand");
    assert_eq!(
        payload["bootstrap"]["prepareWorktree"]["baseBranch"],
        "main"
    );
    assert!(payload["bootstrap"].get("createThread").is_none());
}

#[gpui_kit::test]
fn unsent_local_ref_runs_real_switch_before_bootstrap_advertises_branch(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    cx.update(|window, cx| app.update(cx, |app, cx| app.new_thread(window, cx)));
    while requests.try_recv().is_ok() {}
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.composer
                .update(cx, |input, cx| input.set_value("keep prompt", window, cx));
            app.pick_checkout("origin/feature".into(), None, window, cx);
        })
    });
    cx.run_until_parked();
    let (payload, switched) = call(&requests, "vcs.switchRef");
    assert_eq!(payload["cwd"], "/tmp/fixture");
    assert_eq!(payload["refName"], "origin/feature");
    assert!(app.read_with(cx, |app, _| {
        app.pending_threads.last().unwrap().thread.branch.is_none()
    }));
    switched
        .send_blocking(Ok(json!({"refName":"feature"})))
        .unwrap();
    cx.run_until_parked();
    refresh_refs(cx, &requests);
    cx.update(|window, cx| app.update(cx, |app, cx| app.send_prompt(window, cx)));
    cx.run_until_parked();
    let (sent, _) = call(&requests, "orchestration.dispatchCommand");
    assert_eq!(sent["bootstrap"]["createThread"]["branch"], "feature");
    assert!(sent["bootstrap"]["createThread"]["worktreePath"].is_null());
}

#[gpui_kit::test]
fn unsent_gate_cancellation_releases_operation_without_metadata(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    enter(&app, cx, BranchWorkflow::Create, "never-transmit");
    run(&app, cx, false);
    finish_owned_stop(&app, cx, &requests);
    assert!(matches!(
        requests.try_recv().unwrap(),
        connection::Command::Unsubscribe(_)
    ));
    let connection::Command::CallMutation {
        tag, gate, reply, ..
    } = requests.try_recv().unwrap()
    else {
        panic!("gate-tracked create");
    };
    assert_eq!(tag, "vcs.createRef");
    assert!(!gate.may_have_been_sent());
    cx.update(|_, cx| app.update(cx, |app, cx| app.cancel_branch_workflow(cx)));
    // Simulate the transport's canceled-before-send receipt; transport owner
    // tests atomically prove this gate never allocates an id or wire request.
    reply
        .send_blocking(Err("Mutation canceled before send".into()))
        .unwrap();
    cx.run_until_parked();
    assert!(requests.try_recv().is_err());
    assert!(!app.read_with(cx, |app, _| app.checkout_transition_pending()));
}
