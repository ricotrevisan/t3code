use super::*;
fn click(cx: &mut VisualTestContext, id: &'static str) {
    let bounds = cx.debug_bounds(id).unwrap();
    cx.simulate_click(bounds.center(), Default::default());
    cx.run_until_parked();
}
fn open(app: &Entity<NativeApp>, cx: &mut VisualTestContext, query: &str) {
    cx.simulate_resize(size(px(1600.), px(1600.)));
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.settings_open = true;
            app.settings_query
                .update(cx, |input, cx| input.set_value(query, window, cx));
            cx.notify();
        })
    });
    cx.run_until_parked();
}
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
        panic!("expected call")
    };
    assert_eq!(tag, expected);
    (payload, reply)
}
#[gpui_kit::test]
fn r14_f_diagnostics_transport_failure_retry_and_generation_fence(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    open(&app, cx, "diagnostics");
    click(cx, "scoped-refresh");
    let (payload, reply) = rpc(&requests, "server.getSettings");
    assert_eq!(payload, json!({}));
    reply.send_blocking(Ok(json!({}))).unwrap();
    cx.run_until_parked();
    let (payload, reply) = rpc(&requests, "server.getProcessDiagnostics");
    assert_eq!(payload, json!({}));
    reply.send_blocking(Err("not authorized".into())).unwrap();
    cx.run_until_parked();
    assert_eq!(
        app.read_with(cx, |app, _| app.scoped_pages.error.clone())
            .as_deref(),
        Some("not authorized")
    );
    click(cx, "scoped-refresh");
    let (_, reply) = rpc(&requests, "server.getSettings");
    reply.send_blocking(Ok(json!({}))).unwrap();
    cx.run_until_parked();
    let (_, reply) = rpc(&requests, "server.getProcessDiagnostics");
    reply.send_blocking(Ok(json!({"serverPid":21,"processCount":1,"readAt":"2026-10-10T00:00:00Z","totalRssBytes":123,"totalCpuPercent":4,"processes":[{"pid":22,"command":"agent","rssBytes":123,"status":"running"}],"error":null}))).unwrap();
    cx.run_until_parked();
    let (_, reply) = rpc(&requests, "server.getTraceDiagnostics");
    reply
        .send_blocking(Ok(
            json!({"recordCount":4,"failureCount":1,"latestWarningAndErrorLogs":[],"error":null}),
        ))
        .unwrap();
    cx.run_until_parked();
    assert_eq!(
        app.read_with(cx, |app, _| {
            app.scoped_pages.processes.as_ref().unwrap()["serverPid"].clone()
        }),
        json!(21)
    );
    click(cx, "scoped-refresh");
    let (_, reply) = rpc(&requests, "server.getSettings");
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.generation += 1;
            cx.notify();
        })
    });
    reply.send_blocking(Err("stale failure".into())).unwrap();
    cx.run_until_parked();
    assert!(app.read_with(cx, |app, _| app.scoped_pages.error.is_none()));
}
#[gpui_kit::test]
fn r14_f_environment_default_receipt_and_failed_mutation_keeps_value(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    open(&app, cx, "project defaults");
    click(cx, "scoped-refresh");
    let (_, reply) = rpc(&requests, "server.getSettings");
    reply
        .send_blocking(Ok(json!({"defaultThreadEnvMode":"local"})))
        .unwrap();
    cx.run_until_parked();
    click(cx, "scope-0-defaultThreadEnvMode-1");
    let (_, reply) = rpc(&requests, "server.getSettings");
    reply
        .send_blocking(Ok(json!({"defaultThreadEnvMode":"local"})))
        .unwrap();
    cx.run_until_parked();
    let (payload, reply) = rpc(&requests, "server.updateSettings");
    assert_eq!(
        payload,
        json!({"patch":{"defaultThreadEnvMode":"worktree"}})
    );
    reply.send_blocking(Err("write denied".into())).unwrap();
    cx.run_until_parked();
    assert_eq!(
        app.read_with(cx, |app, _| {
            app.scoped_pages.settings.as_ref().unwrap()["defaultThreadEnvMode"].clone()
        }),
        json!("local")
    );
    assert_eq!(
        app.read_with(cx, |app, _| app.scoped_pages.error.clone())
            .as_deref(),
        Some("write denied")
    );
}
#[gpui_kit::test]
fn r14_f_storage_retention_wire_and_keep_reverse(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.accept_config_snapshot(
                json!({"providers":[],"environment":{"capabilities":{"storageCleanup":true}}}),
                cx,
            )
        })
    });
    open(&app, cx, "storage");
    click(cx, "scoped-refresh");
    let (_, reply) = rpc(&requests, "server.getSettings");
    let mut settings = json!({"storageCleanup":{"logsAfterDays":30,"browserArtifactsAfterDays":7}});
    reply.send_blocking(Ok(settings.clone())).unwrap();
    cx.run_until_parked();
    click(cx, "storage-logsAfterDays-0");
    let (_, reply) = rpc(&requests, "server.getSettings");
    reply.send_blocking(Ok(settings.clone())).unwrap();
    cx.run_until_parked();
    let (payload, reply) = rpc(&requests, "server.updateSettings");
    assert_eq!(
        payload,
        json!({"patch":{"storageCleanup":{"logsAfterDays":null}}})
    );
    settings["storageCleanup"]["logsAfterDays"] = Value::Null;
    reply.send_blocking(Ok(settings)).unwrap();
    cx.run_until_parked();
    assert!(app.read_with(cx, |app, _| {
        app.scoped_pages.settings.as_ref().unwrap()["storageCleanup"]["logsAfterDays"].is_null()
    }));
}
#[gpui_kit::test]
fn r14_f_project_scope_reset_receipt_preserves_unrelated_overrides(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.accept_config_snapshot(
                json!({"providers":[], "environment":{"capabilities":{"projectSettingsOverrides":true}}}), cx,
            );
            app.selected_project = Some("p".into());
            cx.notify();
        })
    });
    open(&app, cx, "project defaults");
    click(cx, "scoped-refresh");
    let (_, reply) = rpc(&requests, "server.getSettings");
    let settings = json!({"defaultThreadEnvMode":"worktree","projectSettingsOverrides":{"p":{"defaultThreadEnvMode":"local","defaultRuntimeMode":"full-access"},"other":{"defaultThreadEnvMode":"local"}}});
    reply.send_blocking(Ok(settings.clone())).unwrap();
    cx.run_until_parked();
    click(cx, "scope-1-defaultThreadEnvMode-2");
    let (_, reply) = rpc(&requests, "server.getSettings");
    reply.send_blocking(Ok(settings.clone())).unwrap();
    cx.run_until_parked();
    let (payload, reply) = rpc(&requests, "server.updateSettings");
    assert_eq!(
        payload["patch"]["projectSettingsOverrides"]["p"],
        json!({"defaultRuntimeMode":"full-access"})
    );
    assert_eq!(
        payload["patch"]["projectSettingsOverrides"]["other"],
        settings["projectSettingsOverrides"]["other"]
    );
    let mut updated = settings;
    updated["projectSettingsOverrides"] = payload["patch"]["projectSettingsOverrides"].clone();
    reply.send_blocking(Ok(updated)).unwrap();
    cx.run_until_parked();
    assert!(app.read_with(cx, |app, _| {
        app.scoped_pages.settings.as_ref().unwrap()["projectSettingsOverrides"]["p"]
            .get("defaultThreadEnvMode")
            .is_none()
    }));
}
#[gpui_kit::test]
fn r14_f_chord_record_save_live_dispatch_and_reset(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    open(&app, cx, "keybindings");
    click(cx, "shortcut-chord-3");
    cx.simulate_keystrokes("ctrl-alt-j ctrl-alt-l enter");
    cx.run_until_parked();
    let binding = cx.update(|_, cx| {
        cx.key_bindings()
            .borrow()
            .bindings()
            .find(|b| {
                b.keystrokes()
                    .iter()
                    .map(|s| s.unparse())
                    .collect::<Vec<_>>()
                    .join(" ")
                    == "ctrl-alt-j ctrl-alt-l"
            })
            .cloned()
    });
    assert!(binding.is_some());
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.settings_open = false;
            app.composer.update(cx, |input, cx| input.focus(window, cx));
            cx.notify();
        })
    });
    cx.simulate_keystrokes("ctrl-alt-j ctrl-alt-l");
    cx.run_until_parked();
    assert!(app.read_with(cx, |app, _| app.palette_open));
    cx.simulate_keystrokes("escape");
    open(&app, cx, "keybindings");
    click(cx, "shortcut-reset-3");
    let present = cx.update(|_, cx| {
        cx.key_bindings()
            .borrow()
            .bindings()
            .any(|b| b.keystrokes().len() == 2 && b.keystrokes()[0].unparse() == "ctrl-alt-j")
    });
    assert!(!present);
    assert!(requests.is_empty());
}
#[gpui_kit::test]
fn r14_f_owned_settings_and_palette_geometry_scales_and_resets(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    open(&app, cx, "about");
    let initial = cx.debug_bounds("settings").unwrap().size.width;
    cx.update(|window, cx| {
        theme::update(window, cx, |p| p.zoom = 2).unwrap();
        app.update(cx, |_, cx| cx.notify());
    });
    cx.run_until_parked();
    assert!(cx.debug_bounds("settings").unwrap().size.width > initial);
    cx.update(|window, cx| {
        theme::update(window, cx, |p| p.zoom = 0).unwrap();
        app.update(cx, |_, cx| cx.notify());
    });
    cx.run_until_parked();
    assert_eq!(cx.debug_bounds("settings").unwrap().size.width, initial);
    click(cx, "settings-close");
    cx.simulate_keystrokes("cmd-k");
    cx.run_until_parked();
    let palette = cx.debug_bounds("command-palette").unwrap().size.width;
    cx.update(|window, cx| {
        theme::update(window, cx, |p| p.zoom = 2).unwrap();
        app.update(cx, |_, cx| cx.notify());
    });
    cx.run_until_parked();
    assert!(cx.debug_bounds("command-palette").unwrap().size.width > palette);
    assert!(requests.is_empty());
}
#[gpui_kit::test]
fn r14_f_palette_category_drilldown_and_reverse(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    cx.simulate_keystrokes("cmd-k");
    cx.run_until_parked();
    // The category row is a real palette destination, not a cosmetic header.
    click(cx, "palette-6");
    assert!(app.read_with(cx, |app, _| app.palette_open));
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            assert_eq!(app.palette_query.read(cx).value().as_ref(), "Settings › ");
            assert!(
                app.palette_items(cx)
                    .iter()
                    .all(|item| item.category() == "Settings")
            );
        })
    });
    click(cx, "palette-up");
    assert!(app.read_with(cx, |app, cx| app.palette_query.read(cx).value().is_empty()));
    assert!(requests.is_empty());
}
#[gpui_kit::test]
fn r14_f_file_listing_context_menu_adds_real_mention_to_durable_composer(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    cx.simulate_resize(size(px(1600.), px(1600.)));
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.inspector_open = true;
            app.right_tab = RightTab::Inspector;
            app.inspector_section = Some(InspectorSection::Files);
            app.file_path
                .update(cx, |input, cx| input.set_value("", window, cx));
            cx.notify();
        })
    });
    cx.run_until_parked();
    click(cx, "list-files");
    let (payload, reply) = rpc(&requests, "projects.listEntries");
    assert_eq!(payload["directoryPath"], json!(""));
    reply
        .send_blocking(Ok(
            json!({"entries":[{"path":"src/with space.rs","kind":"file"}],"truncated":false}),
        ))
        .unwrap();
    cx.run_until_parked();
    let press = cx.debug_bounds("file-entry-0").unwrap().center();
    cx.simulate_mouse_down(press, MouseButton::Right, Default::default());
    cx.simulate_mouse_up(press, MouseButton::Right, Default::default());
    cx.run_until_parked();
    cx.simulate_keystrokes("down down down enter");
    cx.run_until_parked();
    assert!(
        app.read_with(cx, |app, cx| app.composer.read(cx).value().contains(
            &t3code_desktop_gpui::mentions::file_link("src/with space.rs")
        ))
    );
    app.read_with(cx, |app, cx| {
        assert_eq!(
            app.drafts
                .get(app.selected_thread.as_ref().unwrap())
                .map(String::as_str),
            Some(app.composer.read(cx).value().as_ref())
        );
    });
    assert!(
        requests.is_empty(),
        "adding context must not open, write or execute the file"
    );
}
#[gpui_kit::test]
fn r14_f_repository_lookup_receipt_populates_clone_and_rejects_changed_input(
    cx: &mut TestAppContext,
) {
    let (app, cx, requests) = harness(cx);
    cx.simulate_resize(size(px(1600.), px(1200.)));
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.inspector_open = true;
            app.right_tab = RightTab::Inspector;
            app.inspector_section = Some(InspectorSection::Clone);
            app.clone_remote
                .update(cx, |input, cx| input.set_value("team/repo", window, cx));
            cx.notify();
        })
    });
    cx.run_until_parked();
    click(cx, "clone-source-1");
    click(cx, "clone-lookup");
    let (payload, reply) = rpc(&requests, "sourceControl.lookupRepository");
    assert_eq!(
        payload,
        json!({"provider":"github","repository":"team/repo"})
    );
    reply.send_blocking(Ok(json!({"nameWithOwner":"team/repo","sshUrl":"git@github.com:team/repo.git","url":"https://github.com/team/repo"}))).unwrap();
    cx.run_until_parked();
    assert_eq!(
        app.read_with(cx, |app, cx| app.clone_remote.read(cx).value().to_string()),
        "git@github.com:team/repo.git"
    );
    assert!(app.read_with(cx, |app, _| app.repository_chooser.provider.is_none()));
    click(cx, "clone-source-2");
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.clone_remote
                .update(cx, |input, cx| input.set_value("group/old", window, cx))
        })
    });
    click(cx, "clone-lookup");
    let (_, reply) = rpc(&requests, "sourceControl.lookupRepository");
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.clone_remote
                .update(cx, |input, cx| input.set_value("group/new", window, cx))
        })
    });
    reply
        .send_blocking(Ok(
            json!({"nameWithOwner":"group/old","sshUrl":"git@gitlab.com:group/old.git"}),
        ))
        .unwrap();
    cx.run_until_parked();
    assert_eq!(
        app.read_with(cx, |app, cx| app.clone_remote.read(cx).value().to_string()),
        "group/new"
    );
    assert!(requests.is_empty());
}
