use super::*;
use gpui_kit::Modifiers;

#[gpui_kit::test]
fn sidebar_fixb_cleanup_checkbox_run_and_results_are_real(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    seed_second_thread(&app, cx, false);
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.sidebar_state.capabilities = json!({"threadCleanupReview":true});
            app.sidebar_state.cleanup.target = Some((app.environment_scope.clone(), "u".into()));
            app.review_sidebar_cleanup(false, cx);
        })
    });
    cx.run_until_parked();
    let connection::Command::Call { reply, .. } = requests.try_recv().unwrap() else {
        panic!("expected review");
    };
    reply
        .send_blocking(Ok(
            json!({"reviewId":"review-success","threadId":"u","title":"Other","actions":[
                {"id":"archive","title":"Archive","detail":"Keep history","blockedReason":null}
            ]}),
        ))
        .unwrap();
    cx.run_until_parked();
    let choice = cx.debug_bounds("cleanup-choice-archive").unwrap();
    cx.simulate_click(choice.origin + point(px(8.), px(8.)), Default::default());
    cx.run_until_parked();
    assert_eq!(
        app.read_with(cx, |app, _| app.sidebar_state.cleanup.selected.clone()),
        vec!["archive"]
    );
    let run = cx.debug_bounds("cleanup-run").unwrap();
    cx.simulate_click(run.center(), Default::default());
    cx.run_until_parked();
    let connection::Command::Call {
        tag,
        payload,
        reply,
    } = requests.try_recv().unwrap()
    else {
        panic!("expected run");
    };
    assert_eq!(tag, "thread.cleanupRun");
    assert_eq!(payload["selected"], json!(["archive"]));
    reply.send_blocking(Ok(json!({"threadId":"u","actions":[{"id":"archive","status":"completed","detail":"Archived"}]}))).unwrap();
    cx.run_until_parked();
    assert_eq!(
        app.read_with(cx, |app, _| app
            .sidebar_state
            .cleanup
            .result
            .as_ref()
            .unwrap()
            .actions[0]
            .status
            .clone()),
        "completed"
    );
    assert!(cx.debug_bounds("cleanup-run").is_none());
    assert_eq!(
        app.read_with(cx, |app, _| app.selected_thread.clone()),
        Some("t".into())
    );
}

#[gpui_kit::test]
fn sidebar_fixb_switching_view_scrolls_selected_row_into_view(cx: &mut TestAppContext) {
    let (app, cx, _) = harness(cx);
    cx.update(|_, cx| app.update(cx, |app, cx| {
        let mut key: Option<String> = None;
        for index in 0..40 {
            key = sidebar::key_between(key.as_deref(), None);
            app.shell.threads.push(serde_json::from_value(json!({
                "id":format!("tail-{index}"), "projectId":"p", "title":format!("Tail {index}"), "activeOrderKey":key
            })).unwrap());
        }
        app.selected_thread = Some("tail-39".into());
        app.shell.revision += 1;
        cx.notify();
    }));
    cx.run_until_parked();
    let priority = cx.debug_bounds("priority-view").unwrap();
    cx.simulate_click(priority.center(), Default::default());
    cx.run_until_parked();
    cx.update(|window, cx| {
        assert!(window.simulate_next_frame(cx) > 0);
    });
    cx.run_until_parked();
    let viewport = cx.debug_bounds("sidebar-list").unwrap();
    let row = cx.debug_bounds("thread-tail-39").unwrap();
    assert!(
        app.read_with(cx, |app, _| app.sidebar_state.scroll.offset().y < px(0.)),
        "anchor scroll must execute"
    );
    assert!(
        row.top() >= viewport.top() && row.bottom() <= viewport.bottom(),
        "selected row {row:?} not in {viewport:?}"
    );
}

