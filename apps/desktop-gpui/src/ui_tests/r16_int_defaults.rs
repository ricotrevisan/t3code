use super::*;

fn click(cx: &mut VisualTestContext, id: &'static str) {
    let bounds = cx
        .debug_bounds(id)
        .unwrap_or_else(|| panic!("missing {id}"));
    cx.simulate_click(bounds.center(), Default::default());
    cx.run_until_parked();
}
pub(super) fn call(
    requests: &Requests,
    expected: &str,
) -> (Value, async_channel::Sender<Result<Value, String>>) {
    loop {
        match requests.try_recv().unwrap() {
            connection::Command::Call {
                tag,
                payload,
                reply,
            } => {
                assert_eq!(tag, expected);
                return (payload, reply);
            }
            connection::Command::Subscribe { tag, .. } => assert_eq!(tag, "subscribeWorktreeSetup"),
            connection::Command::SelectThread(_) => {}
            _ => panic!("unexpected command"),
        }
    }
}
pub(super) fn save_permissions(
    app: &Entity<NativeApp>,
    cx: &mut VisualTestContext,
    requests: &Requests,
) {
    cx.simulate_resize(size(px(1600.), px(1600.)));
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.settings_open = true;
            app.settings_query.update(cx, |input, cx| {
                input.set_value("project defaults", window, cx)
            });
            cx.notify();
        })
    });
    cx.run_until_parked();
    let initial = json!({"defaultRuntimeMode":"full-access","defaultThreadEnvMode":"local"});
    click(cx, "scoped-refresh");
    call(requests, "server.getSettings")
        .1
        .send_blocking(Ok(initial.clone()))
        .unwrap();
    cx.run_until_parked();
    click(cx, "scope-0-defaultRuntimeMode-0");
    call(requests, "server.getSettings")
        .1
        .send_blocking(Ok(initial.clone()))
        .unwrap();
    cx.run_until_parked();
    let (patch, reply) = call(requests, "server.updateSettings");
    assert_eq!(
        patch,
        json!({"patch":{"defaultRuntimeMode":"approval-required"}})
    );
    let mut saved = initial;
    saved["defaultRuntimeMode"] = json!("approval-required");
    reply.send_blocking(Ok(saved)).unwrap();
    cx.run_until_parked();
    cx.update(|window, cx| app.update(cx, |app, cx| app.close_settings(window, cx)));
    cx.run_until_parked();
}
pub(super) fn send(
    app: &Entity<NativeApp>,
    cx: &mut VisualTestContext,
    requests: &Requests,
    mode: &str,
) -> (Value, async_channel::Sender<Result<Value, String>>) {
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.composer.update(cx, |input, cx| {
                input.set_value("first send 世界", window, cx)
            });
            app.send_prompt(window, cx);
        })
    });
    cx.run_until_parked();
    let (payload, reply) = call(requests, "orchestration.dispatchCommand");
    assert_eq!(payload["type"], "thread.turn.start");
    assert_eq!(payload["runtimeMode"], mode);
    assert_eq!(payload["bootstrap"]["createThread"]["runtimeMode"], mode);
    assert_eq!(
        payload["bootstrap"]["createThread"]["interactionMode"],
        "default"
    );
    (payload, reply)
}
#[gpui_kit::test]
fn r16_int_saved_approval_default_reaches_every_creation_entry_and_direct_bootstrap(
    cx: &mut TestAppContext,
) {
    let (app, cx, requests) = harness(cx);
    save_permissions(&app, cx, &requests);
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.shell.threads[0].branch = Some("main".into());
            cx.notify();
        })
    });
    for entry in [
        "sidebar",
        "cmd-n",
        "palette",
        "breadcrumb",
        "on-branch",
        "next-checkout",
    ] {
        cx.update(|window, cx| {
            app.update(cx, |app, cx| {
                app.select_thread("t", window, cx);
                app.new_thread_checkout = None;
                app.next_thread_defaults = Default::default();
                app.composer
                    .update(cx, |input, cx| input.set_value("", window, cx));
                cx.notify();
            })
        });
        cx.run_until_parked();
        while requests.try_recv().is_ok() {}
        match entry {
            "sidebar" => click(cx, "sidebar-new-thread"),
            "cmd-n" => {
                cx.simulate_keystrokes("cmd-n");
                cx.run_until_parked();
            }
            "palette" => {
                cx.simulate_keystrokes("cmd-k");
                cx.simulate_input("New thread in Fixture");
                cx.simulate_keystrokes("enter");
                cx.run_until_parked();
            }
            "breadcrumb" => click(cx, "header-project"),
            "on-branch" => {
                r14_e_header::ensure_control(cx, "thread-menu");
                click(cx, "thread-menu");
                click(cx, "new-thread-on-branch");
            }
            "next-checkout" => {
                cx.update(|window, cx| {
                    app.update(cx, |app, cx| {
                        app.choose_checkout(
                            "requested".into(),
                            Some("/tmp/verified".into()),
                            window,
                            cx,
                        )
                    })
                });
                cx.run_until_parked();
                let (payload, reply) = call(&requests, "vcs.listRefs");
                assert_eq!(payload["cwd"], "/tmp/verified");
                reply.send_blocking(Ok(json!({"isRepo":true,"refs":[{"name":"verified","current":true}],"nextCursor":null}))).unwrap();
                cx.run_until_parked();
                click(cx, "sidebar-new-thread");
            }
            _ => unreachable!(),
        }
        app.read_with(cx, |app, _| {
            assert_eq!(
                app.pending_threads
                    .last()
                    .unwrap()
                    .thread
                    .runtime_mode
                    .as_deref(),
                Some("approval-required"),
                "{entry}"
            );
        });
        let (payload, reply) = send(&app, cx, &requests, "approval-required");
        if entry == "next-checkout" {
            assert_eq!(
                payload["bootstrap"]["createThread"]["worktreePath"],
                "/tmp/verified"
            );
        }
        reply
            .send_blocking(Err("denied first send".into()))
            .unwrap();
        cx.run_until_parked();
        assert!(app.read_with(cx, |app, _| {
            app.pending_threads
                .iter()
                .any(|d| d.thread.id == payload["threadId"])
        }));
    }
}
#[gpui_kit::test]
fn r16_int_project_override_retarget_explicit_mode_and_queued_bootstrap(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    save_permissions(&app, cx, &requests);
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.shell.projects.push(
                serde_json::from_value(
                    json!({"id":"other","title":"Other","workspaceRoot":"/tmp/other"}),
                )
                .unwrap(),
            );
            let settings = app.project_settings.clone().unwrap();
            app.accept_config_snapshot(json!({"providers":[],"settings":settings,"environment":{"capabilities":{"projectSettingsOverrides":true}}}), cx);
            app.settings_open = true;
            app.settings_query.update(cx, |input, cx| input.set_value("project defaults", window, cx));
            cx.notify();
        })
    });
    cx.run_until_parked();
    let mut settings = app.read_with(cx, |app, _| app.project_settings.clone().unwrap());
    click(cx, "scoped-refresh");
    call(&requests, "server.getSettings")
        .1
        .send_blocking(Ok(settings.clone()))
        .unwrap();
    cx.run_until_parked();
    click(cx, "scope-1-defaultRuntimeMode-1");
    call(&requests, "server.getSettings")
        .1
        .send_blocking(Ok(settings.clone()))
        .unwrap();
    cx.run_until_parked();
    let (patch, reply) = call(&requests, "server.updateSettings");
    assert_eq!(
        patch["patch"]["projectSettingsOverrides"]["p"]["defaultRuntimeMode"],
        "auto-accept-edits"
    );
    settings["projectSettingsOverrides"] = json!({"p":{"defaultRuntimeMode":"auto-accept-edits"}});
    reply.send_blocking(Ok(settings)).unwrap();
    cx.run_until_parked();
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.close_settings(window, cx);
            app.new_thread(window, cx);
        })
    });
    cx.run_until_parked();
    let (_, reply) = send(&app, cx, &requests, "auto-accept-edits");
    reply.send_blocking(Err("denied".into())).unwrap();
    cx.run_until_parked();
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.selected_project = Some("other".into());
            app.new_thread(window, cx);
            app.composer.update(cx, |input, cx| {
                input.set_value("queued first send", window, cx)
            });
            app.queue_prompt(false, window, cx);
        })
    });
    cx.run_until_parked();
    let (thread, id) = app.read_with(cx, |app, _| {
        let thread = app.selected_thread.clone().unwrap();
        (thread.clone(), app.queues.messages(&thread)[0].id)
    });
    cx.update(|_, cx| app.update(cx, |app, cx| app.send_queued(&thread, id, cx)));
    cx.run_until_parked();
    let (payload, reply) = call(&requests, "orchestration.dispatchCommand");
    assert_eq!(payload["runtimeMode"], "approval-required");
    assert_eq!(payload["bootstrap"]["createThread"]["projectId"], "other");
    assert_eq!(
        payload["bootstrap"]["createThread"]["runtimeMode"],
        "approval-required"
    );
    reply.send_blocking(Err("queue denied".into())).unwrap();
    cx.run_until_parked();
    assert!(app.read_with(cx, |app, _| app.queues.held(&thread)
        && app.pending_threads.iter().any(|d| d.thread.id == thread)));
    cx.update(|_, cx| app.update(cx, |app, cx| app.send_queued(&thread, id, cx)));
    cx.run_until_parked();
    let (_, reply) = call(&requests, "orchestration.dispatchCommand");
    reply.send_blocking(Ok(json!({"sequence":2}))).unwrap();
    cx.run_until_parked();
    assert!(app.read_with(cx, |app, _| {
        !app.pending_threads.iter().any(|d| d.thread.id == thread)
    }));
    assert!(
        matches!(requests.try_recv().unwrap(), connection::Command::SelectThread(Some(id)) if id == thread)
    );
    assert!(requests.is_empty());
}
#[gpui_kit::test]
fn r16_int_file_worktree_default_loads_base_and_explicit_local_reverses_without_cross_project_leak(
    cx: &mut TestAppContext,
) {
    let (app, cx, requests) = harness(cx);
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.project_settings =
                Some(json!({"defaultRuntimeMode":"approval-required","defaultThreadEnvMode":null}));
            app.new_thread(window, cx);
        })
    });
    cx.run_until_parked();
    let (payload, reply) = call(&requests, "projects.readFile");
    assert_eq!(
        payload,
        json!({"cwd":"/tmp/fixture","relativePath":"t3.json"})
    );
    reply
        .send_blocking(Ok(
            json!({"contents":"{\"defaultThreadEnvMode\":\"worktree\"}","truncated":false}),
        ))
        .unwrap();
    cx.run_until_parked();
    let (payload, reply) = call(&requests, "vcs.listRefs");
    assert_eq!(payload["cwd"], "/tmp/fixture");
    reply
        .send_blocking(Ok(
            json!({"isRepo":true,"refs":[{"name":"main","current":true}],"nextCursor":null}),
        ))
        .unwrap();
    cx.run_until_parked();
    let (payload, reply) = send(&app, cx, &requests, "approval-required");
    assert_eq!(
        payload["bootstrap"]["prepareWorktree"]["baseBranch"],
        "main"
    );
    assert_eq!(
        payload["bootstrap"]["prepareWorktree"]["startFromOrigin"],
        true
    );
    assert_eq!(payload["bootstrap"]["runSetupScript"], true);
    reply.send_blocking(Err("setup denied".into())).unwrap();
    cx.run_until_parked();
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.choose_next_workspace(false);
            app.choose_next_setup(Some(true), Some(false));
            app.new_thread(window, cx);
        })
    });
    cx.run_until_parked();
    let (payload, reply) = send(&app, cx, &requests, "approval-required");
    assert!(payload["bootstrap"].get("prepareWorktree").is_none());
    assert_eq!(payload["bootstrap"]["runSetupScript"], false);
    reply.send_blocking(Err("denied".into())).unwrap();
    cx.run_until_parked();
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.shell.projects.push(
                serde_json::from_value(
                    json!({"id":"other","title":"Other","workspaceRoot":"/tmp/other"}),
                )
                .unwrap(),
            );
            app.selected_project = Some("other".into());
            app.new_thread(window, cx);
        })
    });
    cx.run_until_parked();
    let (payload, reply) = call(&requests, "projects.readFile");
    assert_eq!(payload["cwd"], "/tmp/other");
    reply
        .send_blocking(Ok(
            json!({"contents":"{\"defaultThreadEnvMode\":\"worktree\"}","truncated":false}),
        ))
        .unwrap();
    cx.run_until_parked();
    let (payload, reply) = call(&requests, "vcs.listRefs");
    assert_eq!(payload["cwd"], "/tmp/other");
    reply
        .send_blocking(Ok(
            json!({"isRepo":true,"refs":[{"name":"other-main","current":true}],"nextCursor":null}),
        ))
        .unwrap();
    cx.run_until_parked();
    let (payload, reply) = send(&app, cx, &requests, "approval-required");
    assert_eq!(
        payload["bootstrap"]["prepareWorktree"]["baseBranch"],
        "other-main"
    );
    assert_eq!(payload["bootstrap"]["runSetupScript"], true);
    assert_eq!(
        payload["bootstrap"]["prepareWorktree"]["startFromOrigin"],
        true
    );
    reply
        .send_blocking(Err("retained other draft".into()))
        .unwrap();
    cx.run_until_parked();
}
#[gpui_kit::test]
fn r16_int_default_reads_cannot_create_after_project_retarget(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.project_settings = None;
            app.new_thread(window, cx);
        })
    });
    cx.run_until_parked();
    let (_, reply) = call(&requests, "server.getSettings");
    cx.update(|_, cx| app.update(cx, |app, _| app.selected_project = Some("other".into())));
    reply
        .send_blocking(Ok(
            json!({"defaultRuntimeMode":"approval-required","defaultThreadEnvMode":"local"}),
        ))
        .unwrap();
    cx.run_until_parked();
    assert!(app.read_with(cx, |app, _| app.pending_threads.is_empty()
        && !app.new_thread_loading));
    assert!(requests.is_empty());
}
