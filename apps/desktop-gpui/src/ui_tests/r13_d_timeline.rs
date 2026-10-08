use super::*;
use gpui_kit::Modifiers;

fn click(cx: &mut VisualTestContext, selector: &'static str) {
    let bounds = cx
        .debug_bounds(selector)
        .unwrap_or_else(|| panic!("missing {selector}"));
    cx.simulate_click(bounds.center(), Modifiers::none());
    cx.run_until_parked();
}

#[gpui_kit::test]
fn r13_d_received_mention_discloses_and_opens_scoped_source_with_receipt(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    cx.update(|_, cx| app.update(cx, |app, cx| {
        app.apply_thread_update("t", snapshot(vec![json!({"id":"u","role":"user","text":"Please inspect [main](t3-context://v1/mention/main)","context":{"version":1,"records":[{"version":1,"kind":"mention","contextId":"main","label":"main","path":"src/main.rs"}]}})], "idle"), cx).unwrap();
        cx.notify();
    }));
    cx.run_until_parked();
    click(cx, "context-toggle-u-main");
    assert!(cx.debug_bounds("context-detail-u-main").is_some());
    click(cx, "context-open-source");
    let connection::Command::Call {
        tag,
        payload,
        reply,
    } = requests.try_recv().unwrap()
    else {
        panic!("file read")
    };
    assert_eq!(tag, "projects.readFile");
    assert_eq!(
        payload,
        json!({"cwd":"/tmp/fixture","relativePath":"src/main.rs"})
    );
    reply
        .send_blocking(Ok(
            json!({"relativePath":"src/main.rs","contents":"fn main() {}","truncated":false}),
        ))
        .unwrap();
    cx.run_until_parked();
    assert_eq!(
        app.read_with(cx, |app, cx| app.file_contents.read(cx).value().to_string()),
        "fn main() {}"
    );
    click(cx, "context-toggle-u-main");
    assert!(cx.debug_bounds("context-detail-u-main").is_none());
}

#[gpui_kit::test]
fn r13_d_plan_chosen_path_retries_failure_and_cancel_never_writes(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            let mut item = snapshot(vec![], "idle");
            item["snapshot"]["thread"]["proposedPlans"] =
                json!([{"id":"p1","planMarkdown":"# Plan\n\nShip it"}]);
            app.apply_thread_update("t", item, cx).unwrap();
            cx.notify();
        })
    });
    cx.run_until_parked();
    click(cx, "plan-save");
    assert!(requests.try_recv().is_err());
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            let input = app
                .conversation_presentation
                .plan_save
                .as_ref()
                .unwrap()
                .path
                .clone();
            input.update(cx, |input, cx| input.set_value("../escape.md", window, cx));
            app.save_conversation_plan(window, cx);
        })
    });
    cx.run_until_parked();
    assert!(requests.try_recv().is_err());
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            let input = app
                .conversation_presentation
                .plan_save
                .as_ref()
                .unwrap()
                .path
                .clone();
            input.update(cx, |input, cx| {
                input.set_value("docs/my-plan.md", window, cx)
            });
        })
    });
    click(cx, "plan-save-confirm");
    let connection::Command::Call {
        tag,
        payload,
        reply,
    } = requests.try_recv().unwrap()
    else {
        panic!("write")
    };
    assert_eq!(tag, "projects.writeFile");
    assert_eq!(
        payload,
        json!({"cwd":"/tmp/fixture","relativePath":"docs/my-plan.md","contents":"# Plan\n\nShip it"})
    );
    assert!(app.read_with(cx, |app, _| app.conversation_presentation.saving));
    reply
        .send_blocking(Err("read-only workspace".into()))
        .unwrap();
    cx.run_until_parked();
    assert!(app.read_with(cx, |app, _| {
        app.conversation_presentation.plan_save.is_some()
    }));
    assert!(!app.read_with(cx, |app, _| app.conversation_presentation.saving));
    click(cx, "plan-save-confirm");
    let connection::Command::Call { payload, reply, .. } = requests.try_recv().unwrap() else {
        panic!("retry")
    };
    assert_eq!(payload["relativePath"], "docs/my-plan.md");
    reply
        .send_blocking(Ok(json!({"relativePath":"docs/my-plan.md"})))
        .unwrap();
    cx.run_until_parked();
    assert!(app.read_with(cx, |app, _| {
        app.conversation_presentation.plan_save.is_none()
    }));
    click(cx, "plan-save");
    click(cx, "plan-save-cancel");
    assert!(requests.try_recv().is_err());
    assert!(app.read_with(cx, |app, _| {
        app.conversation_presentation.plan_save.is_none()
    }));
}

