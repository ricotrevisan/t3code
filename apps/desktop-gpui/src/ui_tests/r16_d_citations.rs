use super::*;

pub(super) struct Source {
    pub(super) requests: Requests,
    pub(super) updates: async_channel::Sender<connection::Update>,
}
pub(super) fn connect(app: Entity<NativeApp>, id: &str, cx: &mut VisualTestContext) -> Source {
    let (commands, requests) = async_channel::bounded(64);
    let (updates, receiver) = async_channel::bounded(64);
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.disconnect(cx);
            app.start_owned_session(
                connection::Session {
                    commands,
                    updates: receiver,
                },
                cx,
            );
        })
    });
    cx.run_until_parked();
    let requests = Requests::from(requests);
    let connection::Command::Call {
        tag,
        payload,
        reply,
    } = requests.try_recv().unwrap()
    else {
        panic!("identity call")
    };
    assert_eq!(tag, "server.getConfig");
    assert_eq!(payload, json!({}));
    let config =
        json!({"environment":{"environmentId":id,"label":id},"providers":[],"settings":{}});
    reply.try_send(Ok(config.clone())).unwrap();
    cx.run_until_parked();
    updates.try_send(connection::Update::Shell(Box::new(serde_json::from_value(json!({"kind":"snapshot","snapshot":{"snapshotSequence":0,"projects":[{"id":"p","title":"Repo","workspaceRoot":"/tmp/fixture"}],"threads":[{"id":"t","projectId":"p","title":"Destination","modelSelection":{"instanceId":"codex","model":"gpt-5.4"},"runtimeMode":"full-access","interactionMode":"default","archivedAt":null},{"id":"source","projectId":"p","title":"Source","modelSelection":{"instanceId":"codex","model":"gpt-5.4"},"runtimeMode":"full-access","interactionMode":"default","archivedAt":null}]}})).unwrap()))).unwrap();
    updates
        .try_send(connection::Update::Shell(Box::new(
            shell::ShellItem::Synchronized,
        )))
        .unwrap();
    cx.run_until_parked();
    while let Ok(command) = requests.try_recv() {
        match command {
            connection::Command::Call { tag, reply, .. } => {
                assert_eq!(tag, "server.getConfig");
                reply.try_send(Ok(config.clone())).unwrap();
            }
            connection::Command::Subscribe { tag, .. } => assert_eq!(tag, "subscribeServerConfig"),
            _ => panic!("unexpected setup command"),
        }
    }
    cx.run_until_parked();
    Source { requests, updates }
}
fn quote(environment: &str) -> Value {
    json!({"version":1,"environmentId":environment,"threadId":"source","messageId":"answer","text":"original","start":4,"end":12,"prefix":"the ","suffix":" quote"})
}
fn receipt(source: &Source, messages: Vec<Value>, cx: &mut VisualTestContext) {
    source.updates.try_send(connection::Update::Thread { thread_id:"source".into(), item:json!({"kind":"snapshot","snapshot":{"snapshotSequence":1,"thread":{"id":"source","projectId":"p","messages":messages,"activities":[],"session":{"status":"idle"}}}}) }).unwrap();
    source
        .updates
        .try_send(connection::Update::Thread {
            thread_id: "source".into(),
            item: json!({"kind":"synchronized"}),
        })
        .unwrap();
    cx.run_until_parked();
}
fn assert_selection(source: &Source) {
    assert!(
        matches!(source.requests.try_recv().unwrap(), connection::Command::Subscribe { tag, payload, .. } if tag == "subscribeWorktreeSetup" && payload["threadId"] == "source")
    );
    assert!(
        matches!(source.requests.try_recv().unwrap(), connection::Command::SelectThread(Some(id)) if id == "source")
    );
}
#[gpui_kit::test]
fn r16_d_cross_thread_citation_loads_owned_stream_and_preserves_draft(cx: &mut TestAppContext) {
    let (app, cx, _) = harness(cx);
    let source = connect(app.clone(), "env", cx);
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.select_thread("t", window, cx);
            app.composer.update(cx, |input, cx| {
                input.set_value("destination draft", window, cx)
            });
        })
    });
    cx.run_until_parked();
    while source.requests.try_recv().is_ok() {}
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.navigate_to_citation(&quote("env"), window, cx)
        })
    });
    cx.run_until_parked();
    assert_selection(&source);
    assert!(cx.debug_bounds("citation-source-status").is_some());
    assert!(app.read_with(cx, |app, _| app.drafts["t"] == "destination draft"));
    receipt(
        &source,
        vec![
            json!({"id":"answer","role":"assistant","text":"the original quote","streaming":false}),
        ],
        cx,
    );
    assert!(cx.debug_bounds("message-answer").is_some());
    assert!(app.read_with(
        cx,
        |app, _| app.conversation_presentation.citation_navigation.is_none()
            && app.conversation_presentation.citation_target.is_none()
    ));
    assert!(source.requests.is_empty());
}
#[gpui_kit::test]
fn r16_d_cross_environment_citation_routes_only_verified_owner_and_missing_source_recovers(
    cx: &mut TestAppContext,
) {
    let (app, cx, _) = harness(cx);
    let other = cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.create_environment_page("https://source.example", window, cx)
        })
    });
    let origin = connect(app.clone(), "destination-env", cx);
    let source = connect(other.clone(), "source-env", cx);
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.navigate_to_citation(&quote("source-env"), window, cx)
        })
    });
    cx.run_until_parked();
    assert_selection(&source);
    assert!(origin.requests.is_empty());
    assert!(app.read_with(cx, |app, _| !app.environment_is_selected()));
    receipt(&source, vec![], cx);
    assert!(other.read_with(cx, |app, _| {
        app.conversation_presentation
            .citation_navigation
            .as_ref()
            .unwrap()
            .unavailable
    }));
    let bounds = cx.debug_bounds("citation-navigation-retry").unwrap();
    cx.simulate_click(bounds.center(), gpui_kit::Modifiers::none());
    cx.run_until_parked();
    assert!(
        matches!(source.requests.try_recv().unwrap(), connection::Command::SelectThread(Some(id)) if id == "source")
    );
    assert!(other.read_with(cx, |app, _| !app.detail.synchronized));
    receipt(
        &source,
        vec![
            json!({"id":"answer","role":"assistant","text":"the original quote","streaming":false}),
        ],
        cx,
    );
    assert!(cx.debug_bounds("message-answer").is_some());
    assert!(other.read_with(cx, |app, _| {
        app.conversation_presentation.citation_navigation.is_none()
    }));
    cx.update(|window, cx| {
        other.update(cx, |app, cx| {
            let mut missing = quote("source-env");
            missing["messageId"] = json!("deleted-answer");
            app.navigate_to_citation(&missing, window, cx);
        })
    });
    cx.run_until_parked();
    let bounds = cx.debug_bounds("citation-navigation-cancel").unwrap();
    cx.simulate_click(bounds.center(), gpui_kit::Modifiers::none());
    cx.run_until_parked();
    assert!(
        other.read_with(cx, |app, _| {
            app.conversation_presentation.citation_navigation.is_none()
        }),
        "cancel failed; selected={} status={}",
        other.read_with(cx, |app, _| app.environment_is_selected()),
        other.read_with(cx, |app, _| app.status.clone())
    );
    // Unknown identity cannot select a similarly named environment/thread.
    cx.update(|window, cx| {
        other.update(cx, |app, cx| {
            app.navigate_to_citation(&quote("unknown"), window, cx)
        })
    });
    cx.run_until_parked();
    assert!(source.requests.is_empty() && origin.requests.is_empty());
    assert!(other.read_with(cx, |app, _| {
        app.status.contains("connect its saved environment")
    }));
}