#[gpui_kit::test]
fn sidebar_fixb_project_settings_preserves_foreign_row_context_on_failure(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    seed_second_thread(&app, cx, false);
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.shell.projects.push(
                serde_json::from_value(
                    json!({"id":"q","title":"Foreign","workspaceRoot":"/tmp/foreign"}),
                )
                .unwrap(),
            );
            app.shell
                .threads
                .iter_mut()
                .find(|row| row.id == "u")
                .unwrap()
                .project_id = "q".into();
            app.shell.revision += 1;
            cx.notify();
        })
    });
    cx.run_until_parked();
    right_click(cx, "thread-u");
    let settings = cx.debug_bounds("project-settings").unwrap();
    cx.simulate_click(settings.center(), Default::default());
    cx.run_until_parked();
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.sidebar_rename.update(cx, |input, cx| {
                input.set_value("Renamed foreign", window, cx)
            });
        })
    });
    let save = cx.debug_bounds("sidebar-project-rename").unwrap();
    cx.simulate_click(save.center(), Default::default());
    cx.run_until_parked();
    let connection::Command::Call { payload, reply, .. } = requests.try_recv().unwrap() else {
        panic!("expected project settings write");
    };
    let command = payload.get("command").unwrap_or(&payload);
    assert_eq!(command["type"], "project.meta.update");
    assert_eq!(command["projectId"], "q");
    assert_eq!(command["title"], "Renamed foreign");
    reply
        .send_blocking(Err("project update rejected".into()))
        .unwrap();
    cx.run_until_parked();
    assert_eq!(
        app.read_with(cx, |app, _| app.selected_project.clone()),
        Some("p".into())
    );
    assert_eq!(
        app.read_with(cx, |app, _| app.selected_thread.clone()),
        Some("t".into())
    );
    assert_eq!(
        app.read_with(cx, |app, cx| app
            .sidebar_rename
            .read(cx)
            .value()
            .to_string()),
        "Renamed foreign"
    );
    assert!(app.read_with(cx, |app, _| {
        app.sidebar_state
            .project_settings_target
            .as_ref()
            .is_some_and(|(_, id)| id == "q")
    }));
}

#[gpui_kit::test]
fn sidebar_fixb_real_drag_reorders_and_rejection_keeps_shell(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    seed_second_thread(&app, cx, false);
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.sidebar_state.priority = true;
            app.sidebar_state.capabilities = json!({"threadActiveReorder":true});
            app.shell
                .threads
                .iter_mut()
                .find(|row| row.id == "t")
                .unwrap()
                .active_order_key = Some("b".into());
            app.shell
                .threads
                .iter_mut()
                .find(|row| row.id == "u")
                .unwrap()
                .active_order_key = Some("d".into());
            app.shell.revision += 1;
            cx.notify();
        })
    });
    cx.run_until_parked();
    let from = cx.debug_bounds("thread-u").unwrap().center();
    let to = cx.debug_bounds("thread-t").unwrap().center();
    drag(cx, from, to);
    let connection::Command::Call { payload, reply, .. } =
        requests.try_recv().expect("real GPUI drag dispatches")
    else {
        panic!("expected reorder");
    };
    let command = payload.get("command").unwrap_or(&payload);
    assert_eq!(command["type"], "thread.active.reorder");
    assert_eq!(command["threadId"], "u");
    assert!(command["orderKey"].as_str().unwrap() < "b");
    reply.send_blocking(Err("reorder rejected".into())).unwrap();
    cx.run_until_parked();
    assert_eq!(
        app.read_with(cx, |app, _| app.sidebar_selectable_order()),
        vec!["t", "u"]
    );
    assert!(!app.read_with(cx, |app, _| app.sidebar_state.order_busy));
    assert!(requests.try_recv().is_err());
}

