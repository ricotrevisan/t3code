use super::r16_int_defaults::{call, save_permissions, send};
use super::*;

#[gpui_kit::test]
fn r16_int_explicit_permissions_persist_and_unsupported_defaults_never_escalate(
    cx: &mut TestAppContext,
) {
    let (app, cx, requests) = harness(cx);
    save_permissions(&app, cx, &requests);
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            let mut provider = composer_parity::provider_fixture("codex", "Personal");
            provider.supported_runtime_modes = Some(vec!["full-access".into()]);
            provider.default_runtime_mode = Some("full-access".into());
            app.provider_catalog = vec![provider];
            app.new_thread(window, cx);
            app.composer.update(cx, |input, cx| {
                input.set_value("do not escalate", window, cx)
            });
            app.send_prompt(window, cx);
        })
    });
    cx.run_until_parked();
    while let Ok(command) = requests.try_recv() {
        assert!(!matches!(command, connection::Command::Call { .. }));
    }
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            assert!(app.composer_message(cx).is_err());
            app.change_thread_mode("thread.runtime-mode.set", "runtimeMode", "full-access", cx);
            let restored: drafts::PendingThread = serde_json::from_value(
                serde_json::to_value(app.pending_threads.last().unwrap()).unwrap(),
            )
            .unwrap();
            assert_eq!(
                restored.defaults_override.runtime_mode.as_deref(),
                Some("full-access")
            );
            app.pending_threads.pop();
            app.pending_threads.push(restored);
            app.project_settings.as_mut().unwrap()["defaultRuntimeMode"] =
                json!("auto-accept-edits");
        })
    });
    let (_, reply) = send(&app, cx, &requests, "full-access");
    reply
        .send_blocking(Err("retain explicit draft".into()))
        .unwrap();
    cx.run_until_parked();
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.provider_catalog.clear();
            app.project_settings.as_mut().unwrap()["defaultRuntimeMode"] =
                json!("approval-required");
            app.new_thread(window, cx);
        })
    });
    cx.run_until_parked();
    let (_, reply) = send(&app, cx, &requests, "approval-required");
    reply.send_blocking(Err("retained".into())).unwrap();
    cx.run_until_parked();
}

#[gpui_kit::test]
fn r16_int_pending_draft_reloads_inherited_file_and_base_before_first_send(
    cx: &mut TestAppContext,
) {
    let (app, cx, requests) = harness(cx);
    save_permissions(&app, cx, &requests);
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.new_thread(window, cx);
            app.project_settings.as_mut().unwrap()["defaultThreadEnvMode"] = Value::Null;
            app.composer.update(cx, |input, cx| {
                input.set_value("retain while loading defaults", window, cx)
            });
            app.send_prompt(window, cx);
        })
    });
    cx.run_until_parked();
    let (_, reply) = call(&requests, "projects.readFile");
    reply
        .send_blocking(Ok(
            json!({"contents":"{\"defaultThreadEnvMode\":\"worktree\"}","truncated":false}),
        ))
        .unwrap();
    cx.run_until_parked();
    let (_, reply) = call(&requests, "vcs.listRefs");
    reply
        .send_blocking(Ok(
            json!({"isRepo":true,"refs":[{"name":"main","current":true}],"nextCursor":null}),
        ))
        .unwrap();
    cx.run_until_parked();
    let (payload, reply) = call(&requests, "orchestration.dispatchCommand");
    assert_eq!(payload["runtimeMode"], "approval-required");
    assert_eq!(
        payload["bootstrap"]["prepareWorktree"]["baseBranch"],
        "main"
    );
    reply.send_blocking(Err("setup cancelled".into())).unwrap();
    cx.run_until_parked();
    assert!(
        app.read_with(cx, |app, cx| app.composer.read(cx).value().as_str()
            == "retain while loading defaults")
    );
}

#[gpui_kit::test]
fn r16_int_simultaneous_pages_capture_distinct_defaults_and_background_acceptance(
    cx: &mut TestAppContext,
) {
    let (office, cx, _) = harness(cx);
    let lab = cx.update(|window, cx| {
        office.update(cx, |app, cx| {
            app.create_environment_page("https://lab.example", window, cx)
        })
    });
    let office_source = r16_d_citations::connect(office.clone(), "office-id", cx);
    let lab_source = r16_d_citations::connect(lab.clone(), "lab-id", cx);
    for (page, source, mode) in [
        (&office, &office_source, "approval-required"),
        (&lab, &lab_source, "auto-accept-edits"),
    ] {
        cx.update(|_, cx| {
            page.update(cx, |app, cx| {
                app.perform("server.getSettings", json!({}), None, cx)
            })
        });
        cx.run_until_parked();
        call(&source.requests, "server.getSettings")
            .1
            .send_blocking(Ok(
                json!({"defaultRuntimeMode":mode,"defaultThreadEnvMode":"local"}),
            ))
            .unwrap();
        cx.run_until_parked();
        cx.update(|window, cx| {
            page.update(cx, |app, cx| {
                app.selected_project = Some("p".into());
                app.provider
                    .update(cx, |input, cx| input.set_value("codex", window, cx));
                app.model
                    .update(cx, |input, cx| input.set_value("gpt-5.4", window, cx));
                app.new_thread(window, cx);
            })
        });
        cx.run_until_parked();
    }
    let (office_payload, office_reply) =
        send(&office, cx, &office_source.requests, "approval-required");
    let (lab_payload, lab_reply) = send(&lab, cx, &lab_source.requests, "auto-accept-edits");
    assert_ne!(office_payload["threadId"], lab_payload["threadId"]);
    office_reply
        .send_blocking(Ok(json!({"sequence":1})))
        .unwrap();
    lab_reply.send_blocking(Err("lab denied".into())).unwrap();
    cx.run_until_parked();
    assert!(office.read_with(cx, |app, _| app.pending_threads.is_empty()));
    assert!(lab.read_with(cx, |app, cx| app.pending_threads.len() == 1
        && app.composer.read(cx).value().as_str() == "first send 世界"));
    office_source.updates.try_send(connection::Update::Shell(Box::new(serde_json::from_value(json!({"kind":"snapshot","snapshot":{"snapshotSequence":1,"projects":[{"id":"p","title":"Repo","workspaceRoot":"/tmp/fixture"}],"threads":[{"id":office_payload["threadId"],"projectId":"p","title":"New thread","modelSelection":{"instanceId":"codex","model":"gpt-5.4"},"runtimeMode":"approval-required","interactionMode":"default","archivedAt":null}]}})).unwrap()))).unwrap();
    cx.run_until_parked();
    assert!(office.read_with(cx, |app, _| {
        app.shell.threads.iter().any(|t| {
            t.id == office_payload["threadId"]
                && t.runtime_mode.as_deref() == Some("approval-required")
        })
    }));
    assert!(lab.read_with(cx, |app, _| {
        !app.shell
            .threads
            .iter()
            .any(|t| t.id == office_payload["threadId"])
    }));
}