#[gpui_kit::test]
fn r16_d_received_snapshot_reuse_sends_fresh_owned_identity_and_keeps_rejected_draft(
    cx: &mut TestAppContext,
) {
    let (app, cx, requests) = harness(cx);
    cx.update(|_, cx| app.update(cx, |app, cx| {
        app.draft_store = None;
        app.provider_catalog = vec![composer_parity::provider_fixture("codex", "Codex")];
        let mut citation = quote("env");
        citation["threadId"] = json!("missing-original-thread");
        app.apply_thread_update("t", snapshot(vec![json!({"id":"received","role":"user","text":"Explain","context":{"version":1,"records":[{"version":1,"kind":"assistant-citation","contextId":"original","label":"Saved quote","payload":citation}]}})], "idle"), cx).unwrap();
        cx.notify();
    }));
    cx.run_until_parked();
    let bounds = cx.debug_bounds("context-toggle-received-original").unwrap();
    cx.simulate_click(bounds.center(), gpui_kit::Modifiers::none());
    cx.run_until_parked();
    let bounds = cx.debug_bounds("context-reuse-snapshot").unwrap();
    cx.simulate_click(bounds.center(), gpui_kit::Modifiers::none());
    cx.run_until_parked();
    let owned = app.read_with(cx, |app, _| app.draft_context_records()[0].clone());
    assert_ne!(owned.id(), "original");
    assert_eq!(
        owned.value()["payload"]["threadId"],
        "missing-original-thread"
    );
    cx.update(|window, cx| app.update(cx, |app, cx| app.send_prompt(window, cx)));
    cx.run_until_parked();
    let connection::Command::Call {
        tag,
        payload,
        reply,
    } = requests.try_recv().unwrap()
    else {
        panic!("send")
    };
    assert_eq!(tag, "orchestration.dispatchCommand");
    assert_eq!(payload["type"], "thread.turn.start");
    assert_eq!(
        payload["message"]["context"]["records"][0],
        owned.value().clone()
    );
    reply.try_send(Err("denied".into())).unwrap();
    cx.run_until_parked();
    assert_eq!(
        app.read_with(cx, |app, _| app.draft_context_records()[0].clone()),
        owned
    );
}