#[gpui_kit::test]
fn sidebar_fixb_drop_across_shelves_and_stale_drag_rejection(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    seed_second_thread(&app, cx, false);
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.sidebar_state.priority = true;
            app.sidebar_state.capabilities = json!({"threadPinning":true,"threadPinReorder":true});
            let stale = sidebar::Drag {
                id: "u".into(),
                title: "Other".into(),
                environment: app.environment_scope.clone(),
                generation: app.generation.wrapping_add(1),
            };
            app.drop_sidebar_thread(&stale, "pinned", None, cx);
            cx.notify();
        })
    });
    cx.run_until_parked();
    assert!(requests.try_recv().is_err());
    let from = cx.debug_bounds("thread-u").unwrap().center();
    let to = cx.debug_bounds("priority-pinned").unwrap().center();
    drag(cx, from, to);
    let connection::Command::Call { payload, reply, .. } = requests.try_recv().unwrap() else {
        panic!("expected pin");
    };
    let command = payload.get("command").unwrap_or(&payload);
    assert_eq!(command["type"], "thread.pin");
    assert_eq!(command["threadId"], "u");
    assert!(command["orderKey"].is_string());
    reply.send_blocking(Err("pin rejected".into())).unwrap();
    cx.run_until_parked();
    assert!(app.read_with(cx, |app, _| {
        app.shell
            .threads
            .iter()
            .find(|row| row.id == "u")
            .unwrap()
            .pinned_at
            .is_none()
    }));
    assert_eq!(
        app.read_with(cx, |app, _| app.selected_thread.clone()),
        Some("t".into())
    );
}

#[gpui_kit::test]
fn sidebar_fixb_cleanup_review_run_and_rejected_receipt_retains_review(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    seed_second_thread(&app, cx, false);
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.sidebar_state.capabilities = json!({"threadCleanupReview":true});
            app.sidebar_state.cleanup.target = Some((app.environment_scope.clone(), "u".into()));
            app.review_sidebar_cleanup(false, cx);
        })
    });
    cx.run_until_parked();
    let connection::Command::Call {
        tag,
        payload,
        reply,
    } = requests.try_recv().unwrap()
    else {
        panic!("expected review");
    };
    assert_eq!(tag, "thread.cleanupReview");
    assert_eq!(payload["threadId"], "u");
    reply.send_blocking(Ok(json!({"reviewId":"review-1","threadId":"u","title":"Other","actions":[
        {"id":"worktree","title":"Remove worktree","detail":"Dirty worktree","blockedReason":"Local modifications"},
        {"id":"archive","title":"Archive history","detail":"Keep in archived threads","blockedReason":null}
    ]}))).unwrap();
    cx.run_until_parked();
    assert!(cx.debug_bounds("sidebar-cleanup").is_some());
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            // Defense in depth: blocked IDs cannot be smuggled into a run.
            app.sidebar_state.cleanup.selected = vec!["worktree".into(), "archive".into()];
            app.review_sidebar_cleanup(true, cx);
        })
    });
    cx.run_until_parked();
    let connection::Command::Call {
        tag,
        payload,
        reply,
    } = requests.try_recv().unwrap()
    else {
        panic!("expected run");
    };
    assert_eq!(tag, "thread.cleanupRun");
    assert_eq!(payload["reviewId"], "review-1");
    assert_eq!(payload["selected"], json!(["archive"]));
    reply.send_blocking(Err("review expired".into())).unwrap();
    cx.run_until_parked();
    assert!(app.read_with(cx, |app, _| app.sidebar_state.cleanup.review.is_some()));
    assert_eq!(
        app.read_with(cx, |app, _| app.sidebar_state.cleanup.error.clone()),
        Some("review expired".into())
    );
    assert_eq!(
        app.read_with(cx, |app, _| app.selected_thread.clone()),
        Some("t".into())
    );
    let close = cx.debug_bounds("cleanup-close").unwrap();
    cx.simulate_click(close.center(), Default::default());
    cx.run_until_parked();
    assert!(app.read_with(cx, |app, _| app.sidebar_state.cleanup.target.is_none()));
}

