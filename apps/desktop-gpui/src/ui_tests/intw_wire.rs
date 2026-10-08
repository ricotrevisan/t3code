use super::*;

fn rpc(
    requests: &Requests,
    expected: &str,
) -> (Value, async_channel::Sender<Result<Value, String>>) {
    let connection::Command::Call {
        tag,
        payload,
        reply,
    } = requests.try_recv().unwrap()
    else {
        panic!("expected {expected}")
    };
    assert_eq!(tag, expected);
    (payload, reply)
}

fn auth(phase: &str, flow: Value, interaction: Value) -> Value {
    json!({"instanceId":"login","phase":phase,"flowId":flow,"authorizationUrl":"https://example.test/start","message":"status from provider","interaction":interaction})
}

#[gpui_kit::test]
fn intw_auth_completion_replaces_phase_and_flow_without_reopening_browser(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    cx.update(|_, cx| app.update(cx, |app, cx| app.start_provider_auth("login", cx)));
    cx.run_until_parked();
    let (_, reply) = rpc(&requests, "provider.auth.start");
    let initial = auth(
        "waiting",
        json!("first"),
        json!({"type":"browser","id":"browser","url":"https://example.test/start","requiresConsent":false,"acceptsCallback":true}),
    );
    reply.send_blocking(Ok(initial.clone())).unwrap();
    cx.run_until_parked();
    assert_eq!(
        cx.opened_url().as_deref(),
        Some("https://example.test/start")
    );
    assert!(
        matches!(requests.try_recv().unwrap(), connection::Command::Subscribe { key, tag, payload } if key == "auth:login" && tag == "provider.auth.subscribe" && payload["instanceId"] == "login")
    );
    // A duplicate start receipt for this flow must not launch another URL.
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            cx.open_url("https://example.test/already-opened");
            app.accept_provider_auth(initial, true, cx);
        })
    });
    assert_eq!(
        cx.opened_url().as_deref(),
        Some("https://example.test/already-opened")
    );
    assert!(requests.is_empty());
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.auth_callback.update(cx, |input, cx| {
                input.set_value("https://example.test/callback", window, cx)
            });
            app.complete_provider_auth(cx);
            cx.open_url("https://example.test/sentinel");
        })
    });
    cx.run_until_parked();
    let (payload, reply) = rpc(&requests, "provider.auth.complete");
    assert_eq!(payload["flowId"], "first");
    let final_state = auth("succeeded", Value::Null, Value::Null);
    reply.send_blocking(Ok(final_state.clone())).unwrap();
    cx.run_until_parked();
    assert_eq!(
        cx.opened_url().as_deref(),
        Some("https://example.test/sentinel")
    );
    assert_eq!(
        app.read_with(cx, |app, _| app.auth_state.clone()),
        Some(final_state)
    );
    let (_, refresh) = rpc(&requests, "server.refreshProviders");
    refresh
        .send_blocking(Err("test refresh stops here".into()))
        .unwrap();
    cx.run_until_parked();
    cx.update(|_, cx| app.update(cx, |app, cx| app.complete_provider_auth(cx)));
    cx.run_until_parked();
    assert!(
        requests.is_empty(),
        "terminal/null-flow auth cannot submit the old callback"
    );
}

#[gpui_kit::test]
fn intw_auth_device_code_is_copyable_and_unsupported_forms_are_hidden(cx: &mut TestAppContext) {
    let (app, cx, _) = harness(cx);
    cx.update(|window, cx| app.update(cx, |app, cx| {
        app.settings_open = true;
        app.settings_query.update(cx, |input, cx| input.set_value("providers", window, cx));
        app.auth_state = Some(auth("waiting", json!("device"), json!({"type":"deviceCode","id":"d","url":"https://example.test/verify","userCode":"ABCD-1234"})));
        cx.notify();
    }));
    cx.run_until_parked();
    let copy = cx
        .debug_bounds("provider-auth-copy-code")
        .expect("device code is actionable");
    cx.simulate_click(copy.center(), Default::default());
    assert_eq!(
        cx.read_from_clipboard().unwrap().text().as_deref(),
        Some("ABCD-1234")
    );
    let open = cx
        .debug_bounds("provider-auth-open")
        .expect("device verification URL is actionable");
    cx.simulate_click(open.center(), Default::default());
    assert_eq!(
        cx.opened_url().as_deref(),
        Some("https://example.test/verify")
    );
    assert!(cx.debug_bounds("provider-auth-complete").is_none());
    for interaction in [
        json!({"type":"terminal","id":"t","output":"Run provider login"}),
        json!({"type":"credentials","id":"c","fields":[{"name":"key","label":"API key","secret":true}]}),
        json!({"type":"browser","id":"b","url":"https://example.test/consent","requiresConsent":true,"acceptsCallback":true}),
    ] {
        cx.update(|_, cx| {
            app.update(cx, |app, cx| {
                app.auth_state = Some(auth("waiting", json!("unsupported"), interaction));
                cx.notify();
            })
        });
        cx.run_until_parked();
        assert!(
            cx.debug_bounds("provider-auth-complete").is_none(),
            "unsupported interactions must not advertise callback submission"
        );
        cx.update(|_, cx| app.update(cx, |app, cx| app.complete_provider_auth(cx)));
    }
}