#[gpui_kit::test]
fn r16_d_plan_action_captures_durable_source_before_mode_receipt_and_edit(cx: &mut TestAppContext) {
    let (app, cx, _) = harness(cx);
    let source = connect(app.clone(), "env", cx);
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.draft_store = None;
            app.provider_catalog = vec![composer_parity::provider_fixture("codex", "Codex")];
            app.select_thread("t", window, cx);
        })
    });
    cx.run_until_parked();
    while source.requests.try_recv().is_ok() {}
    let mut update = snapshot(vec![], "idle");
    update["snapshot"]["thread"]["proposedPlans"] =
        json!([{"id":"chosen-plan","planMarkdown":"# Plan\n\nShip it"}]);
    source
        .updates
        .try_send(connection::Update::Thread {
            thread_id: "t".into(),
            item: update,
        })
        .unwrap();
    source
        .updates
        .try_send(connection::Update::Thread {
            thread_id: "t".into(),
            item: json!({"kind":"synchronized"}),
        })
        .unwrap();
    cx.run_until_parked();
    let bounds = cx.debug_bounds("plan-implement").unwrap();
    cx.simulate_click(bounds.center(), gpui_kit::Modifiers::none());
    cx.run_until_parked();
    let captured = app.read_with(cx, |app, _| app.durable_plan_intent().unwrap());
    assert_eq!(captured.plan, "chosen-plan");
    assert_eq!(captured.source_thread, "t");
    let connection::Command::Call {
        tag,
        payload,
        reply,
    } = source.requests.try_recv().unwrap()
    else {
        panic!("mode")
    };
    assert_eq!(tag, "orchestration.dispatchCommand");
    assert_eq!(payload["type"], "thread.interaction-mode.set");
    assert_eq!(payload["interactionMode"], "default");
    reply.try_send(Err("denied".into())).unwrap();
    cx.run_until_parked();
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.composer.update(cx, |input, cx| {
                input.set_value("Edited implementation instructions", window, cx)
            });
            app.remember_composer_draft(cx);
            app.conversation_presentation.plan_followup = None;
            cx.notify();
        })
    });
    cx.run_until_parked();
    assert!(cx.debug_bounds("plan-draft-recovery").is_none());
    let edited = app.read_with(cx, |app, _| app.durable_plan_intent().unwrap());
    assert_eq!(edited.id, captured.id);
    assert_eq!(edited.plan, captured.plan);
    let bounds = cx.debug_bounds("durable-plan-confirm").unwrap();
    cx.simulate_click(bounds.center(), gpui_kit::Modifiers::none());
    cx.run_until_parked();
    let connection::Command::Call { reply, .. } = source.requests.try_recv().unwrap() else {
        panic!("reconfirm")
    };
    reply.try_send(Ok(json!({}))).unwrap();
    cx.run_until_parked();
    cx.update(|window, cx| app.update(cx, |app, cx| app.send_prompt(window, cx)));
    cx.run_until_parked();
    let connection::Command::Call { payload, reply, .. } = source.requests.try_recv().unwrap()
    else {
        panic!("send")
    };
    assert_eq!(
        payload["sourceProposedPlan"],
        json!({"threadId":"t","planId":"chosen-plan"})
    );
    assert_eq!(
        payload["message"]["text"],
        "Edited implementation instructions"
    );
    reply.try_send(Err("offline".into())).unwrap();
    cx.run_until_parked();
    assert_eq!(
        app.read_with(cx, |app, _| app.durable_plan_intent().unwrap().id),
        captured.id
    );
}