#[gpui_kit::test]
fn r13_d_shell_run_entry_sends_scoped_pty_command_and_projects_failure(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    let (id, _) = super::surfaces_terminal::open(&app, cx, &requests);
    cx.update(|_, cx| app.update(cx, |app, cx| {
        app.apply_thread_update("t", snapshot(vec![json!({"id":"a","role":"assistant","text":"```bash\necho café\n```","streaming":false})], "idle"), cx).unwrap();
        cx.notify();
    }));
    cx.run_until_parked();
    super::surfaces_terminal::drain_sizes(cx, &requests);
    click(cx, "run-shell-fence");
    let (payload, reply) = super::surfaces_terminal::call(&requests, "terminal.write");
    assert_eq!(
        payload,
        json!({"threadId":"t","terminalId":id,"data":"echo café\r"})
    );
    reply.send_blocking(Err("PTY write failed".into())).unwrap();
    cx.run_until_parked();
    assert_eq!(
        app.read_with(cx, |app, _| app.terminal_drawer.tabs[0].error.clone()),
        Some("PTY write failed".into())
    );
    super::surfaces_terminal::drain_sizes(cx, &requests);
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.connection_phase = connection::Phase::Reconnecting {
                attempt: 1,
                retry_in: std::time::Duration::from_secs(1),
                reason: "fixture".into(),
            };
            cx.notify();
        })
    });
    cx.run_until_parked();
    click(cx, "run-shell-fence");
    assert!(
        requests.try_recv().is_err(),
        "disabled reconnect entry must not enqueue or replay"
    );
}

#[gpui_kit::test]
fn r13_d_plan_workspace_image_signing_failure_and_retry_keep_exact_resource(
    cx: &mut TestAppContext,
) {
    let (app, cx, requests) = harness(cx);
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            let mut item = snapshot(vec![], "idle");
            item["snapshot"]["thread"]["proposedPlans"] =
                json!([{"id":"p1","planMarkdown":"# Plan\n\n![chart](charts/result.png)"}]);
            app.apply_thread_update("t", item, cx).unwrap();
            cx.notify();
        })
    });
    cx.run_until_parked();
    let connection::Command::Call {
        tag,
        payload,
        reply,
    } = requests.try_recv().unwrap()
    else {
        panic!("sign workspace image")
    };
    assert_eq!(tag, "assets.createUrl");
    let resource = json!({"_tag":"media-file","threadId":"t","path":"charts/result.png"});
    assert_eq!(payload, json!({"resource":resource}));
    reply
        .send_blocking(Err("unavailable asset".into()))
        .unwrap();
    cx.run_until_parked();
    assert!(
        cx.debug_bounds("workspace-image-open-charts/result.png")
            .is_some()
    );
    click(cx, "workspace-image-retry");
    let connection::Command::Call {
        tag,
        payload,
        reply,
    } = requests.try_recv().unwrap()
    else {
        panic!("retry signing")
    };
    assert_eq!(tag, "assets.createUrl");
    assert_eq!(payload, json!({"resource":resource}));
    reply
        .send_blocking(Err("still unavailable".into()))
        .unwrap();
    cx.run_until_parked();
    assert!(cx.debug_bounds("workspace-image-retry").is_some());
}

#[gpui_kit::test]
fn r13_d_table_copy_uses_rendered_cells_and_reports_feedback(cx: &mut TestAppContext) {
    let (app, cx, _) = harness(cx);
    cx.update(|_, cx| app.update(cx, |app, cx| {
        app.apply_thread_update("t", snapshot(vec![json!({"id":"a","role":"assistant","text":"| Name | Value |\n| --- | --- |\n| **alpha** | one,two |"})], "idle"), cx).unwrap();
        cx.notify();
    }));
    cx.run_until_parked();
    click(cx, "copy-table-csv");
    assert_eq!(
        cx.read_from_clipboard().unwrap().text().unwrap(),
        "Name,Value\r\nalpha,\"one,two\""
    );
    assert!(app.read_with(cx, |app, _| {
        app.conversation_presentation
            .copied
            .as_ref()
            .unwrap()
            .starts_with("table:")
    }));
    click(cx, "copy-table-markdown");
    let copied = cx.read_from_clipboard().unwrap().text().unwrap();
    assert!(copied.contains("**alpha**"));
    assert!(copied.contains("| Name | Value |"));
}

#[gpui_kit::test]
fn r13_d_work_groups_toggle_independently_across_message_boundary(cx: &mut TestAppContext) {
    let (app, cx, _) = harness(cx);
    cx.update(|_, cx| app.update(cx, |app, cx| {
        let mut item = snapshot(vec![
            json!({"id":"u","role":"user","text":"go","createdAt":"2026-01-01T00:00:00Z"}),
            json!({"id":"a","role":"assistant","text":"progress","turnId":"turn","createdAt":"2026-01-01T00:00:03Z"}),
            json!({"id":"b","role":"assistant","text":"done","turnId":"turn","createdAt":"2026-01-01T00:00:06Z"})], "idle");
        item["snapshot"]["thread"]["activities"] = json!([
            {"id":"c1","turnId":"turn","kind":"tool.completed","createdAt":"2026-01-01T00:00:01Z","payload":{"title":"bash","toolCallId":"c1"}},
            {"id":"c2","turnId":"turn","kind":"tool.completed","createdAt":"2026-01-01T00:00:04Z","payload":{"title":"bash","toolCallId":"c2"}}
        ]);
        app.apply_thread_update("t", item, cx).unwrap();
        cx.notify();
    }));
    cx.run_until_parked();
    click(cx, "activity-toggle-turn");
    assert!(cx.debug_bounds("tool-row-c1").is_some());
    assert!(cx.debug_bounds("tool-row-c2").is_none());
    click(cx, "activity-toggle");
    assert!(cx.debug_bounds("tool-row-c1").is_some());
    assert!(cx.debug_bounds("tool-row-c2").is_some());
    click(cx, "activity-toggle-turn");
    assert!(cx.debug_bounds("tool-row-c1").is_none());
    assert!(cx.debug_bounds("tool-row-c2").is_some());
}