#[gpui_kit::test]
fn intw_subscription_full_queue_retries_and_new_selection_cancels_old_enqueue(
    cx: &mut TestAppContext,
) {
    let (app, cx, _) = harness(cx);
    let (sender, requests) = async_channel::bounded(1);
    sender.try_send(connection::Command::RetryNow).unwrap();
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.commands = Some(sender);
            app.enqueue_subscription(
                connection::Command::Subscribe {
                    key: "worktree-setup".into(),
                    tag: "subscribeWorktreeSetup".into(),
                    payload: json!({"threadId":"old"}),
                },
                cx,
            );
        })
    });
    cx.run_until_parked();
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.enqueue_subscription(
                connection::Command::Subscribe {
                    key: "worktree-setup".into(),
                    tag: "subscribeWorktreeSetup".into(),
                    payload: json!({"threadId":"new"}),
                },
                cx,
            )
        })
    });
    cx.run_until_parked();
    assert!(matches!(
        requests.try_recv().unwrap(),
        connection::Command::RetryNow
    ));
    cx.run_until_parked();
    assert!(
        matches!(requests.try_recv().unwrap(), connection::Command::Subscribe { payload, .. } if payload["threadId"] == "new")
    );
    assert!(requests.is_empty());
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.commands
                .as_ref()
                .unwrap()
                .try_send(connection::Command::RetryNow)
                .unwrap();
            app.enqueue_subscription(
                connection::Command::Subscribe {
                    key: "server-config".into(),
                    tag: "subscribeServerConfig".into(),
                    payload: json!({"usageLimitSources":true}),
                },
                cx,
            );
        })
    });
    cx.run_until_parked();
    requests.try_recv().unwrap();
    cx.run_until_parked();
    assert!(
        matches!(requests.try_recv().unwrap(), connection::Command::Subscribe { tag, .. } if tag == "subscribeServerConfig")
    );
}

#[gpui_kit::test]
fn intw_subscription_disconnect_cancels_pending_send_and_closed_queue_surfaces_error(
    cx: &mut TestAppContext,
) {
    let (app, cx, _) = harness(cx);
    let (sender, requests) = async_channel::bounded(1);
    sender.try_send(connection::Command::RetryNow).unwrap();
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.commands = Some(sender.clone());
            app.enqueue_subscription(
                connection::Command::Subscribe {
                    key: "project-clones".into(),
                    tag: "subscribeProjectClones".into(),
                    payload: json!({}),
                },
                cx,
            );
        })
    });
    cx.run_until_parked();
    cx.update(|_, cx| app.update(cx, |app, cx| app.disconnect(cx)));
    requests.try_recv().unwrap();
    cx.run_until_parked();
    assert!(requests.is_empty());
    requests.close();
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.commands = Some(sender);
            app.enqueue_subscription(
                connection::Command::Subscribe {
                    key: "server-config".into(),
                    tag: "subscribeServerConfig".into(),
                    payload: json!({}),
                },
                cx,
            );
        })
    });
    assert!(app.read_with(cx, |app, _| {
        app.status.contains("Subscription not requested")
    }));
}

#[gpui_kit::test]
fn intw_search_and_attachment_names_use_utf16_contract_limits(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.search.update(cx, |input, cx| {
                input.set_value("😀".repeat(101), window, cx)
            });
            app.search_threads(cx);
            let scope = app.attachment_scope().unwrap();
            app.attach_inputs(
                scope,
                vec![crate::app::attachments::AttachmentInput::Bytes {
                    name: format!("{}.txt", "😀".repeat(128)),
                    bytes: vec![1],
                    source: None,
                }],
                cx,
            );
        })
    });
    cx.run_until_parked();
    assert!(
        requests.is_empty(),
        "oversize Unicode must never reach upload URL issuance/search RPC"
    );
    assert!(!app.read_with(cx, |app, _| app.attachment_drafts.uploading));
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.search.update(cx, |input, cx| {
                input.set_value("😀".repeat(100), window, cx)
            });
            app.search_threads(cx);
        })
    });
    cx.run_until_parked();
    let (payload, reply) = rpc(&requests, "orchestration.searchThreads");
    assert_eq!(
        payload["query"].as_str().unwrap().encode_utf16().count(),
        200
    );
    reply.send_blocking(Ok(json!({"matches":[]}))).unwrap();
}

#[gpui_kit::test]
fn intw_conversation_shares_projection_and_detects_direct_arc_replacement(cx: &mut TestAppContext) {
    let (app, cx, _) = harness(cx);
    cx.update(|_, cx| app.update(cx, |app, cx| {
        app.detail.thread = Some(json!({"id":"t","messages":[{"id":"u","role":"user","text":"first","createdAt":"2026-01-01T00:00:00Z"}],"activities":[],"checkpoints":[],"proposedPlans":[]}).into());
        cx.notify();
    }));
    cx.run_until_parked();
    let (revision, preparations) = app.read_with(cx, |app, _| {
        assert!(
            app.conversation_presentation
                .same_projection(app.detail.thread.as_ref())
        );
        assert!(
            std::sync::Arc::strong_count(app.detail.thread.as_ref().unwrap()) >= 2,
            "cache shares, not deep-copies projection"
        );
        (
            app.detail.revision,
            app.conversation_presentation.preparations,
        )
    });
    cx.update(|_, cx| app.update(cx, |_, cx| cx.notify()));
    cx.run_until_parked();
    assert_eq!(
        app.read_with(cx, |app, _| app.conversation_presentation.preparations),
        preparations
    );
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            let mut replacement = app.detail.thread.as_ref().unwrap().as_ref().clone();
            replacement["messages"][0]["text"] = json!("replacement");
            app.detail.thread = Some(replacement.into());
            cx.notify();
        })
    });
    cx.run_until_parked();
    app.read_with(cx, |app, _| {
        assert_eq!(app.detail.revision, revision);
        assert_eq!(app.conversation_presentation.preparations, preparations + 1);
        assert_eq!(
            *app.conversation_rows,
            t3code_desktop_gpui::conversation::timeline_rows(
                app.detail.thread.as_ref().unwrap(),
                &app.activity_expanded
            )
        );
    });
}
