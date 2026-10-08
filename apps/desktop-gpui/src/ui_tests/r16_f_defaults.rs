use super::*;
fn click(cx: &mut VisualTestContext, id: &'static str) {
    let bounds = cx.debug_bounds(id).unwrap();
    cx.simulate_click(bounds.center(), Default::default());
    cx.run_until_parked();
}
fn reply(requests: &Requests, tag: &str, result: Result<Value, String>) -> Value {
    let connection::Command::Call {
        tag: actual,
        payload,
        reply,
    } = requests.try_recv().unwrap()
    else {
        panic!("expected RPC")
    };
    assert_eq!(actual, tag);
    reply.send_blocking(result).unwrap();
    payload
}
#[gpui_kit::test]
fn r16_f_workspace_default_write_and_environment_inherit_reverse(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
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
    let mut settings =
        json!({"defaultRuntimeMode":"approval-required","defaultThreadEnvMode":"local"});
    click(cx, "scoped-refresh");
    reply(&requests, "server.getSettings", Ok(settings.clone()));
    cx.run_until_parked();
    click(cx, "scope-0-defaultThreadEnvMode-1");
    reply(&requests, "server.getSettings", Ok(settings.clone()));
    cx.run_until_parked();
    settings["defaultThreadEnvMode"] = json!("worktree");
    assert_eq!(
        reply(&requests, "server.updateSettings", Ok(settings.clone())),
        json!({"patch":{"defaultThreadEnvMode":"worktree"}})
    );
    cx.run_until_parked();
    assert_eq!(
        app.read_with(cx, |app, _| app
            .effective_thread_defaults("p", None, None, None)
            .unwrap()
            .workspace_mode),
        "worktree"
    );
    click(cx, "scope-0-defaultThreadEnvMode-2");
    reply(&requests, "server.getSettings", Ok(settings.clone()));
    cx.run_until_parked();
    settings
        .as_object_mut()
        .unwrap()
        .remove("defaultThreadEnvMode");
    assert_eq!(
        reply(&requests, "server.updateSettings", Ok(settings)),
        json!({"patch":{"defaultThreadEnvMode":null}})
    );
    cx.run_until_parked();
    assert_eq!(
        app.read_with(cx, |app, _| app
            .effective_thread_defaults("p", None, None, None)
            .unwrap()
            .workspace_mode),
        "local"
    );
    let file = json!({"defaultThreadEnvMode":"worktree"});
    assert_eq!(
        app.read_with(cx, |app, _| app
            .effective_thread_defaults("p", None, Some(&file), None)
            .unwrap()
            .workspace_mode),
        "worktree"
    );
}

#[gpui_kit::test]
fn r16_f_project_inherit_receipt_updates_resolver_without_other_project_leak(
    cx: &mut TestAppContext,
) {
    let (app, cx, requests) = harness(cx);
    cx.simulate_resize(size(px(1600.), px(1600.)));
    cx.update(|window, cx| app.update(cx, |app, cx| {
        app.accept_config_snapshot(json!({"providers":[],"environment":{"capabilities":{"projectSettingsOverrides":true}}}), cx);
        app.selected_project = Some("p".into());
        app.settings_open = true;
        app.settings_query.update(cx, |input, cx| input.set_value("project defaults", window, cx));
        cx.notify();
    }));
    cx.run_until_parked();
    let settings = json!({"defaultRuntimeMode":"approval-required","defaultThreadEnvMode":"worktree","projectSettingsFolded":true,"projectSettingsOverrides":{"p":{"defaultRuntimeMode":"full-access","defaultThreadEnvMode":"local"},"other":{"defaultRuntimeMode":"auto-accept-edits"}}});
    click(cx, "scoped-refresh");
    reply(&requests, "server.getSettings", Ok(settings.clone()));
    cx.run_until_parked();
    assert_eq!(
        app.read_with(cx, |app, _| app
            .effective_thread_defaults("p", None, None, None)
            .unwrap()
            .runtime_mode),
        "full-access"
    );
    click(cx, "scope-1-defaultRuntimeMode-4");
    reply(&requests, "server.getSettings", Ok(settings.clone()));
    cx.run_until_parked();
    let mut saved = settings.clone();
    saved["projectSettingsOverrides"]["p"]
        .as_object_mut()
        .unwrap()
        .remove("defaultRuntimeMode");
    let payload = reply(&requests, "server.updateSettings", Ok(saved.clone()));
    assert_eq!(
        payload["patch"]["projectSettingsOverrides"],
        saved["projectSettingsOverrides"]
    );
    cx.run_until_parked();
    assert_eq!(
        app.read_with(cx, |app, _| app
            .effective_thread_defaults("p", None, None, None)
            .unwrap()
            .runtime_mode),
        "approval-required"
    );
    assert_eq!(
        app.read_with(cx, |app, _| app
            .effective_thread_defaults("p", None, None, None)
            .unwrap()
            .workspace_mode),
        "local"
    );
    assert_eq!(
        app.read_with(cx, |app, _| app
            .effective_thread_defaults("other", None, None, None)
            .unwrap()
            .runtime_mode),
        "auto-accept-edits"
    );
    assert!(requests.is_empty());
}

#[gpui_kit::test]
fn r16_f_saved_permissions_feed_page_resolver_denial_and_stale_receipt(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
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
    click(cx, "scoped-refresh");
    let initial = json!({"defaultRuntimeMode":"full-access","defaultThreadEnvMode":"local"});
    assert_eq!(
        reply(&requests, "server.getSettings", Ok(initial.clone())),
        json!({})
    );
    cx.run_until_parked();
    click(cx, "scope-0-defaultRuntimeMode-0");
    reply(&requests, "server.getSettings", Ok(initial));
    cx.run_until_parked();
    let saved = json!({"defaultRuntimeMode":"approval-required","defaultThreadEnvMode":"worktree"});
    assert_eq!(
        reply(&requests, "server.updateSettings", Ok(saved.clone())),
        json!({"patch":{"defaultRuntimeMode":"approval-required"}})
    );
    cx.run_until_parked();
    assert_eq!(
        app.read_with(cx, |app, _| app
            .effective_thread_defaults("p", None, None, None)
            .unwrap()
            .runtime_mode),
        "approval-required"
    );
    assert_eq!(
        app.read_with(cx, |app, _| app
            .effective_thread_defaults("p", None, None, None)
            .unwrap()
            .workspace_mode),
        "worktree"
    );
    click(cx, "scope-0-defaultRuntimeMode-3");
    reply(&requests, "server.getSettings", Ok(saved));
    cx.run_until_parked();
    reply(&requests, "server.updateSettings", Err("denied".into()));
    cx.run_until_parked();
    assert_eq!(
        app.read_with(cx, |app, _| app
            .effective_thread_defaults("p", None, None, None)
            .unwrap()
            .runtime_mode),
        "approval-required"
    );
    assert_eq!(
        app.read_with(cx, |app, _| app.scoped_pages.error.clone()),
        Some("denied".into())
    );
    click(cx, "scoped-refresh");
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.generation += 1;
            app.project_settings = None;
            cx.notify();
        })
    });
    reply(
        &requests,
        "server.getSettings",
        Ok(json!({"defaultRuntimeMode":"full-access"})),
    );
    cx.run_until_parked();
    assert!(app.read_with(cx, |app, _| {
        app.effective_thread_defaults("p", None, None, None)
            .is_err()
    }));
}