#[gpui_kit::test]
fn sidebar_fixb_shelf_change_drops_hidden_selection_and_modifier_seed(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    seed_second_thread(&app, cx, false);
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.shell
                .threads
                .iter_mut()
                .find(|row| row.id == "u")
                .unwrap()
                .settled_override = Some("settled".into());
            app.shell.revision += 1;
            app.sidebar_state.selection = vec!["t".into()];
            app.sidebar_state.selection_anchor = Some("t".into());
            app.sidebar_state.selection_environment = app.environment_scope.clone();
            cx.notify();
        })
    });
    cx.run_until_parked();
    let settled = cx.debug_bounds("settled-view").unwrap();
    cx.simulate_click(settled.center(), Default::default());
    cx.run_until_parked();
    assert!(app.read_with(cx, |app, _| app.sidebar_state.selection.is_empty()));
    assert!(app.read_with(cx, |app, _| app.sidebar_state.selection_anchor.is_none()));
    let row = cx.debug_bounds("thread-u").unwrap();
    cx.simulate_click(
        row.center(),
        Modifiers {
            platform: true,
            ..Default::default()
        },
    );
    cx.run_until_parked();
    assert_eq!(
        app.read_with(cx, |app, _| app.sidebar_state.selection.clone()),
        vec!["u"]
    );
    // Stale callers cannot bypass the execution scope, even without a render.
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.sidebar_state.selection.push("t".into());
            app.bulk_thread_lifecycle("archive", "thread.archive", cx);
        })
    });
    cx.run_until_parked();
    let connection::Command::Call { payload, reply, .. } = requests.try_recv().unwrap() else {
        panic!("expected archive");
    };
    assert_eq!(
        payload["command"]["threadId"]
            .as_str()
            .or(payload["threadId"].as_str()),
        Some("u")
    );
    reply.send_blocking(Ok(json!({}))).unwrap();
    cx.run_until_parked();
    assert!(
        requests.try_recv().is_err(),
        "hidden active thread must not be archived"
    );
}

#[gpui_kit::test]
fn sidebar_fixb_priority_shelves_reveal_selected_and_collapse_reconciles(cx: &mut TestAppContext) {
    let (app, cx, _) = harness(cx);
    seed_second_thread(&app, cx, true);
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.collapsed_groups.push(sidebar::group_key("", "active"));
            app.sidebar_state.selection = vec!["t".into(), "u".into()];
            app.sidebar_state.selection_environment = app.environment_scope.clone();
            cx.notify();
        })
    });
    cx.run_until_parked();
    let priority = cx.debug_bounds("priority-view").unwrap();
    cx.simulate_click(priority.center(), Default::default());
    cx.run_until_parked();
    assert!(cx.debug_bounds("thread-t").is_some());
    assert_eq!(
        app.read_with(cx, |app, _| app.sidebar_selectable_order()),
        vec!["u", "t"]
    );
    let pinned = cx.debug_bounds("priority-pinned").unwrap();
    cx.simulate_click(pinned.center(), Default::default());
    cx.run_until_parked();
    assert_eq!(
        app.read_with(cx, |app, _| app.sidebar_state.selection.clone()),
        vec!["t"]
    );
    assert_eq!(
        app.read_with(cx, |app, _| app.selected_thread.clone()),
        Some("t".into())
    );
}

#[gpui_kit::test]
fn sidebar_fixb_auto_settle_override_is_row_targeted_and_receipt_authoritative(
    cx: &mut TestAppContext,
) {
    let (app, cx, requests) = harness(cx);
    seed_second_thread(&app, cx, false);
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.sidebar_state.capabilities = json!({"threadAutoSettleOptOut": true});
            cx.notify();
        })
    });
    cx.run_until_parked();
    right_click(cx, "thread-u");
    let button = cx.debug_bounds("auto-settle-disable").unwrap();
    cx.simulate_click(button.center(), Default::default());
    cx.run_until_parked();
    let connection::Command::Call { payload, reply, .. } = requests.try_recv().unwrap() else {
        panic!("expected override");
    };
    let command = payload.get("command").unwrap_or(&payload);
    assert_eq!(command["type"], "thread.auto-settle.set");
    assert_eq!(command["threadId"], "u");
    assert_eq!(command["enabled"], false);
    reply
        .send_blocking(Err("override rejected".into()))
        .unwrap();
    cx.run_until_parked();
    assert_eq!(
        app.read_with(cx, |app, _| app.selected_thread.clone()),
        Some("t".into())
    );
    assert!(app.read_with(cx, |app, _| {
        app.shell
            .threads
            .iter()
            .find(|row| row.id == "u")
            .unwrap()
            .auto_settle_disabled_at
            .is_none()
    }));
}