#[gpui_kit::test]
fn r16_int_composer_citation_button_loads_source_and_owned_denial_recovers(cx: &mut TestAppContext) {
    let (app, cx, _) = harness(cx);
    let source = connect(app.clone(), "env", cx);
    let id = cx.update(|window, cx| app.update(cx, |app, cx| {
        app.select_thread("t", window, cx);
        let record = t3code_desktop_gpui::draft_context::DraftContextRecord::citation("Original quote", quote("env")).unwrap();
        app.intake_draft_context(&app.environment_scope.clone(), "t", record, window, cx).unwrap()
    }));
    cx.run_until_parked();
    while source.requests.try_recv().is_ok() {}
    let bounds = cx.debug_bounds(Box::leak(format!("draft-citation-source-{id}").into_boxed_str())).unwrap();
    cx.simulate_click(bounds.center(), Default::default());
    cx.run_until_parked();
    assert_selection(&source);
    source.updates.try_send(connection::Update::ThreadError { thread_id:"t".into(), error:"old selection denied".into() }).unwrap();
    cx.run_until_parked();
    assert!(app.read_with(cx, |app, _| !app.conversation_presentation.citation_navigation.as_ref().unwrap().unavailable));
    source.updates.try_send(connection::Update::ThreadError { thread_id:"source".into(), error:"history denied".into() }).unwrap();
    cx.run_until_parked();
    assert!(app.read_with(cx, |app, _| app.conversation_presentation.citation_navigation.as_ref().unwrap().unavailable));
    assert!(cx.debug_bounds("citation-navigation-retry").is_some());
    let retry = cx.debug_bounds("citation-navigation-retry").unwrap();
    cx.simulate_click(retry.center(), Default::default());
    cx.run_until_parked();
    assert!(matches!(source.requests.try_recv().unwrap(), connection::Command::SelectThread(Some(id)) if id == "source"));
    receipt(&source, vec![json!({"id":"answer","role":"assistant","text":"the original quote","streaming":false})], cx);
    assert!(app.read_with(cx, |app, _| app.conversation_presentation.citation_navigation.is_none()));
}
