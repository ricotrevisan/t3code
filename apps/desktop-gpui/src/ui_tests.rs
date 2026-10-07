//! Headless GPUI interactions: real input dispatch and asynchronous receipts, not markup assertions.
use super::*;
use gpui_kit::component::Root;
use gpui_kit::{AppContext, MouseButton, TestAppContext, VisualTestContext, point};

fn harness(
    cx: &mut TestAppContext,
) -> (
    Entity<NativeApp>,
    &mut VisualTestContext,
    async_channel::Receiver<connection::Command>,
) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        theme::init(cx);
        bind_native_keys(cx);
        cx.set_app_identity("codes.t3.native.test", "T3 Code Native");
    });
    let mut app = None;
    let (_, cx) = cx.add_window_view(|window, cx| {
        let entity = cx.new(|cx| NativeApp::new(window, cx, false));
        app = Some(entity.clone());
        Root::new(entity, window, cx)
    });
    cx.simulate_resize(size(px(1280.), px(1000.)));
    let app = app.unwrap();
    let (sender, requests) = async_channel::bounded(16);
    cx.update(|window,cx| app.update(cx,|app,cx| {
        app.shell.apply(serde_json::from_value(json!({"kind":"snapshot","snapshot":{"snapshotSequence":0,"projects":[{"id":"p","title":"Fixture","workspaceRoot":"/tmp/fixture"}],"threads":[{"id":"t","projectId":"p","title":"Thread","modelSelection":{"instanceId":"codex","model":"gpt-5.4"},"runtimeMode":"full-access","interactionMode":"default","archivedAt":null}]}})).unwrap()).unwrap();
        app.commands = Some(sender);
        app.select_thread("t",window,cx);
        app.composer.update(cx,|input,cx| input.focus(window,cx));
        cx.notify();
    }));
    // Selection request belongs to setup, not the action under test.
    assert!(
        matches!(requests.try_recv().unwrap(),connection::Command::Subscribe {tag,..} if tag=="subscribeWorktreeSetup")
    );
    assert!(
        matches!(requests.try_recv().unwrap(),connection::Command::SelectThread(Some(id)) if id == "t")
    );
    cx.run_until_parked();
    (app, cx, requests)
}

#[gpui_kit::test]
fn zed_one_dark_is_the_installed_theme(cx: &mut TestAppContext) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        theme::init(cx);
        assert_eq!(
            gpui_kit::component::Theme::global(cx)
                .theme_name()
                .to_string(),
            "One Dark"
        );
    });
}

#[gpui_kit::test]
fn runtime_and_interaction_mode_chips_dispatch_the_toggled_command(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.detail.thread = Some(json!({
                "id": "t",
                "title": "Thread",
                "runtimeMode": "full-access",
                "interactionMode": "default",
                "messages": [],
                "activities": [],
                "proposedPlans": [],
                "checkpoints": [],
            }));
            cx.notify();
        })
    });
    cx.run_until_parked();

    let bounds = cx
        .debug_bounds("runtime-mode")
        .expect("runtime mode chip is visible");
    cx.simulate_click(bounds.center(), Default::default());
    let connection::Command::Call { payload, .. } = requests.try_recv().unwrap() else {
        panic!("expected a runtime mode command");
    };
    assert_eq!(payload["type"], "thread.runtime-mode.set");
    assert_eq!(payload["threadId"], "t");
    assert_eq!(payload["runtimeMode"], "approval-required");

    let bounds = cx
        .debug_bounds("interaction-mode")
        .expect("interaction mode chip is visible");
    cx.simulate_click(bounds.center(), Default::default());
    let connection::Command::Call { payload, .. } = requests.try_recv().unwrap() else {
        panic!("expected an interaction mode command");
    };
    assert_eq!(payload["type"], "thread.interaction-mode.set");
    assert_eq!(payload["interactionMode"], "plan");
}

#[gpui_kit::test]
fn settings_surface_opens_and_closes(cx: &mut TestAppContext) {
    let (app, cx, _requests) = harness(cx);
    let bounds = cx
        .debug_bounds("settings-open")
        .expect("title bar settings button");
    cx.simulate_click(bounds.center(), Default::default());
    assert!(app.read_with(cx, |app, _| app.settings_open));
    cx.run_until_parked();
    let bounds = cx
        .debug_bounds("settings-close")
        .expect("settings close button");
    cx.simulate_click(bounds.center(), Default::default());
    assert!(!app.read_with(cx, |app, _| app.settings_open));
}

#[gpui_kit::test]
fn checkpoint_revert_arms_then_dispatches(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.detail.thread = Some(json!({
                "id": "t",
                "title": "Thread",
                "messages": [],
                "activities": [],
                "proposedPlans": [],
                "checkpoints": [{
                    "turnId": "turn-1",
                    "checkpointTurnCount": 2,
                    "checkpointRef": "ref",
                    "status": "ready",
                    "files": [{"path": "a.rs", "kind": "modified", "additions": 1, "deletions": 0}],
                    "assistantMessageId": null,
                    "completedAt": "2026-01-01T00:00:00Z"
                }],
                "session": null,
            }));
            cx.notify();
        })
    });
    cx.run_until_parked();

    let bounds = cx
        .debug_bounds("revert-checkpoint")
        .expect("revert button is visible");
    cx.simulate_click(bounds.center(), Default::default());
    assert!(requests.try_recv().is_err(), "first click only arms the revert");
    assert_eq!(app.read_with(cx, |app, _| app.confirm_revert), Some(2));

    let bounds = cx.debug_bounds("revert-checkpoint").unwrap();
    cx.simulate_click(bounds.center(), Default::default());
    let connection::Command::Call { payload, .. } = requests.try_recv().unwrap() else {
        panic!("expected a revert command");
    };
    assert_eq!(payload["type"], "thread.checkpoint.revert");
    assert_eq!(payload["turnCount"], 2);
}

#[gpui_kit::test]
fn each_turn_owns_its_activity_and_checkpoint_controls(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    let checkpoint = |turn: &str, count: u64, at: &str| {
        json!({
            "turnId": turn,
            "checkpointTurnCount": count,
            "checkpointRef": "ref",
            "status": "ready",
            "files": [{"path": "a.rs", "kind": "modified", "additions": 1, "deletions": 0}],
            "assistantMessageId": null,
            "completedAt": at
        })
    };
    let activity = |id: &str, turn: &str, at: &str| {
        json!({"id": id, "kind": "tool.completed", "tone": "tool", "summary": "Ran tests", "payload": {}, "turnId": turn, "createdAt": at})
    };
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.detail.thread = Some(json!({
                "id": "t",
                "title": "Thread",
                "messages": [
                    {"id": "u1", "role": "user", "text": "one", "turnId": null, "streaming": false, "createdAt": "2026-01-01T00:00:00Z", "updatedAt": "2026-01-01T00:00:00Z"},
                    {"id": "u2", "role": "user", "text": "two", "turnId": null, "streaming": false, "createdAt": "2026-01-01T00:01:00Z", "updatedAt": "2026-01-01T00:01:00Z"}
                ],
                "activities": [
                    activity("e1", "turn-1", "2026-01-01T00:00:05Z"),
                    activity("e2", "turn-2", "2026-01-01T00:01:05Z")
                ],
                "proposedPlans": [],
                "checkpoints": [
                    checkpoint("turn-1", 1, "2026-01-01T00:00:30Z"),
                    checkpoint("turn-2", 2, "2026-01-01T00:01:30Z")
                ],
                "latestTurn": {"turnId": "turn-2", "state": "completed", "requestedAt": "2026-01-01T00:01:00Z", "startedAt": "2026-01-01T00:01:00Z", "completedAt": "2026-01-01T00:01:30Z", "assistantMessageId": null},
                "session": null,
            }));
            cx.notify();
        })
    });
    cx.run_until_parked();
    assert!(cx.debug_bounds("activity-toggle").is_some());
    assert!(cx.debug_bounds("revert-checkpoint").is_some());

    let bounds = cx
        .debug_bounds("activity-toggle-turn-1")
        .expect("the first turn has its own activity toggle");
    cx.simulate_click(bounds.center(), Default::default());
    cx.run_until_parked();
    app.read_with(cx, |app, _| {
        assert_eq!(app.activity_expanded.get("turn-1"), Some(&true));
        assert_eq!(app.activity_expanded.get("turn-2"), None);
    });

    let bounds = cx
        .debug_bounds("revert-checkpoint-turn-1")
        .expect("the first turn keeps its own checkpoint");
    cx.simulate_click(bounds.center(), Default::default());
    assert_eq!(app.read_with(cx, |app, _| app.confirm_revert), Some(1));
    cx.run_until_parked();
    let bounds = cx.debug_bounds("revert-checkpoint").unwrap();
    cx.simulate_click(bounds.center(), Default::default());
    assert_eq!(app.read_with(cx, |app, _| app.confirm_revert), Some(2));
    assert!(requests.try_recv().is_err(), "arming never dispatches a revert");
}

#[gpui_kit::test]
fn tool_calls_render_as_rows_that_expand_one_at_a_time(cx: &mut TestAppContext) {
    let (app, cx, _requests) = harness(cx);
    let activity = |id: &str, kind: &str, tone: &str, summary: &str, at: &str, payload: Value| {
        json!({"id": id, "kind": kind, "tone": tone, "summary": summary, "payload": payload, "turnId": "turn-1", "createdAt": at})
    };
    let command = json!({"itemType": "command_execution", "toolCallId": "c1", "title": "Ran command", "data": {"item": {"command": "cargo test"}}});
    let mut command_done = command.clone();
    command_done["status"] = json!("completed");
    command_done["data"]["item"]["aggregatedOutput"] = json!("test result: ok");
    let edit = json!({"itemType": "file_change", "toolCallId": "f1", "title": "File change", "status": "failed", "detail": "patch rejected", "data": {"files": [{"path": "src/lib.rs"}]}});
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.detail.thread = Some(json!({
                "id": "t",
                "title": "Thread",
                "messages": [
                    {"id": "u1", "role": "user", "text": "test it", "turnId": null, "streaming": false, "createdAt": "2026-01-01T00:00:00Z", "updatedAt": "2026-01-01T00:00:00Z"}
                ],
                "activities": [
                    activity("e1", "tool.started", "tool", "Ran command started", "2026-01-01T00:00:01Z", command),
                    activity("e2", "runtime.warning", "info", "Retrying the provider", "2026-01-01T00:00:02Z", json!({})),
                    activity("e3", "tool.completed", "tool", "Ran command", "2026-01-01T00:00:03Z", command_done),
                    activity("e4", "tool.completed", "tool", "File change", "2026-01-01T00:00:04Z", edit)
                ],
                "proposedPlans": [],
                "checkpoints": [],
                "latestTurn": {"turnId": "turn-1", "state": "completed", "requestedAt": "2026-01-01T00:00:00Z", "startedAt": "2026-01-01T00:00:00Z", "completedAt": "2026-01-01T00:00:05Z", "assistantMessageId": null},
                "session": null,
            }));
            cx.notify();
        })
    });
    cx.run_until_parked();
    let bounds = cx.debug_bounds("activity-toggle").expect("the turn has its disclosure");
    cx.simulate_click(bounds.center(), Default::default());
    cx.run_until_parked();

    for id in ["tool-row-e1", "tool-name-e1", "tool-row-e4", "tool-name-e4", "activity-e2"] {
        assert!(cx.debug_bounds(id).is_some(), "{id} renders");
    }
    assert!(cx.debug_bounds("activity-e3").is_none(), "the completion merges into its call's row");
    let names: Vec<String> = app.read_with(cx, |app, _| {
        app.conversation_rows
            .iter()
            .flat_map(|row| match &row.entry {
                t3code_desktop_gpui::conversation::TimelineEntry::Activity { items, .. } => items.clone(),
                _ => Vec::new(),
            })
            .filter_map(|item| match item {
                t3code_desktop_gpui::conversation::ActivityItem::Tool(call) => Some(call.name),
                _ => None,
            })
            .collect()
    });
    assert_eq!(names, ["Ran command", "File change"]);
    assert!(cx.debug_bounds("tool-body-e1").is_none() && cx.debug_bounds("tool-body-e4").is_none());

    let bounds = cx.debug_bounds("tool-toggle-e1").expect("the command call can expand");
    cx.simulate_click(bounds.center(), Default::default());
    cx.run_until_parked();
    assert!(cx.debug_bounds("tool-body-e1").is_some(), "the command output shows");
    assert!(cx.debug_bounds("tool-body-e4").is_none(), "the other call stays collapsed");
    assert!(cx.debug_bounds("activity-e2").is_some(), "the plain activity still renders");
    app.read_with(cx, |app, _| {
        assert_eq!(app.activity_expanded.get("tool:e1"), Some(&true));
        assert_eq!(app.activity_expanded.get("turn-1"), Some(&true));
    });
}

#[gpui_kit::test]
fn open_in_editor_prefers_zed_and_targets_the_worktree(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.workspace
                .update(cx, |input, cx| input.set_value("/tmp/fixture", window, cx));
            app.available_editors = vec!["vscode".into(), "zed".into()];
        })
    });
    cx.run_until_parked();
    let bounds = cx.debug_bounds("open-editor").expect("open button is visible");
    cx.simulate_click(bounds.center(), Default::default());
    cx.run_until_parked();
    let connection::Command::Call { tag, payload, .. } = requests.try_recv().unwrap() else {
        panic!("expected an editor launch");
    };
    assert_eq!(tag, "shell.openInEditor");
    assert_eq!(payload["cwd"], "/tmp/fixture");
    assert_eq!(payload["editor"], "zed");
}

#[gpui_kit::test]
fn slash_menu_filters_and_executes_commands(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.composer
                .update(cx, |input, cx| input.set_value("/archive", window, cx));
            cx.notify();
        })
    });
    cx.run_until_parked();
    let bounds = cx
        .debug_bounds("slash-Archive thread")
        .expect("slash command entry is visible");
    cx.simulate_click(bounds.center(), Default::default());
    assert!(app.read_with(cx, |app, cx| app.composer.read(cx).value().is_empty()));
    let connection::Command::Call { payload, .. } = requests.try_recv().unwrap() else {
        panic!("expected an archive command");
    };
    assert_eq!(payload["type"], "thread.archive");
}

#[gpui_kit::test]
fn sidebar_search_runs_on_enter(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.search.update(cx, |input, cx| input.focus(window, cx));
        })
    });
    cx.simulate_input("current");
    cx.simulate_keystrokes("enter");
    let connection::Command::Call { tag, payload, .. } = requests.try_recv().unwrap() else {
        panic!("expected a thread search");
    };
    assert_eq!(tag, "orchestration.searchThreads");
    assert_eq!(payload["query"], "current");
}

#[gpui_kit::test]
fn right_panel_tabs_switch_tools(cx: &mut TestAppContext) {
    let (app, cx, _requests) = harness(cx);
    let changes = cx
        .debug_bounds("right-tab-changes")
        .expect("changes tab is visible");
    cx.simulate_click(changes.center(), Default::default());
    assert_eq!(app.read_with(cx, |app, _| app.right_tab), RightTab::Changes);
    cx.run_until_parked();
    assert!(cx.debug_bounds("changes-panel").is_some());

    let agents = cx
        .debug_bounds("right-tab-agents")
        .expect("agents tab is visible");
    cx.simulate_click(agents.center(), Default::default());
    assert_eq!(app.read_with(cx, |app, _| app.right_tab), RightTab::Agents);
    cx.run_until_parked();
    assert!(cx.debug_bounds("agents-panel").is_some());
}

#[gpui_kit::test]
fn sidebar_projects_collapse_and_expand(cx: &mut TestAppContext) {
    let (app, cx, _requests) = harness(cx);
    let toggle = cx
        .debug_bounds("project-toggle-p")
        .expect("project toggle is visible");
    cx.simulate_click(toggle.center(), Default::default());
    assert!(app.read_with(cx, |app, _| app
        .collapsed_projects
        .contains(&"p".to_owned())));
    let toggle = cx.debug_bounds("project-toggle-p").unwrap();
    cx.simulate_click(toggle.center(), Default::default());
    assert!(app.read_with(cx, |app, _| app.collapsed_projects.is_empty()));
}

#[gpui_kit::test]
fn pending_approval_renders_and_responds(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.detail.thread = Some(json!({
                "id": "t",
                "title": "Thread",
                "messages": [],
                "activities": [{
                    "id": "a1",
                    "kind": "approval.requested",
                    "tone": "approval",
                    "summary": "Command approval requested",
                    "payload": {
                        "requestId": "req-1",
                        "detail": "run rm -rf build",
                        "options": [
                            {"decision": "accept", "label": "Allow once"},
                            {"decision": "decline", "label": "Deny"}
                        ]
                    },
                    "createdAt": "2026-01-01T00:00:00Z",
                    "turnId": null
                }],
                "proposedPlans": [],
                "checkpoints": [],
                "session": null,
            }));
            cx.notify();
        })
    });
    cx.run_until_parked();
    let bounds = cx
        .debug_bounds("approval-accept")
        .expect("approval option is visible");
    cx.simulate_click(bounds.center(), Default::default());
    let connection::Command::Call { payload, .. } = requests.try_recv().unwrap() else {
        panic!("expected an approval response");
    };
    assert_eq!(payload["type"], "thread.approval.respond");
    assert_eq!(payload["requestId"], "req-1");
    assert_eq!(payload["decision"], "accept");
}

#[gpui_kit::test]
fn user_input_card_submits_answers_and_closes_once_resolved(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    let requested = json!({
        "id": "a1",
        "kind": "user-input.requested",
        "tone": "info",
        "summary": "User input requested",
        "payload": {"requestId": "req-1", "questions": [
            {"id": "scope", "header": "Scope", "question": "Which package?", "allowCustomAnswer": false, "options": [
                {"label": "Web", "description": "apps/web", "value": "web"},
                {"label": "Server", "description": "apps/server"}
            ]},
            {"id": "checks", "header": "Checks", "question": "What should run?", "multiSelect": true, "allowCustomAnswer": true, "options": [
                {"label": "Tests", "description": ""},
                {"label": "Lint", "description": ""},
                {"label": "Types", "description": ""}
            ]}
        ]},
        "createdAt": "2026-01-01T00:00:00Z",
        "turnId": null
    });
    cx.update(|_, cx| app.update(cx, |app, cx| {
        let item = json!({"kind":"snapshot","snapshot":{"snapshotSequence":1,"thread":{"id":"t","title":"Thread","messages":[],"activities":[requested],"proposedPlans":[],"checkpoints":[],"session":{"status":"running"}}}});
        app.apply_thread_update("t", item, cx).unwrap();
        cx.notify();
    }));
    cx.run_until_parked();
    assert!(cx.debug_bounds("user-input-custom-0").is_none(), "the single-select question forbids custom text");
    assert!(cx.debug_bounds("user-input-custom-1").is_some());
    assert!(cx.debug_bounds("user-input-dismiss").is_none(), "a native callback cannot be dismissed");
    for selector in ["user-input-option-0-0", "user-input-option-0-1", "user-input-option-1-0", "user-input-option-1-2", "user-input-option-1-1", "user-input-option-1-0"] {
        let bounds = cx.debug_bounds(selector).expect("option chip is visible");
        cx.simulate_click(bounds.center(), Default::default());
    }
    let submit = cx.debug_bounds("user-input-submit").expect("submit is visible");
    cx.simulate_click(submit.center(), Default::default());
    let connection::Command::Call { payload, .. } = requests.try_recv().unwrap() else {
        panic!("expected a user-input response");
    };
    assert_eq!(payload["type"], "thread.user-input.respond");
    assert_eq!(payload["requestId"], "req-1");
    assert_eq!(payload["answers"], json!({"scope": "Server", "checks": ["Types", "Lint"]}));

    let custom = cx.debug_bounds("user-input-custom-1").unwrap();
    cx.simulate_click(custom.center(), Default::default());
    cx.simulate_input("Benchmarks");
    cx.simulate_click(submit.center(), Default::default());
    let connection::Command::Call { payload, .. } = requests.try_recv().unwrap() else {
        panic!("expected a user-input response");
    };
    assert_eq!(payload["answers"], json!({"scope": "Server", "checks": "Benchmarks"}));
    assert!(
        app.read_with(cx, |app, _| !app.user_input_selected.contains_key("checks")),
        "typed text replaces the chosen chips, so they no longer show selected"
    );

    cx.update(|window, cx| app.update(cx, |app, cx| {
        app.composer.update(cx, |input, cx| input.set_value("Draft ", window, cx));
    }));
    let tests = cx.debug_bounds("user-input-option-1-0").unwrap();
    cx.simulate_click(tests.center(), Default::default());
    assert_eq!(
        app.read_with(cx, |app, cx| app.composer.read(cx).value().to_string()),
        "Draft\n\nBenchmarks",
        "a chip click carries the displaced custom answer into the composer"
    );
    cx.simulate_click(submit.center(), Default::default());
    let connection::Command::Call { payload, .. } = requests.try_recv().unwrap() else {
        panic!("expected a user-input response");
    };
    assert_eq!(payload["answers"], json!({"scope": "Server", "checks": ["Tests"]}));

    cx.update(|_, cx| app.update(cx, |app, cx| {
        let resolved = json!({"id":"a2","kind":"user-input.resolved","tone":"info","summary":"User input submitted","payload":{"requestId":"req-1","answers":{}},"createdAt":"2026-01-01T00:00:05Z","turnId":null});
        let item = json!({"kind":"event","event":{"sequence":2,"type":"thread.activity-appended","occurredAt":"2026-01-01T00:00:05Z","payload":{"threadId":"t","activity":resolved}}});
        app.apply_thread_update("t", item, cx).unwrap();
        cx.notify();
    }));
    cx.run_until_parked();
    assert!(cx.debug_bounds("user-input-card").is_none());
    assert!(cx.debug_bounds("user-input-submit").is_none());
}

#[gpui_kit::test]
fn push_and_create_pr_streams_the_stacked_action(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.workspace
                .update(cx, |input, cx| input.set_value("/tmp/fixture", window, cx));
        })
    });
    cx.run_until_parked();
    let bounds = cx
        .debug_bounds("push-pr")
        .expect("push-pr button is visible");
    cx.simulate_click(bounds.center(), Default::default());
    cx.run_until_parked();
    let connection::Command::CallStream { tag, payload, .. } = requests.try_recv().unwrap() else {
        panic!("expected a streamed git action");
    };
    assert_eq!(tag, "git.runStackedAction");
    assert_eq!(payload["action"], "commit_push_pr");
    assert_eq!(payload["cwd"], "/tmp/fixture");
}

#[test]
fn attachment_mime_maps_common_extensions() {
    assert_eq!(attachment_mime("a.PNG"), "image/png");
    assert_eq!(attachment_mime("b.jpeg"), "image/jpeg");
    assert_eq!(attachment_mime("notes.md"), "text/markdown");
    assert_eq!(attachment_mime("archive.bin"), "application/octet-stream");
}

#[gpui_kit::test]
fn attachments_render_and_detach(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.attachments.push(
                json!({"id": "a1", "name": "notes.md", "mimeType": "text/markdown", "sizeBytes": 12}),
            );
            cx.notify();
        })
    });
    cx.run_until_parked();
    let bounds = cx.debug_bounds("detach-0").expect("attachment chip is visible");
    cx.simulate_click(bounds.center(), Default::default());
    assert!(app.read_with(cx, |app, _| app.attachments.is_empty()));
    let connection::Command::Call { payload, .. } = requests.try_recv().unwrap() else {
        panic!("expected an attachment delete");
    };
    assert_eq!(payload["attachmentId"], "a1");
}

#[gpui_kit::test]
fn sidebar_thread_groups_collapse_per_project(cx: &mut TestAppContext) {
    let (app, cx, _requests) = harness(cx);
    cx.update(|_, cx| app.update(cx, |app, cx| {
        app.shell.apply(serde_json::from_value(json!({"kind":"snapshot","snapshot":{"snapshotSequence":1,"projects":[{"id":"p","title":"Fixture","workspaceRoot":"/tmp/fixture"},{"id":"q","title":"Other","workspaceRoot":"/tmp/other"}],"threads":[{"id":"t","projectId":"p","title":"Thread","modelSelection":{"instanceId":"codex","model":"gpt-5.4"},"archivedAt":null},{"id":"u","projectId":"q","title":"Elsewhere","modelSelection":{"instanceId":"codex","model":"gpt-5.4"},"archivedAt":null}]}})).unwrap()).unwrap();
        cx.notify();
    }));
    cx.run_until_parked();
    let toggle = cx
        .debug_bounds("group-toggle-p:active")
        .expect("active group toggle is visible");
    cx.simulate_click(toggle.center(), Default::default());
    assert_eq!(
        app.read_with(cx, |app, _| app.collapsed_groups.clone()),
        vec!["p:active".to_owned()]
    );
    // Folding one project's group must not hide the other project's rows from navigation.
    cx.simulate_keystrokes("cmd-shift-]");
    assert_eq!(
        app.read_with(cx, |app, _| app.selected_thread.clone()),
        Some("u".into())
    );
    assert!(cx.debug_bounds("group-toggle-q:active").is_some());
}

fn message(index: usize) -> Value {
    json!({"id":format!("m{index}"),"role":"assistant","text":format!("Message {index}\n\nbody"),"createdAt":"2026-10-06T10:00:00Z"})
}

fn snapshot(messages: Vec<Value>, status: &str) -> Value {
    json!({"kind":"snapshot","snapshot":{"snapshotSequence":1,"thread":{"id":"t","title":"Thread","messages":messages,"session":{"status":status}}}})
}

#[gpui_kit::test]
fn conversation_follows_the_tail_until_the_user_scrolls_up(cx: &mut TestAppContext) {
    let (app, cx, _requests) = harness(cx);
    let load = |count: usize, cx: &mut VisualTestContext| {
        cx.update(|_, cx| app.update(cx, |app, cx| {
            app.apply_thread_update("t", snapshot((0..count).map(message).collect(), "idle"), cx)
                .unwrap();
            cx.notify();
        }));
        cx.run_until_parked();
        app.read_with(cx, |app, _| {
            let list = &app.conversation_scroll;
            (list.scroll_px_offset_for_scrollbar().y, list.max_offset_for_scrollbar().y)
        })
    };
    let (offset, max) = load(60, cx);
    assert!(max > px(0.), "fixture must overflow the viewport");
    assert_eq!(offset, -max, "opening a thread lands on the newest message");
    let (offset, max) = load(80, cx);
    assert_eq!(offset, -max, "new content keeps the tail in view");
    // Reading history: later updates must not yank the viewport.
    app.read_with(cx, |app, _| app.conversation_scroll.scroll_to(Default::default()));
    let (offset, _) = load(90, cx);
    assert_eq!(offset, px(0.));
}

#[gpui_kit::test]
fn conversation_tail_survives_streaming_and_thread_switches(cx: &mut TestAppContext) {
    let (app, cx, _requests) = harness(cx);
    let tail = |cx: &mut VisualTestContext| {
        app.read_with(cx, |app, _| {
            let list = &app.conversation_scroll;
            (list.scroll_px_offset_for_scrollbar().y, list.max_offset_for_scrollbar().y)
        })
    };
    let load = |thread: &str, messages: Vec<Value>, cx: &mut VisualTestContext| {
        cx.update(|_, cx| app.update(cx, |app, cx| {
            let item = json!({"kind":"snapshot","snapshot":{"snapshotSequence":1,"thread":{"id":thread,"title":"Thread","messages":messages,"session":{"status":"running"}}}});
            app.apply_thread_update(thread, item, cx).unwrap();
            cx.notify();
        }));
        cx.run_until_parked();
    };
    let mut messages: Vec<Value> = (0..60).map(message).collect();
    load("t", messages.clone(), cx);
    let (_, before) = tail(cx);
    messages[59]["text"] = json!("streamed line\n\n".repeat(40));
    messages[59]["streaming"] = json!(true);
    load("t", messages, cx);
    let (offset, max) = tail(cx);
    assert!(max > before, "the streamed message grew");
    assert_eq!(offset, -max, "streaming growth keeps the tail in view");

    app.read_with(cx, |app, _| app.conversation_scroll.scroll_to(Default::default()));
    cx.update(|window, cx| app.update(cx, |app, cx| {
        app.shell.apply(serde_json::from_value(json!({"kind":"snapshot","snapshot":{"snapshotSequence":1,"projects":[{"id":"p","title":"Fixture","workspaceRoot":"/tmp/fixture"}],"threads":[{"id":"t","projectId":"p","title":"Thread","modelSelection":{"instanceId":"codex","model":"gpt-5.4"},"archivedAt":null},{"id":"u","projectId":"p","title":"Other","modelSelection":{"instanceId":"codex","model":"gpt-5.4"},"archivedAt":null}]}})).unwrap()).unwrap();
        app.select_thread("u", window, cx);
    }));
    load("u", (0..60).map(message).collect(), cx);
    let (offset, max) = tail(cx);
    assert!(max > px(0.));
    assert_eq!(offset, -max, "switching threads lands on the new thread's tail");
}

#[gpui_kit::test]
fn long_conversations_only_build_rows_near_the_viewport(cx: &mut TestAppContext) {
    let (app, cx, _requests) = harness(cx);
    cx.update(|_, cx| app.update(cx, |app, cx| {
        app.apply_thread_update("t", snapshot((0..2000).map(message).collect(), "idle"), cx)
            .unwrap();
        cx.notify();
    }));
    cx.run_until_parked();
    let ids: Vec<&'static str> = (0..2000)
        .map(|index| &*format!("message-m{index}").leak())
        .collect();
    let built = |cx: &mut VisualTestContext| {
        ids.iter()
            .filter(|id| cx.debug_bounds(id).is_some())
            .count()
    };
    assert!(cx.debug_bounds("message-m1999").is_some(), "the newest message is built");
    assert!(cx.debug_bounds("message-m1000").is_none());
    assert!(cx.debug_bounds("message-m0").is_none());
    let tail_rows = built(cx);
    assert!((1..50).contains(&tail_rows), "{tail_rows} rows built at the tail");

    app.read_with(cx, |app, _| app.conversation_scroll.scroll_to(Default::default()));
    cx.update(|_, cx| app.update(cx, |_, cx| cx.notify()));
    cx.run_until_parked();
    assert!(cx.debug_bounds("message-m0").is_some(), "the oldest message is built");
    assert!(cx.debug_bounds("message-m1000").is_none());
    assert!(cx.debug_bounds("message-m1999").is_none());
    let head_rows = built(cx);
    assert!((1..50).contains(&head_rows), "{head_rows} rows built at the top");
}

#[gpui_kit::test]
fn enter_sends_and_shift_enter_inserts_a_newline(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    cx.simulate_input("first");
    cx.simulate_keystrokes("shift-enter");
    cx.simulate_input("second");
    assert!(requests.try_recv().is_err());
    cx.simulate_keystrokes("enter");
    let connection::Command::Call { payload, reply, .. } = requests.try_recv().unwrap() else {
        panic!("expected turn start");
    };
    assert_eq!(payload["message"]["text"], "first\nsecond");
    reply.send_blocking(Ok(json!({"sequence":1}))).unwrap();
    cx.run_until_parked();
    assert_eq!(
        app.read_with(cx, |app, cx| app.composer.read(cx).value().to_string()),
        ""
    );
}

#[gpui_kit::test]
fn running_turn_swaps_send_for_stop(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    assert!(cx.debug_bounds("send").is_some());
    assert!(cx.debug_bounds("stop-turn").is_none());
    cx.update(|_, cx| app.update(cx, |app, cx| {
        app.apply_thread_update("t", snapshot(vec![message(0)], "running"), cx).unwrap();
        cx.notify();
    }));
    cx.run_until_parked();
    assert!(cx.debug_bounds("send").is_none());
    let stop = cx.debug_bounds("stop-turn").expect("Stop replaces Send");
    cx.simulate_click(stop.center(), Default::default());
    let connection::Command::Call { payload, .. } = requests.try_recv().unwrap() else {
        panic!("expected interrupt");
    };
    assert_eq!(payload["type"], "thread.turn.interrupt");
}

#[gpui_kit::test]
fn escape_closes_the_topmost_overlay_and_thread_menu_tracks_state(cx: &mut TestAppContext) {
    let (app, cx, _requests) = harness(cx);
    let trigger = cx.debug_bounds("thread-menu").unwrap();
    cx.simulate_click(trigger.center(), Default::default());
    cx.run_until_parked();
    // The fixture thread is unpinned and active: only the forward actions are offered.
    let pin = cx.debug_bounds("pin").expect("Pin is offered");
    assert!(cx.debug_bounds("unpin").is_none());
    assert!(cx.debug_bounds("unarchive").is_none());
    // The menu hangs below its trigger instead of the window's left edge.
    assert!(pin.top() >= trigger.bottom());
    assert!(pin.left() >= trigger.left() - px(1.));
    // Settings covers the window with a backdrop, so it opens after the menu.
    cx.simulate_keystrokes("cmd-,");
    assert!(app.read_with(cx, |app, _| app.settings_open && app.thread_menu_open));
    cx.simulate_keystrokes("escape");
    assert!(app.read_with(cx, |app, _| !app.thread_menu_open && app.settings_open));
    cx.simulate_keystrokes("escape");
    assert!(app.read_with(cx, |app, _| !app.settings_open));
    cx.simulate_keystrokes("cmd-alt-b");
    assert!(app.read_with(cx, |app, _| !app.inspector_open));
    cx.simulate_keystrokes("cmd-alt-b");
    assert!(app.read_with(cx, |app, _| app.inspector_open));
}

#[gpui_kit::test]
fn palette_archive_emits_same_command_and_escape_restores_composer_focus(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    cx.simulate_keystrokes("cmd-k");
    cx.simulate_input("archive");
    cx.simulate_keystrokes("enter");
    let connection::Command::Call {
        tag,
        payload,
        reply,
    } = requests.try_recv().unwrap()
    else {
        panic!("expected archive mutation");
    };
    assert_eq!(tag, "orchestration.dispatchCommand");
    assert_eq!(payload["type"], "thread.archive");
    assert_eq!(payload["threadId"], "t");
    reply.send_blocking(Ok(json!({"sequence":1}))).unwrap();
    cx.run_until_parked();
    assert!(!app.read_with(cx, |app, _| app.palette_open));
    cx.simulate_keystrokes("cmd-k escape");
    cx.simulate_input("still the composer");
    assert_eq!(
        app.read_with(cx, |app, cx| app.composer.read(cx).value().to_string()),
        "still the composer"
    );
}

#[gpui_kit::test]
fn palette_arrows_choose_reverse_action_and_pin_shortcut_tracks_server_state(
    cx: &mut TestAppContext,
) {
    let (app, cx, requests) = harness(cx);
    cx.simulate_keystrokes("cmd-k");
    cx.simulate_input("archive");
    cx.simulate_keystrokes("down enter");
    let connection::Command::Call { payload, reply, .. } = requests.try_recv().unwrap() else {
        panic!("expected reverse action");
    };
    assert_eq!(payload["type"], "thread.unarchive");
    reply.send_blocking(Err("already active".into())).unwrap();
    cx.run_until_parked();
    assert_eq!(
        app.read_with(cx, |app, _| app.status.clone()),
        "already active"
    );
    cx.simulate_keystrokes("cmd-shift-p");
    let connection::Command::Call { payload, reply, .. } = requests.try_recv().unwrap() else {
        panic!("expected pin");
    };
    assert_eq!(payload["type"], "thread.pin");
    reply.send_blocking(Ok(json!({"sequence":1}))).unwrap();
    cx.run_until_parked();
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            let mut pinned = app.shell.threads[0].clone();
            pinned.pinned_at = Some("2026-01-01T00:00:00Z".into());
            app.shell
                .apply(shell::ShellItem::ThreadUpserted {
                    sequence: 1,
                    thread: pinned,
                })
                .unwrap();
            cx.notify();
        })
    });
    cx.simulate_keystrokes("cmd-shift-p");
    let connection::Command::Call { payload, reply, .. } = requests.try_recv().unwrap() else {
        panic!("expected unpin");
    };
    assert_eq!(payload["type"], "thread.unpin");
    reply.send_blocking(Ok(json!({"sequence":2}))).unwrap();
    cx.run_until_parked();
}

#[gpui_kit::test]
fn palette_deletion_still_requires_confirmation_and_shortcut_stash_preserves_text(
    cx: &mut TestAppContext,
) {
    let (app, cx, requests) = harness(cx);
    cx.simulate_input("preserve 🌍");
    cx.simulate_keystrokes("cmd-s");
    assert_eq!(
        app.read_with(cx, |app, cx| app.composer.read(cx).value().to_string()),
        ""
    );
    assert_eq!(
        app.read_with(cx, |app, _| app.stash_memory.clone()),
        vec!["preserve 🌍"]
    );
    cx.simulate_keystrokes("cmd-k");
    cx.simulate_input("delete");
    cx.simulate_keystrokes("enter");
    assert!(
        requests.try_recv().is_err(),
        "first activation must not delete"
    );
    cx.simulate_keystrokes("cmd-k");
    cx.simulate_input("delete");
    cx.simulate_keystrokes("enter");
    let connection::Command::Call { payload, reply, .. } = requests.try_recv().unwrap() else {
        panic!("expected confirmed deletion");
    };
    assert_eq!(payload["type"], "thread.delete");
    reply
        .send_blocking(Err("owned fixture rejection".into()))
        .unwrap();
    cx.run_until_parked();
}

#[gpui_kit::test]
fn model_change_restriction_cannot_be_bypassed_by_manual_ids(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    cx.update(|window,cx|app.update(cx,|app,cx| {
        app.provider_catalog=serde_json::from_value(json!([{"instanceId":"codex","driver":"codex","enabled":true,"installed":true,"status":"ready","requiresNewThreadForModelChange":true,"models":[{"slug":"other","name":"Other"}]}])).unwrap();
        app.detail.thread=Some(json!({"modelSelection":{"instanceId":"codex","model":"gpt-5.4"},"messages":[{"role":"user","text":"existing"}]}));
        app.model.update(cx,|input,cx|input.set_value("other",window,cx));
        app.composer.update(cx,|input,cx|input.set_value("keep this draft",window,cx));
        app.send_prompt(window,cx);
    }));
    cx.run_until_parked();
    assert!(requests.try_recv().is_err());
    assert_eq!(
        app.read_with(cx, |app, _| app.status.clone()),
        "This provider requires a new thread for a model change"
    );
    assert_eq!(
        app.read_with(cx, |app, cx| app.composer.read(cx).value().to_string()),
        "keep this draft"
    );
}

#[gpui_kit::test]
fn choosing_catalog_model_uses_instance_routing_and_preserves_unchanged_options(
    cx: &mut TestAppContext,
) {
    let (app, cx, requests) = harness(cx);
    cx.update(|_,cx|app.update(cx,|app,cx| {
        app.shell.threads[0].model_selection=json!({"instanceId":"codex","model":"gpt-5.4","options":[{"id":"reasoningEffort","value":"high"}]});
        app.provider_catalog=serde_json::from_value(json!([{"instanceId":"codex","driver":"codex","enabled":true,"installed":true,"status":"ready","models":[{"slug":"gpt-5.4","name":"Current model"}]}])).unwrap();app.catalog_open=true;cx.notify();
    }));
    cx.run_until_parked();
    let _ = cx.debug_bounds("model-catalog").unwrap();
    cx.simulate_event(gpui_kit::ScrollWheelEvent {
        position: gpui_kit::point(px(1100.), px(50.)),
        delta: gpui_kit::ScrollDelta::Pixels(gpui_kit::point(px(0.), px(-400.))),
        modifiers: Default::default(),
        touch_phase: gpui_kit::TouchPhase::Moved,
    });
    cx.run_until_parked();
    let bounds = cx.debug_bounds("catalog-codex-0").unwrap();
    cx.simulate_click(bounds.center(), Default::default());
    assert!(!app.read_with(cx, |app, _| app.catalog_open));
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.composer.update(cx, |input, cx| {
                input.set_value("Prompt", window, cx);
                input.focus(window, cx);
            })
        })
    });
    cx.run_until_parked();
    let bounds = cx.debug_bounds("send").unwrap();
    cx.simulate_click(bounds.center(), Default::default());
    let connection::Command::Call { payload, reply, .. } = requests.try_recv().unwrap() else {
        panic!("expected turn start");
    };
    assert_eq!(payload["modelSelection"]["instanceId"], "codex");
    assert_eq!(payload["modelSelection"]["options"][0]["value"], "high");
    reply
        .send_blocking(Err("owned provider fixture".into()))
        .unwrap();
    cx.run_until_parked();
}

#[gpui_kit::test]
fn workspace_browse_and_path_search_use_server_and_ignore_other_project_results(
    cx: &mut TestAppContext,
) {
    let (app, cx, requests) = harness(cx);
    cx.update(|_, cx| app.update(cx, |app, cx| app.list_files(false, cx)));
    cx.run_until_parked();
    let connection::Command::Call {
        tag,
        payload,
        reply,
    } = requests.try_recv().unwrap()
    else {
        panic!("list request");
    };
    assert_eq!(tag, "projects.listEntries");
    assert_eq!(payload["directoryPath"], "");
    assert_eq!(payload["cwd"], "/tmp/fixture");
    reply.send_blocking(Ok(json!({"entries":[{"path":"src","kind":"directory"},{"path":"README.md","kind":"file"}],"truncated":false}))).unwrap();
    cx.run_until_parked();
    assert_eq!(app.read_with(cx, |app, _| app.file_entries.len()), 2);
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.file_path
                .update(cx, |input, cx| input.set_value("README", window, cx));
            app.list_files(true, cx);
        })
    });
    cx.run_until_parked();
    let connection::Command::Call {
        tag,
        payload,
        reply,
    } = requests.try_recv().unwrap()
    else {
        panic!("search request");
    };
    assert_eq!(tag, "projects.searchEntries");
    assert_eq!(payload["query"], "README");
    assert_eq!(payload["limit"], 100);
    cx.update(|_, cx| {
        app.update(cx, |app, _| {
            app.selected_project = Some("other-project".into())
        })
    });
    reply
        .send_blocking(Ok(json!({"entries":[],"truncated":true})))
        .unwrap();
    cx.run_until_parked();
    assert_eq!(
        app.read_with(cx, |app, _| app.file_entries.len()),
        2,
        "outdated listing must not replace current results"
    );
}

#[gpui_kit::test]
fn loaded_file_editor_accepts_real_input_dispatch(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.inspector_section = Some(InspectorSection::Files);
            app.file_path
                .update(cx, |input, cx| input.set_value("notes.txt", window, cx));
            app.file_operation(false, window, cx);
        })
    });
    cx.run_until_parked();
    let connection::Command::Call { reply, .. } = requests.try_recv().unwrap() else {
        panic!("expected read");
    };
    reply.send_blocking(Ok(json!({"relativePath":"notes.txt","contents":"original","byteLength":8,"truncated":false}))).unwrap();
    cx.run_until_parked();
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.file_contents
                .update(cx, |input, cx| input.focus(window, cx))
        })
    });
    cx.simulate_keystrokes("cmd-a");
    cx.simulate_input("世界 native save");
    assert_eq!(
        app.read_with(cx, |app, cx| app.file_contents.read(cx).value().to_string()),
        "世界 native save"
    );
}

#[gpui_kit::test]
fn file_save_uses_loaded_target_preserves_later_edits_and_failed_reads(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.inspector_section = Some(InspectorSection::Files);
            app.file_path
                .update(cx, |input, cx| input.set_value("notes.txt", window, cx));
            app.file_operation(false, window, cx);
        })
    });
    cx.run_until_parked();
    let connection::Command::Call {
        tag,
        payload,
        reply,
    } = requests.try_recv().unwrap()
    else {
        panic!("read request");
    };
    assert_eq!(tag, "projects.readFile");
    assert_eq!(payload["cwd"], "/tmp/fixture");
    reply.send_blocking(Ok(json!({"relativePath":"notes.txt","contents":"original","byteLength":8,"truncated":false}))).unwrap();
    cx.run_until_parked();
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.file_contents
                .update(cx, |input, cx| input.set_value("edited", window, cx));
            app.file_path
                .update(cx, |input, cx| input.set_value("different.txt", window, cx));
            app.file_operation(true, window, cx);
        })
    });
    cx.run_until_parked();
    let connection::Command::Call {
        tag,
        payload,
        reply,
    } = requests.try_recv().unwrap()
    else {
        panic!("save request");
    };
    assert_eq!(tag, "projects.writeFile");
    assert_eq!(payload["relativePath"], "notes.txt");
    assert_eq!(payload["contents"], "edited");
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.file_contents
                .update(cx, |input, cx| input.set_value("later edit", window, cx))
        })
    });
    reply
        .send_blocking(Ok(json!({"relativePath":"notes.txt"})))
        .unwrap();
    cx.run_until_parked();
    assert!(app.read_with(cx, |app, cx| {
        app.file_editor.dirty(&app.file_contents.read(cx).value())
    }));
    cx.update(|window, cx| app.update(cx, |app, cx| app.file_operation(false, window, cx)));
    cx.run_until_parked();
    assert!(
        requests.try_recv().is_err(),
        "reading must not discard unsaved edits"
    );
}

#[gpui_kit::test]
fn project_removal_requires_confirmation_and_rejection_preserves_navigation(
    cx: &mut TestAppContext,
) {
    let (app, cx, requests) = harness(cx);
    let bounds = cx.debug_bounds("remove-project").unwrap();
    cx.simulate_click(bounds.center(), Default::default());
    assert!(requests.try_recv().is_err());
    cx.simulate_click(bounds.center(), Default::default());
    let connection::Command::Call { payload, reply, .. } = requests.try_recv().unwrap() else {
        panic!("expected project removal");
    };
    assert_eq!(payload["type"], "project.delete");
    assert_eq!(payload["projectId"], "p");
    assert!(payload.get("force").is_none());
    reply
        .send_blocking(Err("Project has running sessions".into()))
        .unwrap();
    cx.run_until_parked();
    assert_eq!(
        app.read_with(cx, |app, _| app.selected_project.clone()),
        Some("p".into())
    );
    assert_eq!(app.read_with(cx, |app, _| app.shell.projects.len()), 1);
    assert_eq!(
        app.read_with(cx, |app, _| app.status.clone()),
        "Project has running sessions"
    );
}

#[gpui_kit::test]
fn changing_server_cannot_reuse_another_origins_access_credential(cx: &mut TestAppContext) {
    let (app, cx, _) = harness(cx);
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.use_dev_credential = false;
            app.access = Some(auth::Access {
                origin: "http://localhost:1234".into(),
                token: "private-first-origin".into(),
                proof_key: None,
            });
            app.server.update(cx, |input, cx| {
                input.set_value("http://localhost:5678", window, cx)
            });
            app.connect(cx);
            assert!(app.commands.is_none());
            assert!(app.selected_thread.is_none());
            assert_eq!(app.status, "Enter the server origin and pair to connect");
        })
    });
}

#[gpui_kit::test]
fn unicode_multiline_prompt_clears_only_after_accepted_receipt(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    cx.simulate_input("Hello 🌍\n世界");
    let before = app.read_with(cx, |app, cx| app.composer.read(cx).value().to_string());
    assert_eq!(before, "Hello 🌍\n世界");
    let bounds = cx.debug_bounds("send").expect("visible Send button");
    cx.simulate_click(bounds.center(), Default::default());
    let connection::Command::Call {
        tag,
        payload,
        reply,
    } = requests.try_recv().unwrap()
    else {
        panic!("expected turn start");
    };
    assert_eq!(tag, "orchestration.dispatchCommand");
    assert_eq!(payload["message"]["text"], before);
    assert_eq!(
        app.read_with(cx, |app, cx| app.composer.read(cx).value().to_string()),
        before
    );
    reply.send_blocking(Ok(json!({"sequence":1}))).unwrap();
    cx.run_until_parked();
    assert_eq!(
        app.read_with(cx, |app, cx| app.composer.read(cx).value().to_string()),
        ""
    );
}

#[gpui_kit::test]
fn rejected_send_preserves_user_prompt(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    cx.simulate_input("Do not lose this prompt");
    let bounds = cx.debug_bounds("send").unwrap();
    cx.simulate_click(bounds.center(), Default::default());
    let connection::Command::Call { reply, .. } = requests.try_recv().unwrap() else {
        panic!("expected call");
    };
    reply
        .send_blocking(Err("Provider rejected request".into()))
        .unwrap();
    cx.run_until_parked();
    assert_eq!(
        app.read_with(cx, |app, cx| app.composer.read(cx).value().to_string()),
        "Do not lose this prompt"
    );
    assert_eq!(app.read_with(cx, |app, _| app.pending_actions), 0);
    assert_eq!(
        app.read_with(cx, |app, _| app.status.clone()),
        "Provider rejected request"
    );
}

#[gpui_kit::test]
fn editing_while_send_is_pending_does_not_get_erased(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    cx.simulate_input("First prompt");
    let bounds = cx.debug_bounds("send").unwrap();
    cx.simulate_click(bounds.center(), Default::default());
    let connection::Command::Call { reply, .. } = requests.try_recv().unwrap() else {
        panic!("expected call");
    };
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.composer.update(cx, |input, cx| input.focus(window, cx))
        })
    });
    cx.simulate_input(" plus a later edit");
    reply.send_blocking(Ok(json!({"sequence":1}))).unwrap();
    cx.run_until_parked();
    assert_eq!(
        app.read_with(cx, |app, cx| app.composer.read(cx).value().to_string()),
        "First prompt plus a later edit"
    );
}

#[gpui_kit::test]
fn delete_requires_confirmation_before_any_request(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    let menu = cx.debug_bounds("thread-menu").unwrap();
    cx.simulate_click(menu.center(), Default::default());
    cx.run_until_parked();
    let bounds = cx.debug_bounds("delete").unwrap();
    cx.simulate_click(bounds.center(), Default::default());
    assert!(requests.try_recv().is_err());
    assert_eq!(
        app.read_with(cx, |app, _| app.confirm_delete.clone()),
        Some("t".into())
    );
    cx.simulate_click(bounds.center(), Default::default());
    let connection::Command::Call { payload, reply, .. } = requests.try_recv().unwrap() else {
        panic!("expected delete request");
    };
    assert_eq!(payload["type"], "thread.delete");
    reply
        .send_blocking(Err("Fixture refuses deletion".into()))
        .unwrap();
    cx.run_until_parked();
}

#[gpui_kit::test]
fn clipboard_roundtrip_preserves_multiline_unicode(cx: &mut TestAppContext) {
    let (app, cx, _requests) = harness(cx);
    cx.write_to_clipboard(gpui_kit::ClipboardItem::new_string("Hello 🌍\n世界".into()));
    cx.simulate_keystrokes("cmd-v");
    assert_eq!(
        app.read_with(cx, |app, cx| app.composer.read(cx).value().to_string()),
        "Hello 🌍\n世界"
    );
    cx.simulate_keystrokes("cmd-a cmd-c");
    assert_eq!(
        cx.read_from_clipboard().unwrap().text().unwrap(),
        "Hello 🌍\n世界"
    );
    cx.simulate_keystrokes("backspace");
    assert_eq!(
        app.read_with(cx, |app, cx| app.composer.read(cx).value().to_string()),
        ""
    );
}

#[gpui_kit::test]
fn stashing_and_restoring_preserves_prompt_without_overwriting_edits(cx: &mut TestAppContext) {
    let (app, cx, _requests) = harness(cx);
    cx.simulate_input("Saved prompt");
    let stash = cx.debug_bounds("stash").unwrap();
    cx.simulate_click(stash.center(), Default::default());
    assert_eq!(
        app.read_with(cx, |app, cx| app.composer.read(cx).value().to_string()),
        ""
    );
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.composer.update(cx, |input, cx| input.focus(window, cx))
        })
    });
    cx.simulate_input("Keep this edit");
    let restore = cx.debug_bounds("restore-stash").unwrap();
    cx.simulate_click(restore.center(), Default::default());
    assert_eq!(
        app.read_with(cx, |app, cx| app.composer.read(cx).value().to_string()),
        "Keep this edit"
    );
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.composer
                .update(cx, |input, cx| input.set_value("", window, cx))
        })
    });
    cx.simulate_click(restore.center(), Default::default());
    assert_eq!(
        app.read_with(cx, |app, cx| app.composer.read(cx).value().to_string()),
        "Saved prompt"
    );
}

#[gpui_kit::test]
fn snoozed_and_settled_views_have_a_way_back_to_active(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    let bounds = cx.debug_bounds("snoozed-view").unwrap();
    cx.simulate_click(bounds.center(), Default::default());
    assert!(app.read_with(cx, |app, _| app.sidebar_view == sidebar::View::Snoozed));
    let bounds = cx.debug_bounds("settled-view").unwrap();
    cx.simulate_click(bounds.center(), Default::default());
    assert!(app.read_with(cx, |app, _| app.sidebar_view == sidebar::View::Settled));
    let bounds = cx.debug_bounds("active-view").unwrap();
    cx.simulate_click(bounds.center(), Default::default());
    assert!(app.read_with(cx, |app, _| app.sidebar_view == sidebar::View::Active));
    assert!(requests.try_recv().is_err());
}

#[gpui_kit::test]
fn custom_snooze_rejects_invalid_times_and_normalizes_offsets(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    cx.update(|_, cx| app.update(cx, |app, cx| app.snooze_custom(cx)));
    assert!(requests.try_recv().is_err());
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.custom_snooze.update(cx, |input, cx| {
                input.set_value("2099-04-01T15:00:00+02:00", window, cx)
            });
            app.snooze_custom(cx);
        })
    });
    cx.run_until_parked();
    let connection::Command::Call { payload, reply, .. } = requests.try_recv().unwrap() else {
        panic!("expected snooze");
    };
    assert_eq!(payload["type"], "thread.snooze");
    assert_eq!(payload["snoozedUntil"], "2099-04-01T13:00:00.000Z");
    reply.send_blocking(Ok(json!({"sequence":1}))).unwrap();
    cx.run_until_parked();
}

#[gpui_kit::test]
fn thread_selection_keeps_independent_drafts(cx: &mut TestAppContext) {
    let (app, cx, _requests) = harness(cx);
    cx.simulate_input("Draft for first thread");
    cx.update(|window, cx| {
        app.update(cx, |app, cx| app.select_thread("second-thread", window, cx))
    });
    assert_eq!(
        app.read_with(cx, |app, cx| app.composer.read(cx).value().to_string()),
        ""
    );
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.composer.update(cx, |input, cx| input.focus(window, cx))
        })
    });
    cx.simulate_input("Draft for second thread");
    cx.update(|window, cx| app.update(cx, |app, cx| app.select_thread("t", window, cx)));
    assert_eq!(
        app.read_with(cx, |app, cx| app.composer.read(cx).value().to_string()),
        "Draft for first thread"
    );
    cx.update(|window, cx| {
        app.update(cx, |app, cx| app.select_thread("second-thread", window, cx))
    });
    assert_eq!(
        app.read_with(cx, |app, cx| app.composer.read(cx).value().to_string()),
        "Draft for second thread"
    );
}

#[gpui_kit::test]
fn content_search_preserves_spaces_reports_regex_fallback_and_ignores_stale_results(
    cx: &mut TestAppContext,
) {
    let (app, cx, requests) = harness(cx);
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.content_query
                .update(cx, |input, cx| input.set_value(" foo ", window, cx));
            app.content_options.use_regex = true;
            app.search_contents(cx);
        })
    });
    cx.run_until_parked();
    let connection::Command::Call {
        tag,
        payload,
        reply,
    } = requests.try_recv().unwrap()
    else {
        panic!("content request")
    };
    assert_eq!(tag, "projects.searchContents");
    assert_eq!(payload["query"], " foo ");
    assert_eq!(payload["useRegex"], true);
    reply.send_blocking(Ok(json!({"matches":[{"path":"notes.txt","lineNumber":2,"lineContent":" foo ","matchRanges":[{"start":0,"end":5}]}],"truncated":true,"regexFallbackError":"invalid pattern"}))).unwrap();
    cx.run_until_parked();
    assert!(app.read_with(cx, |app, _| app.status.contains("showing literal matches")));
    assert!(app.read_with(cx, |app, _| app.content_results.as_ref().unwrap().truncated));
    cx.update(|_, cx| app.update(cx, |app, cx| app.search_contents(cx)));
    cx.run_until_parked();
    let connection::Command::Call { reply, .. } = requests.try_recv().unwrap() else {
        panic!("second content request")
    };
    cx.update(|_, cx| app.update(cx, |app, _| app.generation += 1));
    reply
        .send_blocking(Ok(json!({"matches":[],"truncated":false})))
        .unwrap();
    cx.run_until_parked();
    assert!(app.read_with(cx, |app, _| app.content_results.is_none()));
}

#[gpui_kit::test]
fn content_results_keyboard_open_unicode_match_at_correct_line_and_column(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    cx.simulate_keystrokes("cmd-shift-f");
    cx.simulate_input("世界");
    cx.simulate_keystrokes("enter");
    let connection::Command::Call { reply, .. } = requests.try_recv().unwrap() else {
        panic!("search request")
    };
    reply.send_blocking(Ok(json!({"matches":[
        {"path":"first.txt","lineNumber":1,"lineContent":"世界","matchRanges":[{"start":0,"end":2}]},
        {"path":"second.txt","lineNumber":3,"lineContent":"🎉 世界","matchRanges":[{"start":3,"end":5}]}
    ],"truncated":false}))).unwrap();
    cx.run_until_parked();
    cx.simulate_keystrokes("down enter");
    let connection::Command::Call {
        tag,
        payload,
        reply,
    } = requests.try_recv().unwrap()
    else {
        panic!("read request")
    };
    assert_eq!(tag, "projects.readFile");
    assert_eq!(payload["relativePath"], "second.txt");
    reply
        .send_blocking(Ok(
            json!({"relativePath":"second.txt","contents":"one\ntwo\n🎉 世界","truncated":false}),
        ))
        .unwrap();
    cx.run_until_parked();
    assert_eq!(
        app.read_with(cx, |app, cx| app.file_contents.read(cx).cursor_position()),
        Position::new(2, 2)
    );
    cx.simulate_input("HERE");
    assert_eq!(
        app.read_with(cx, |app, cx| app.file_contents.read(cx).value().to_string()),
        "one\ntwo\n🎉 HERE世界"
    );
}

#[gpui_kit::test]
fn clone_validates_destination_and_keeps_failed_form_for_retry(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    cx.simulate_keystrokes("cmd-k");
    cx.simulate_input("clone repository");
    cx.simulate_keystrokes("enter");
    assert!(
        matches!(requests.try_recv().unwrap(), connection::Command::Subscribe { key, tag, .. } if key == "project-clones" && tag == "subscribeProjectClones")
    );
    cx.simulate_input("file:///tmp/owned-source.git");
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.title
                .update(cx, |input, cx| input.set_value("Owned clone", window, cx));
            app.clone_project(cx);
        })
    });
    assert!(requests.try_recv().is_err());
    assert!(app.read_with(cx, |app, _| app.status.contains("destination")));
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.workspace.update(cx, |input, cx| {
                input.set_value("/tmp/owned-destination", window, cx)
            });
            app.clone_project(cx);
        })
    });
    cx.run_until_parked();
    let connection::Command::Call {
        tag,
        payload,
        reply,
    } = requests.try_recv().unwrap()
    else {
        panic!("expected clone")
    };
    assert_eq!(tag, "projectClone.start");
    assert_eq!(payload["remoteUrl"], "file:///tmp/owned-source.git");
    assert_eq!(payload["destinationPath"], "/tmp/owned-destination");
    reply
        .send_blocking(Err("Destination already exists".into()))
        .unwrap();
    cx.run_until_parked();
    assert_eq!(
        app.read_with(cx, |app, _| app.status.clone()),
        "Destination already exists"
    );
    assert_eq!(
        app.read_with(cx, |app, cx| app.clone_remote.read(cx).value().to_string()),
        "file:///tmp/owned-source.git"
    );
    cx.update(|_, cx| app.update(cx, |app, cx| app.clone_project(cx)));
    cx.run_until_parked();
    let connection::Command::Call {
        payload: retry,
        reply,
        ..
    } = requests.try_recv().unwrap()
    else {
        panic!("retry")
    };
    assert_ne!(payload["projectId"], retry["projectId"]);
    reply
        .send_blocking(Ok(json!({"projectId":"created"})))
        .unwrap();
    cx.run_until_parked();
    assert_eq!(
        app.read_with(cx, |app, _| app.selected_project.clone()),
        Some("created".into())
    );
}

#[gpui_kit::test]
fn workspace_reads_follow_selected_thread_worktree_and_reject_other_project_detail(
    cx: &mut TestAppContext,
) {
    let (app, cx, requests) = harness(cx);
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.shell.threads[0].worktree_path = Some("/tmp/owned-worktree".into());
            app.inspector_section = Some(InspectorSection::Files);
            app.file_path
                .update(cx, |input, cx| input.set_value("README.md", window, cx));
            app.file_operation(false, window, cx);
        })
    });
    cx.run_until_parked();
    let connection::Command::Call {
        tag,
        payload,
        reply,
    } = requests.try_recv().unwrap()
    else {
        panic!("read")
    };
    assert_eq!(tag, "projects.readFile");
    assert_eq!(payload["cwd"], "/tmp/owned-worktree");
    reply
        .send_blocking(Err("File missing in worktree".into()))
        .unwrap();
    cx.run_until_parked();
    cx.update(|_, cx| {
        app.update(cx, |app, _| {
            app.detail.thread = Some(json!({"id":"other-thread","worktreePath":"/tmp/unrelated"}));
            assert_eq!(
                app.selected_workspace().as_deref(),
                Some("/tmp/owned-worktree")
            );
            app.detail.thread = Some(json!({"id":"t","worktreePath":null}));
            app.shell.threads[0].worktree_path = None;
            assert_eq!(app.selected_workspace().as_deref(), Some("/tmp/fixture"));
        })
    });
}

#[gpui_kit::test]
fn worktree_removal_requires_confirmation_and_dirty_rejection_preserves_choice(
    cx: &mut TestAppContext,
) {
    let (app, cx, requests) = harness(cx);
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.worktree_path.update(cx, |input, cx| {
                input.set_value("/tmp/owned-worktree", window, cx)
            });
            app.worktree_operation("vcs.removeWorktree", false, cx);
        })
    });
    cx.run_until_parked();
    assert!(requests.try_recv().is_err());
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.worktree_operation("vcs.removeWorktree", false, cx)
        })
    });
    cx.run_until_parked();
    let connection::Command::Call {
        tag,
        payload,
        reply,
    } = requests.try_recv().unwrap()
    else {
        panic!("remove")
    };
    assert_eq!(tag, "vcs.removeWorktree");
    assert_eq!(payload["force"], false);
    assert_eq!(payload["path"], "/tmp/owned-worktree");
    reply
        .send_blocking(Err("Worktree contains modified files".into()))
        .unwrap();
    cx.run_until_parked();
    assert!(!app.read_with(cx, |app, _| app.worktree_busy));
    assert!(app.read_with(cx, |app, _| app.status.contains("modified files")));
    assert_eq!(
        app.read_with(cx, |app, cx| app.worktree_path.read(cx).value().to_string()),
        "/tmp/owned-worktree"
    );
}

#[gpui_kit::test]
fn checkout_choice_preserves_draft_and_creates_new_thread_in_chosen_worktree(
    cx: &mut TestAppContext,
) {
    let (app, cx, requests) = harness(cx);
    cx.simulate_input("Existing draft 世界");
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.choose_checkout(
                "owned-branch".into(),
                Some("/tmp/owned-worktree".into()),
                window,
                cx,
            );
            app.title.update(cx, |input, cx| {
                input.set_value("New worktree thread", window, cx)
            });
            app.provider
                .update(cx, |input, cx| input.set_value("codex", window, cx));
            app.model
                .update(cx, |input, cx| input.set_value("gpt-6.1-sol", window, cx));
            app.execute_command(palette::Command::NewThread, window, cx);
        })
    });
    cx.run_until_parked();
    assert!(matches!(
        requests.try_recv().unwrap(),
        connection::Command::SelectThread(None)
    ));
    assert!(
        matches!(requests.try_recv().unwrap(),connection::Command::Subscribe {tag,..} if tag=="subscribeWorktreeSetup")
    );
    assert!(matches!(
        requests.try_recv().unwrap(),
        connection::Command::SelectThread(None)
    ));
    assert!(requests.try_recv().is_err());
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.composer.update(cx, |input, cx| {
                input.set_value("Owned first turn", window, cx)
            });
            app.send_prompt(window, cx);
        })
    });
    cx.run_until_parked();
    let connection::Command::Call { payload, reply, .. } = requests.try_recv().unwrap() else {
        panic!("first turn")
    };
    assert_eq!(payload["type"], "thread.turn.start");
    assert_eq!(
        payload["bootstrap"]["createThread"]["branch"],
        "owned-branch"
    );
    assert_eq!(
        payload["bootstrap"]["createThread"]["worktreePath"],
        "/tmp/owned-worktree"
    );
    assert_eq!(
        app.read_with(cx, |app, _| app.drafts.get("t").cloned()),
        Some("Existing draft 世界".into())
    );
    assert_eq!(
        app.read_with(cx, |app, _| app.selected_workspace()),
        Some("/tmp/owned-worktree".into())
    );
    reply
        .send_blocking(Err("Creation rejected".into()))
        .unwrap();
    cx.run_until_parked();
    assert_eq!(
        app.read_with(cx, |app, _| app.status.clone()),
        "Creation rejected"
    );
}

#[gpui_kit::test]
fn first_turn_bootstrap_keeps_failed_draft_and_subscribes_history_only_after_acceptance(
    cx: &mut TestAppContext,
) {
    let (app, cx, requests) = harness(cx);
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.prepare_new_worktree = true;
            app.worktree_ref
                .update(cx, |input, cx| input.set_value("main", window, cx));
            app.worktree_branch
                .update(cx, |input, cx| input.set_value("owned-branch", window, cx));
            app.title.update(cx, |input, cx| {
                input.set_value("Owned deferred thread", window, cx)
            });
            app.new_thread(window, cx);
            app.composer.update(cx, |input, cx| {
                input.set_value("Owned first turn 世界", window, cx)
            });
            app.send_prompt(window, cx);
        })
    });
    cx.run_until_parked();
    assert!(
        matches!(requests.try_recv().unwrap(),connection::Command::Subscribe {tag,..} if tag=="subscribeWorktreeSetup")
    );
    assert!(matches!(
        requests.try_recv().unwrap(),
        connection::Command::SelectThread(None)
    ));
    let connection::Command::Call { payload, reply, .. } = requests.try_recv().unwrap() else {
        panic!("start")
    };
    assert_eq!(
        payload["bootstrap"]["prepareWorktree"]["projectCwd"],
        "/tmp/fixture"
    );
    assert_eq!(
        payload["bootstrap"]["prepareWorktree"]["baseBranch"],
        "main"
    );
    assert_eq!(
        payload["bootstrap"]["prepareWorktree"]["branch"],
        "owned-branch"
    );
    assert_eq!(
        payload["bootstrap"]["prepareWorktree"]["startFromOrigin"],
        true
    );
    assert_eq!(payload["bootstrap"]["runSetupScript"], true);
    assert_eq!(payload["message"]["text"], "Owned first turn 世界");
    reply.send_blocking(Err("Setup cancelled".into())).unwrap();
    cx.run_until_parked();
    assert_eq!(app.read_with(cx, |app, _| app.pending_threads.len()), 1);
    assert_eq!(
        app.read_with(cx, |app, cx| app.composer.read(cx).value().to_string()),
        "Owned first turn 世界"
    );
    cx.update(|window, cx| app.update(cx, |app, cx| app.send_prompt(window, cx)));
    cx.run_until_parked();
    let connection::Command::Call {
        payload: retry,
        reply,
        ..
    } = requests.try_recv().unwrap()
    else {
        panic!("retry")
    };
    assert_ne!(payload["commandId"], retry["commandId"]);
    assert_eq!(payload["threadId"], retry["threadId"]);
    reply.send_blocking(Ok(json!({"sequence":1}))).unwrap();
    cx.run_until_parked();
    assert!(app.read_with(cx, |app, _| app.pending_threads.is_empty()));
    assert!(
        matches!(requests.try_recv().unwrap(),connection::Command::SelectThread(Some(id)) if id==retry["threadId"])
    );
}

#[gpui_kit::test]
fn unsent_thread_rename_and_discard_are_local_and_preserve_other_draft(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    cx.simulate_input("Existing draft");
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.new_thread(window, cx);
            app.title.update(cx, |input, cx| {
                input.set_value("Renamed unsent", window, cx)
            });
            app.lifecycle("thread.meta.update", cx);
            app.composer.update(cx, |input, cx| {
                input.set_value("Draft to discard", window, cx)
            });
            app.discard_pending(window, cx);
        })
    });
    cx.run_until_parked();
    assert_eq!(
        app.read_with(cx, |app, _| app.pending_threads[0].thread.title.clone()),
        "Renamed unsent"
    );
    cx.update(|window, cx| app.update(cx, |app, cx| app.discard_pending(window, cx)));
    cx.run_until_parked();
    assert!(app.read_with(cx, |app, _| app.pending_threads.is_empty()));
    assert_eq!(
        app.read_with(cx, |app, _| app.drafts.get("t").cloned()),
        Some("Existing draft".into())
    );
    assert!(app.read_with(cx, |app, cx| app.composer.read(cx).value().is_empty()));
    while let Ok(request) = requests.try_recv() {
        assert!(!matches!(request, connection::Command::Call { .. }));
    }
}

#[gpui_kit::test]
fn changing_project_clears_thread_scope_and_preserves_its_draft(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    cx.simulate_input("retained previous thread draft");
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.shell.projects.push(
                serde_json::from_value(
                    json!({"id":"other","title":"Other project","workspaceRoot":"/tmp/other"}),
                )
                .unwrap(),
            );
            cx.notify();
        })
    });
    let bounds = cx.debug_bounds("project-other").unwrap();
    cx.simulate_click(bounds.center(), gpui_kit::Modifiers::default());
    assert_eq!(
        app.read_with(cx, |app, _| app.selected_project.clone()),
        Some("other".into())
    );
    assert!(app.read_with(cx, |app, _| app.selected_thread.is_none()));
    assert!(app.read_with(cx, |app, cx| app.composer.read(cx).value().is_empty()));
    assert_eq!(
        app.read_with(cx, |app, _| app.drafts.get("t").cloned()),
        Some("retained previous thread draft".into())
    );
    assert!(
        matches!(requests.try_recv().unwrap(), connection::Command::Unsubscribe(key) if key=="worktree-setup")
    );
    assert!(matches!(
        requests.try_recv().unwrap(),
        connection::Command::SelectThread(None)
    ));
}

#[gpui_kit::test]
fn project_action_save_reads_latest_settings_and_preserves_form_on_rejection(
    cx: &mut TestAppContext,
) {
    let (app, cx, requests) = harness(cx);
    cx.simulate_keystrokes("cmd-k");
    cx.simulate_input("edit project actions");
    cx.simulate_keystrokes("enter");
    let connection::Command::Call { tag, reply, .. } = requests.try_recv().unwrap() else {
        panic!("expected settings read")
    };
    assert_eq!(tag, "server.getSettings");
    reply
        .send_blocking(Ok(
            json!({"projectSettingsFolded":true,"defaultProjectScripts":[]}),
        ))
        .unwrap();
    cx.run_until_parked();
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.action_name
                .update(cx, |input, cx| input.set_value("Owned setup", window, cx));
            app.action_command.update(cx, |input, cx| {
                input.set_value("node setup.cjs", window, cx)
            });
            app.action_setup = true;
            app.action_wait = true;
            cx.notify();
        })
    });
    let bounds = cx.debug_bounds("action-save").unwrap();
    cx.simulate_click(bounds.center(), gpui_kit::Modifiers::default());
    let connection::Command::Call { tag, reply, .. } = requests.try_recv().unwrap() else {
        panic!("expected latest settings read")
    };
    assert_eq!(tag, "server.getSettings");
    reply.send_blocking(Ok(json!({"projectSettingsFolded":true,"defaultProjectScripts":[],"projectSettingsOverrides":{"p":{"defaultRuntimeMode":"approval-required"}}}))).unwrap();
    cx.run_until_parked();
    let connection::Command::Call {
        tag,
        payload,
        reply,
    } = requests.try_recv().unwrap()
    else {
        panic!("expected settings update")
    };
    assert_eq!(tag, "server.updateSettings");
    assert_eq!(
        payload["patch"]["projectSettingsOverrides"]["p"]["defaultRuntimeMode"],
        "approval-required"
    );
    let script = &payload["patch"]["projectSettingsOverrides"]["p"]["defaultProjectScripts"][0];
    assert_eq!(script["name"], "Owned setup");
    assert_eq!(script["async"], false);
    reply
        .send_blocking(Err("Owned server settings rejection".into()))
        .unwrap();
    cx.run_until_parked();
    assert!(app.read_with(cx, |app, _| app.status.contains("rejection")));
    assert!(!app.read_with(cx, |app, _| app.action_busy));
    assert_eq!(
        app.read_with(cx, |app, cx| app
            .action_command
            .read(cx)
            .value()
            .to_string()),
        "node setup.cjs"
    );
}

#[gpui_kit::test]
fn materialized_bootstrap_draft_attaches_history_once_before_turn_receipt(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            let thread = app.shell.threads[0].clone();
            app.pending_threads.push(drafts::PendingThread {
                thread,
                created_at: commands::now(),
                prepare_worktree: None,
                run_setup_script: true,
            });
            app.pending_history_attached = false;
            app.attach_created_pending(cx);
            app.attach_created_pending(cx);
        })
    });
    assert!(
        matches!(requests.try_recv().unwrap(),connection::Command::SelectThread(Some(id)) if id=="t")
    );
    assert!(requests.try_recv().is_err());
    assert!(app.read_with(cx, |app, _| app.pending_history_attached));
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.detail.thread = Some(json!({"id":"t","session":{"status":"starting"}}));
            app.shell.threads.clear();
            app.attach_created_pending(cx);
        })
    });
    assert!(matches!(
        requests.try_recv().unwrap(),
        connection::Command::SelectThread(None)
    ));
    assert!(app.read_with(cx, |app, _| app.detail.thread.is_none()));
    assert!(!app.read_with(cx, |app, _| app.pending_history_attached));
    assert_eq!(app.read_with(cx, |app, _| app.pending_threads.len()), 1);
}

#[gpui_kit::test]
fn cancelled_draft_checkout_can_change_without_losing_prompt_or_identity(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.title
                .update(cx, |input, cx| input.set_value("Retry fixture", window, cx));
            app.new_thread(window, cx);
            app.composer.update(cx, |input, cx| {
                input.set_value("Retained prompt 世界", window, cx)
            });
            app.prepare_new_worktree = true;
            app.worktree_ref
                .update(cx, |input, cx| input.set_value("main", window, cx));
            app.worktree_branch
                .update(cx, |input, cx| input.set_value("fresh-retry", window, cx));
            app.update_draft_checkout(cx);
            app.send_prompt(window, cx);
        })
    });
    cx.run_until_parked();
    let mut payload = None;
    while let Ok(command) = requests.try_recv() {
        if let connection::Command::Call { payload: sent, .. } = command {
            payload = Some(sent);
        }
    }
    let payload = payload.expect("first turn command");
    assert_eq!(payload["message"]["text"], "Retained prompt 世界");
    assert_eq!(
        payload["bootstrap"]["prepareWorktree"]["branch"],
        "fresh-retry"
    );
    assert_eq!(
        payload["threadId"],
        app.read_with(cx, |app, _| app.pending_threads[0].thread.id.clone())
    );
}

#[gpui_kit::test]
fn project_default_changes_read_fresh_settings_and_new_draft_keeps_saved_options(
    cx: &mut TestAppContext,
) {
    let (app, cx, requests) = harness(cx);
    cx.update(|_, cx| app.update(cx, |app, cx| app.update_project_default(false, cx)));
    cx.run_until_parked();
    let connection::Command::Call { tag, reply, .. } = requests.try_recv().unwrap() else {
        panic!("settings read")
    };
    assert_eq!(tag, "server.getSettings");
    let selection = json!({"instanceId":"codex","model":"gpt-5.4","options":[{"id":"reasoningEffort","value":"high"}]});
    let settings = json!({"projectSettingsFolded":true,"projectSettingsOverrides":{"p":{"defaultProjectScripts":[{"id":"owned"}]}}});
    reply.send_blocking(Ok(settings.clone())).unwrap();
    cx.run_until_parked();
    let connection::Command::Call {
        tag,
        payload,
        reply,
    } = requests.try_recv().unwrap()
    else {
        panic!("settings write")
    };
    assert_eq!(tag, "server.updateSettings");
    assert_eq!(
        payload["patch"]["projectSettingsOverrides"]["p"]["defaultProjectScripts"],
        settings["projectSettingsOverrides"]["p"]["defaultProjectScripts"]
    );
    let mut saved = settings;
    saved["projectSettingsOverrides"]["p"]["defaultModelSelection"] = selection.clone();
    reply.send_blocking(Ok(saved)).unwrap();
    cx.run_until_parked();
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.title.update(cx, |input, cx| {
                input.set_value("Owned default draft", window, cx)
            });
            app.new_thread(window, cx);
        })
    });
    assert_eq!(
        app.read_with(cx, |app, _| app
            .pending_threads
            .last()
            .unwrap()
            .thread
            .model_selection
            .clone()),
        selection
    );
}

#[gpui_kit::test]
fn native_single_line_insertion_rejects_newlines_without_losing_previous_value(
    cx: &mut TestAppContext,
) {
    use gpui_kit::EntityInputHandler;
    let (app, cx, _) = harness(cx);
    let provider = app.read_with(cx, |app, _| app.provider.clone());
    cx.update(|window, cx| {
        provider.update(cx, |input, cx| {
            input.replace_text_in_range(None, "\n", window, cx);
        })
    });
    cx.run_until_parked();
    assert_eq!(
        provider.read_with(cx, |input, _| input.value().to_string()),
        "codex"
    );
}

#[gpui_kit::test]
fn catalog_options_reach_emitted_turn_and_invalid_choices_preserve_selection(
    cx: &mut TestAppContext,
) {
    let (app, cx, requests) = harness(cx);
    cx.update(|_,cx|app.update(cx,|app,cx| {
        app.provider_catalog=serde_json::from_value(json!([{"instanceId":"codex","driver":"codex","enabled":true,"installed":true,"status":"ready","models":[{"slug":"gpt-5.4","name":"Owned model","capabilities":{"optionDescriptors":[{"id":"reasoningEffort","label":"Effort","type":"select","options":[{"id":"low","label":"Low"},{"id":"high","label":"High"}]}]}}]}])).unwrap();
        app.choose_model_option("reasoningEffort",json!("high"),cx);
        app.choose_model_option("reasoningEffort",json!("invalid"),cx);
    }));
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.composer.update(cx, |input, cx| {
                input.set_value("Owned options prompt", window, cx)
            });
            app.send_prompt(window, cx);
        })
    });
    cx.run_until_parked();
    let connection::Command::Call { payload, reply, .. } = requests.try_recv().unwrap() else {
        panic!("turn")
    };
    assert_eq!(
        payload["modelSelection"]["options"],
        json!([{"id":"reasoningEffort","value":"high"}])
    );
    reply.send_blocking(Err("owned rejection".into())).unwrap();
    cx.run_until_parked();
    assert_eq!(
        app.read_with(cx, |app, cx| app.composer.read(cx).value().to_string()),
        "Owned options prompt"
    );
}

#[gpui_kit::test]
fn palette_stays_centred_as_the_window_resizes(cx: &mut TestAppContext) {
    let (_app, cx, _requests) = harness(cx);
    cx.simulate_keystrokes("cmd-k");
    for width in [1280., 900.] {
        cx.simulate_resize(size(px(width), px(1000.)));
        cx.run_until_parked();
        let palette = cx.debug_bounds("command-palette").expect("palette is open");
        let window = cx.update(|window, _| window.viewport_size());
        assert!((palette.center().x - window.width / 2.).abs() <= px(1.));
        let backdrop = cx.debug_bounds("overlay-backdrop").unwrap();
        assert_eq!(backdrop.size, window);
    }
}

#[gpui_kit::test]
fn settings_closes_on_a_backdrop_click_but_not_an_inside_click(cx: &mut TestAppContext) {
    let (app, cx, _requests) = harness(cx);
    cx.simulate_keystrokes("cmd-,");
    cx.run_until_parked();
    let settings = cx.debug_bounds("settings").expect("settings is open");
    cx.simulate_click(settings.origin + point(px(8.), px(8.)), Default::default());
    assert!(app.read_with(cx, |app, _| app.settings_open));
    cx.simulate_click(
        point(settings.left() - px(24.), settings.center().y),
        Default::default(),
    );
    assert!(!app.read_with(cx, |app, _| app.settings_open));
}

#[gpui_kit::test]
fn slash_menu_runs_the_arrow_selected_command_on_enter(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    // "Pin thread" and "Unpin thread" both match and dispatch different commands,
    // so the request proves which row Enter ran.
    cx.simulate_input("/pin thread");
    cx.simulate_keystrokes("down");
    assert_eq!(app.read_with(cx, |app, _| app.slash_index), 1);
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    let connection::Command::Call { tag, payload, reply } = requests.try_recv().unwrap() else {
        panic!("expected the selected command");
    };
    assert_eq!(tag, "orchestration.dispatchCommand");
    assert_eq!(payload["type"], "thread.unpin");
    reply.send_blocking(Err("owned fixture".into())).unwrap();
    cx.run_until_parked();
    assert!(requests.try_recv().is_err());
    assert_eq!(
        app.read_with(cx, |app, cx| app.composer.read(cx).value().to_string()),
        ""
    );
    // Up wraps to the last row, and Escape clears the slash text instead of sending.
    cx.simulate_input("/");
    cx.simulate_keystrokes("up");
    assert_eq!(app.read_with(cx, |app, _| app.slash_index), 7);
    cx.simulate_keystrokes("escape");
    assert_eq!(
        app.read_with(cx, |app, cx| app.composer.read(cx).value().to_string()),
        ""
    );
    assert!(requests.try_recv().is_err());
}

#[gpui_kit::test]
fn catalog_and_thread_menu_close_on_an_outside_click(cx: &mut TestAppContext) {
    let (app, cx, _requests) = harness(cx);
    let chip = cx.debug_bounds("model-catalog").unwrap();
    cx.simulate_click(chip.center(), Default::default());
    cx.run_until_parked();
    assert!(app.read_with(cx, |app, _| app.catalog_open));
    let catalog = cx
        .debug_bounds("provider-model-catalog")
        .expect("catalog is open");
    // The catalog opens upward from its chip.
    assert!(catalog.bottom() <= chip.top());
    assert!(catalog.left() >= chip.left() - px(1.));
    cx.simulate_click(catalog.origin + point(px(8.), px(8.)), Default::default());
    assert!(app.read_with(cx, |app, _| app.catalog_open));
    cx.simulate_click(
        point(catalog.right() + px(24.), catalog.top() + px(8.)),
        Default::default(),
    );
    assert!(!app.read_with(cx, |app, _| app.catalog_open));
    // Pressing the chip again while open closes it rather than reopening it.
    cx.simulate_click(chip.center(), Default::default());
    cx.run_until_parked();
    cx.simulate_click(chip.center(), Default::default());
    assert!(!app.read_with(cx, |app, _| app.catalog_open));

    let trigger = cx.debug_bounds("thread-menu").unwrap();
    cx.simulate_click(trigger.center(), Default::default());
    cx.run_until_parked();
    assert!(app.read_with(cx, |app, _| app.thread_menu_open));
    cx.simulate_click(point(px(640.), px(500.)), Default::default());
    assert!(!app.read_with(cx, |app, _| app.thread_menu_open));
}

fn seed_second_thread(app: &Entity<NativeApp>, cx: &mut VisualTestContext, pinned: bool) {
    cx.update(|_, cx| app.update(cx, |app, cx| {
        app.shell.apply(serde_json::from_value(json!({"kind":"snapshot","snapshot":{"snapshotSequence":1,"projects":[{"id":"p","title":"Fixture","workspaceRoot":"/tmp/fixture"}],"threads":[
            {"id":"t","projectId":"p","title":"Thread","modelSelection":{"instanceId":"codex","model":"gpt-5.4"},"runtimeMode":"full-access","interactionMode":"default","archivedAt":null},
            {"id":"u","projectId":"p","title":"Other","modelSelection":{"instanceId":"codex","model":"gpt-5.4"},"runtimeMode":"full-access","interactionMode":"default","archivedAt":null,"pinnedAt":if pinned { json!("2026-01-01T00:00:00Z") } else { json!(null) }}
        ]}})).unwrap()).unwrap();
        cx.notify();
    }));
    cx.run_until_parked();
}

fn right_click(cx: &mut VisualTestContext, selector: &'static str) {
    let row = cx.debug_bounds(selector).expect("row is visible");
    cx.simulate_mouse_down(row.center(), MouseButton::Right, Default::default());
    cx.simulate_mouse_up(row.center(), MouseButton::Right, Default::default());
    cx.run_until_parked();
}

#[gpui_kit::test]
fn row_menu_acts_on_its_row_without_changing_the_selection(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    seed_second_thread(&app, cx, false);
    right_click(cx, "thread-u");
    assert!(app.read_with(cx, |app, _| app.thread_menu_open));
    let pin = cx.debug_bounds("pin").expect("Pin is offered for the row");
    cx.simulate_click(pin.center(), Default::default());
    let connection::Command::Call { payload, .. } = requests.try_recv().unwrap() else {
        panic!("expected a pin command");
    };
    assert_eq!(payload["type"], "thread.pin");
    assert_eq!(payload["threadId"], "u");
    assert_eq!(
        app.read_with(cx, |app, _| app.selected_thread.clone()),
        Some("t".into())
    );
    assert!(!app.read_with(cx, |app, _| app.thread_menu_open));

    right_click(cx, "thread-u");
    assert!(app.read_with(cx, |app, _| app.thread_menu_open));
    cx.simulate_keystrokes("escape");
    assert!(!app.read_with(cx, |app, _| app.thread_menu_open));
    assert!(cx.debug_bounds("pin").is_none());
}

#[gpui_kit::test]
fn row_menu_for_a_pinned_thread_offers_unpin_only(cx: &mut TestAppContext) {
    let (app, cx, _requests) = harness(cx);
    seed_second_thread(&app, cx, true);
    right_click(cx, "thread-u");
    assert!(cx.debug_bounds("unpin").is_some());
    assert!(cx.debug_bounds("pin").is_none());
}

fn hide_right_panel_on_changes(app: &Entity<NativeApp>, cx: &mut VisualTestContext) {
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.inspector_open = false;
            app.right_tab = RightTab::Changes;
            cx.notify();
        })
    });
    cx.run_until_parked();
}

#[gpui_kit::test]
fn branch_chip_reveals_worktree_controls_from_a_hidden_panel(cx: &mut TestAppContext) {
    let (app, cx, _requests) = harness(cx);
    cx.update(|_, cx| app.update(cx, |app, _| app.shell.threads[0].branch = Some("main".into())));
    hide_right_panel_on_changes(&app, cx);
    assert!(cx.debug_bounds("draft-checkout-update").is_none());
    let chip = cx.debug_bounds("branch-selector").expect("branch chip");
    cx.simulate_click(chip.center(), Default::default());
    cx.run_until_parked();
    assert!(app.read_with(cx, |app, _| app.inspector_open));
    assert_eq!(app.read_with(cx, |app, _| app.right_tab), RightTab::Inspector);
    assert!(cx.debug_bounds("draft-checkout-update").is_some());
}

#[gpui_kit::test]
fn palette_files_reveals_the_file_controls_from_a_hidden_panel(cx: &mut TestAppContext) {
    let (app, cx, _requests) = harness(cx);
    hide_right_panel_on_changes(&app, cx);
    assert!(cx.debug_bounds("list-files").is_none());
    cx.simulate_keystrokes("cmd-k");
    cx.simulate_input("browse workspace files");
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    assert!(app.read_with(cx, |app, _| app.inspector_open));
    assert!(cx.debug_bounds("list-files").is_some());
    cx.simulate_input("src");
    assert_eq!(
        app.read_with(cx, |app, cx| app.file_path.read(cx).value().to_string()),
        "src"
    );
}

#[gpui_kit::test]
fn opening_a_section_closes_the_previous_one_and_leaving_clone_unsubscribes(
    cx: &mut TestAppContext,
) {
    let (app, cx, requests) = harness(cx);
    assert!(cx.debug_bounds("remove-project").is_some());
    let clone = cx.debug_bounds("section-clone").unwrap();
    cx.simulate_click(clone.center(), Default::default());
    cx.run_until_parked();
    assert!(
        matches!(requests.try_recv().unwrap(), connection::Command::Subscribe { key, .. } if key == "project-clones")
    );
    assert!(cx.debug_bounds("remove-project").is_none());
    assert!(cx.debug_bounds("clone-start").is_some());

    let files = cx.debug_bounds("files").unwrap();
    cx.simulate_click(files.center(), Default::default());
    cx.run_until_parked();
    assert!(
        matches!(requests.try_recv().unwrap(), connection::Command::Unsubscribe(key) if key == "project-clones")
    );
    assert!(cx.debug_bounds("clone-start").is_none());
    assert!(cx.debug_bounds("list-files").is_some());

    let clone = cx.debug_bounds("section-clone").unwrap();
    cx.simulate_click(clone.center(), Default::default());
    cx.run_until_parked();
    assert!(matches!(
        requests.try_recv().unwrap(),
        connection::Command::Subscribe { .. }
    ));
    let clone = cx.debug_bounds("section-clone").unwrap();
    cx.simulate_click(clone.center(), Default::default());
    cx.run_until_parked();
    assert!(
        matches!(requests.try_recv().unwrap(), connection::Command::Unsubscribe(key) if key == "project-clones")
    );
    assert_eq!(app.read_with(cx, |app, _| app.inspector_section), None);
}

fn set_session(app: &Entity<NativeApp>, cx: &mut VisualTestContext, thread: &str, status: &str) {
    cx.update(|_, cx| app.update(cx, |app, cx| {
        let item = json!({"kind":"snapshot","snapshot":{"snapshotSequence":1,"thread":{"id":thread,"title":"Thread","messages":[message(0)],"session":{"status":status}}}});
        app.apply_thread_update(thread, item, cx).unwrap();
        cx.notify();
    }));
    cx.run_until_parked();
}

/// The dispatched calls waiting on the channel; subscriptions are skipped.
fn calls(
    requests: &async_channel::Receiver<connection::Command>,
) -> Vec<(Value, async_channel::Sender<Result<Value, String>>)> {
    std::iter::from_fn(|| requests.try_recv().ok())
        .filter_map(|command| match command {
            connection::Command::Call { payload, reply, .. } => Some((payload, reply)),
            _ => None,
        })
        .collect()
}

fn queue_prompt(cx: &mut VisualTestContext, text: &str) {
    cx.simulate_input(text);
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
}

#[gpui_kit::test]
fn enter_while_running_queues_and_the_completed_turn_sends_it_with_its_settings(
    cx: &mut TestAppContext,
) {
    let (app, cx, requests) = harness(cx);
    set_session(&app, cx, "t", "running");
    queue_prompt(cx, "follow up");
    assert!(calls(&requests).is_empty());
    assert_eq!(
        app.read_with(cx, |app, cx| app.composer.read(cx).value().to_string()),
        ""
    );
    assert!(cx.debug_bounds("queued-0").is_some());
    queue_prompt(cx, "and then this");
    assert!(cx.debug_bounds("queued-1").is_some());
    // Settings changed after queueing do not reach the queued send.
    cx.update(|window, cx| app.update(cx, |app, cx| {
        app.shell.threads[0].runtime_mode = Some("approval-required".into());
        app.shell.threads[0].interaction_mode = Some("plan".into());
        app.model.update(cx, |input, cx| input.set_value("gpt-5.5", window, cx));
    }));

    set_session(&app, cx, "t", "ready");
    let mut sent = calls(&requests);
    assert_eq!(sent.len(), 1);
    let (payload, reply) = sent.remove(0);
    assert_eq!(payload["type"], "thread.turn.start");
    assert_eq!(payload["message"]["text"], "follow up");
    assert_eq!(payload["modelSelection"]["model"], "gpt-5.4");
    assert_eq!(payload["runtimeMode"], "full-access");
    assert_eq!(payload["interactionMode"], "default");
    reply.send_blocking(Ok(json!({"sequence":2}))).unwrap();
    cx.run_until_parked();
    assert!(cx.debug_bounds("queued-1").is_none());

    // The next one waits for the turn just sent to complete.
    set_session(&app, cx, "t", "ready");
    assert!(calls(&requests).is_empty());
    set_session(&app, cx, "t", "running");
    set_session(&app, cx, "t", "ready");
    let sent = calls(&requests);
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].0["message"]["text"], "and then this");
}

#[gpui_kit::test]
fn a_removed_queued_message_is_never_sent(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    set_session(&app, cx, "t", "running");
    cx.update(|_, cx| app.update(cx, |app, _| {
        app.attachments.push(
            json!({"id": "a1", "name": "notes.md", "mimeType": "text/markdown", "sizeBytes": 12}),
        );
    }));
    queue_prompt(cx, "never mind");
    let remove = cx.debug_bounds("queued-remove-0").expect("queued row is visible");
    cx.simulate_click(remove.center(), Default::default());
    cx.run_until_parked();
    assert!(cx.debug_bounds("queued-0").is_none());
    let sent = calls(&requests);
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].0["attachmentId"], "a1");

    set_session(&app, cx, "t", "ready");
    assert!(calls(&requests).is_empty());
}

#[gpui_kit::test]
fn steering_interrupts_then_sends_ahead_of_the_queue(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    set_session(&app, cx, "t", "running");
    queue_prompt(cx, "later");
    cx.simulate_input("change course");
    cx.simulate_keystrokes("cmd-shift-enter");
    cx.run_until_parked();
    let mut sent = calls(&requests);
    assert_eq!(sent.len(), 1);
    let (payload, reply) = sent.remove(0);
    assert_eq!(payload["type"], "thread.turn.interrupt");
    assert_eq!(
        app.read_with(cx, |app, cx| app.composer.read(cx).value().to_string()),
        ""
    );
    reply.send_blocking(Ok(json!({"sequence":2}))).unwrap();
    cx.run_until_parked();
    let sent = calls(&requests);
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].0["type"], "thread.turn.start");
    assert_eq!(sent[0].0["message"]["text"], "change course");
}

#[gpui_kit::test]
fn a_queue_waits_for_its_own_thread_to_complete(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    set_session(&app, cx, "t", "running");
    queue_prompt(cx, "for t");
    let shell = |status: &str| {
        json!({"kind":"snapshot","snapshot":{"snapshotSequence":1,"projects":[{"id":"p","title":"Fixture","workspaceRoot":"/tmp/fixture"}],"threads":[
            {"id":"t","projectId":"p","title":"Thread","modelSelection":{"instanceId":"codex","model":"gpt-5.4"},"session":{"status":status}},
            {"id":"u","projectId":"p","title":"Other","modelSelection":{"instanceId":"codex","model":"gpt-5.4"},"session":{"status":"running"}}
        ]}})
    };
    cx.update(|window, cx| app.update(cx, |app, cx| {
        app.shell.apply(serde_json::from_value(shell("running")).unwrap()).unwrap();
        app.select_thread("u", window, cx);
    }));
    set_session(&app, cx, "u", "running");
    set_session(&app, cx, "u", "ready");
    assert!(calls(&requests).is_empty());
    assert!(cx.debug_bounds("queued-0").is_none(), "the queue belongs to t");

    cx.update(|_, cx| app.update(cx, |app, cx| {
        app.shell.apply(serde_json::from_value(shell("ready")).unwrap()).unwrap();
        app.flush_queues(cx);
    }));
    cx.run_until_parked();
    let sent = calls(&requests);
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].0["threadId"], "t");
    assert_eq!(sent[0].0["message"]["text"], "for t");
}

#[gpui_kit::test]
fn context_meter_follows_streamed_readings_and_the_setting_hides_it(cx: &mut TestAppContext) {
    let (app, cx, _requests) = harness(cx);
    let reading = |id: &str, sequence: u64, used: u64| {
        json!({"id":id,"kind":"context-window.updated","tone":"info","summary":"Context window updated","turnId":null,"sequence":sequence,"createdAt":"2026-01-01T00:00:00Z","payload":{"usedTokens":used,"maxTokens":200_000}})
    };
    assert!(cx.debug_bounds("context-meter").is_none(), "no reading, no meter");
    cx.update(|_, cx| app.update(cx, |app, cx| {
        let item = json!({"kind":"snapshot","snapshot":{"snapshotSequence":1,"thread":{"id":"t","title":"Thread","messages":[message(0)],"activities":[reading("a1",1,132_400)],"session":{"status":"running"}}}});
        app.apply_thread_update("t", item, cx).unwrap();
        cx.notify();
    }));
    cx.run_until_parked();
    assert!(cx.debug_bounds("context-meter").is_some());
    assert!(cx.debug_bounds("context-meter-66%").is_some());

    cx.update(|_, cx| app.update(cx, |app, cx| {
        let item = json!({"kind":"event","event":{"sequence":2,"type":"thread.activity-appended","occurredAt":"2026-01-01T00:00:05Z","payload":{"threadId":"t","activity":reading("a2",2,190_000)}}});
        app.apply_thread_update("t", item, cx).unwrap();
        cx.notify();
    }));
    cx.run_until_parked();
    assert!(cx.debug_bounds("context-meter-66%").is_none());
    assert!(cx.debug_bounds("context-meter-95%").is_some(), "streamed reading updates the meter");

    let bounds = cx.debug_bounds("settings-open").expect("title bar settings button");
    cx.simulate_click(bounds.center(), Default::default());
    cx.run_until_parked();
    let bounds = cx
        .debug_bounds("settings-context-meter")
        .expect("appearance meter toggle");
    cx.simulate_click(bounds.center(), Default::default());
    cx.run_until_parked();
    assert!(app.read_with(cx, |app, _| app.context_meter_hidden));
    assert!(cx.debug_bounds("context-meter").is_none(), "the setting hides the meter");
}

fn composer_focused(app: &Entity<NativeApp>, cx: &mut VisualTestContext) -> bool {
    cx.update(|window, cx| {
        let composer = app.read(cx).composer.clone();
        composer.read(cx).focus_handle(cx).is_focused(window)
    })
}

#[gpui_kit::test]
fn cmd_b_hides_the_sidebar_and_the_title_bar_button_restores_it(cx: &mut TestAppContext) {
    let (app, cx, _requests) = harness(cx);
    let composer = cx.debug_bounds("composer").expect("composer");
    let inspector = cx.debug_bounds("inspector").expect("inspector");
    assert_eq!(
        inspector.size.width,
        px(window_state::RIGHT_PANEL_WIDTH.1),
        "window resizes do not rescale the side panels"
    );
    assert!(cx.debug_bounds("sidebar").is_some());
    assert!(cx.debug_bounds("sidebar-show").is_none());

    cx.simulate_keystrokes("cmd-b");
    assert!(cx.debug_bounds("sidebar").is_none());
    let wide = cx.debug_bounds("composer").expect("composer");
    assert!(wide.left() < composer.left() && wide.size.width > composer.size.width);
    assert_eq!(
        cx.debug_bounds("inspector").unwrap().size.width,
        inspector.size.width,
        "the right panel keeps its width"
    );
    assert!(composer_focused(&app, cx), "toggling leaves focus alone");

    let restore = cx.debug_bounds("sidebar-show").expect("restore button");
    cx.simulate_click(restore.center(), Default::default());
    cx.run_until_parked();
    assert!(cx.debug_bounds("sidebar").is_some());
    assert!(cx.debug_bounds("sidebar-show").is_none());
    assert_eq!(cx.debug_bounds("composer").unwrap(), composer);
    assert!(composer_focused(&app, cx));
}

fn drag(cx: &mut VisualTestContext, from: Point<Pixels>, to: Point<Pixels>) {
    cx.simulate_mouse_down(from, MouseButton::Left, Default::default());
    cx.simulate_mouse_move(
        from + point(px(10.), px(0.)),
        Some(MouseButton::Left),
        Default::default(),
    );
    cx.simulate_mouse_move(to, Some(MouseButton::Left), Default::default());
    cx.simulate_mouse_up(to, MouseButton::Left, Default::default());
    cx.run_until_parked();
}

#[gpui_kit::test]
fn resizing_and_toggling_panels_update_the_saved_layout(cx: &mut TestAppContext) {
    let (app, cx, _requests) = harness(cx);
    cx.update(|_, cx| {
        app.update(cx, |app, _| {
            app.saved_window = Some(window_state::WindowState::default())
        })
    });
    let saved = |cx: &mut VisualTestContext| app.read_with(cx, |app, _| app.saved_window.unwrap());

    let sidebar = cx.debug_bounds("sidebar").unwrap();
    let y = sidebar.center().y;
    drag(cx, point(sidebar.right() - px(2.), y), point(px(360.), y));
    assert_eq!(cx.debug_bounds("sidebar").unwrap().size.width, px(360.));
    assert_eq!(saved(cx).sidebar_width, 360.);

    let inspector = cx.debug_bounds("inspector").unwrap();
    drag(
        cx,
        point(inspector.left() - px(2.), y),
        point(inspector.left() - px(80.), y),
    );
    let widened = cx.debug_bounds("inspector").unwrap().size.width;
    assert!((widened - inspector.size.width - px(80.)).abs() < px(1.));
    assert!((saved(cx).right_panel_width - widened / px(1.)).abs() < 1.);

    cx.simulate_keystrokes("cmd-b");
    assert!(!saved(cx).sidebar_open);
    cx.simulate_keystrokes("cmd-alt-b");
    assert!(!saved(cx).right_panel_open);
    cx.simulate_keystrokes("cmd-b");
    assert_eq!(cx.debug_bounds("sidebar").unwrap().size.width, px(360.));
    let toggle = cx.debug_bounds("toggle-inspector").unwrap();
    cx.simulate_click(toggle.center(), Default::default());
    cx.run_until_parked();
    let state = saved(cx);
    assert!(state.sidebar_open && state.right_panel_open);
    assert!((cx.debug_bounds("inspector").unwrap().size.width - widened).abs() < px(1.));
}

#[gpui_kit::test]
fn hiding_the_sidebar_keeps_an_open_row_menu_usable(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    seed_second_thread(&app, cx, false);
    right_click(cx, "thread-u");
    cx.simulate_keystrokes("cmd-b");
    assert!(cx.debug_bounds("sidebar").is_none());
    let pin = cx.debug_bounds("pin").expect("the row menu stays open");
    cx.simulate_click(pin.center(), Default::default());
    let connection::Command::Call { payload, .. } = requests.try_recv().unwrap() else {
        panic!("expected a pin command");
    };
    assert_eq!(payload["threadId"], "u");
    assert!(!app.read_with(cx, |app, _| app.thread_menu_open));
}

/// Arms and confirms the fixture checkpoint's revert, answering its call with `reply`.
fn revert_checkpoint(
    app: &Entity<NativeApp>,
    cx: &mut VisualTestContext,
    requests: &async_channel::Receiver<connection::Command>,
    reply: Result<Value, String>,
) {
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.detail.thread = Some(json!({
                "id": "t", "title": "Thread", "messages": [], "activities": [], "proposedPlans": [],
                "checkpoints": [{"turnId": "turn-1", "checkpointTurnCount": 1, "checkpointRef": "ref", "status": "ready",
                    "files": [{"path": "a.rs", "kind": "modified", "additions": 1, "deletions": 0}], "assistantMessageId": null, "completedAt": "2026-01-01T00:00:00Z"}],
                "session": null,
            }));
            cx.notify();
        })
    });
    cx.run_until_parked();
    for _ in 0..2 {
        let bounds = cx.debug_bounds("revert-checkpoint").expect("revert button");
        cx.simulate_click(bounds.center(), Default::default());
    }
    let mut sent = calls(requests);
    assert_eq!(sent.len(), 1);
    let (payload, sender) = sent.remove(0);
    assert_eq!(payload["type"], "thread.checkpoint.revert");
    sender.try_send(reply).unwrap();
    cx.run_until_parked();
}

fn toast_messages(app: &Entity<NativeApp>, cx: &mut VisualTestContext) -> Vec<String> {
    app.read_with(cx, |app, _| {
        app.toasts
            .iter()
            .map(|toast| toast.message.clone())
            .collect()
    })
}

#[gpui_kit::test]
fn an_error_outcome_raises_a_toast_that_stays_until_closed(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    revert_checkpoint(
        &app,
        cx,
        &requests,
        Err("Checkpoint revert rejected".into()),
    );
    assert_eq!(toast_messages(&app, cx), ["Checkpoint revert rejected"]);
    assert_eq!(app.read_with(cx, |app, _| app.toasts[0].id), 1);
    assert!(cx.debug_bounds("toast-1").is_some());

    cx.background_executor
        .advance_clock(notifications::TOAST_LIFETIME * 2);
    cx.run_until_parked();
    assert_eq!(toast_messages(&app, cx), ["Checkpoint revert rejected"]);

    let close = cx
        .debug_bounds("toast-close-1")
        .expect("the toast has a close control");
    cx.simulate_click(close.center(), Default::default());
    cx.run_until_parked();
    assert!(toast_messages(&app, cx).is_empty());
    assert_eq!(
        app.read_with(cx, |app, _| app.status.clone()),
        "Checkpoint revert rejected"
    );
}

#[gpui_kit::test]
fn a_success_toast_leaves_when_its_dismissal_fires(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    revert_checkpoint(&app, cx, &requests, Ok(json!({"sequence": 3})));
    assert_eq!(toast_messages(&app, cx), ["Checkpoint restored"]);

    cx.background_executor
        .advance_clock(notifications::TOAST_LIFETIME - std::time::Duration::from_millis(1));
    cx.run_until_parked();
    assert_eq!(toast_messages(&app, cx), ["Checkpoint restored"]);
    cx.background_executor
        .advance_clock(std::time::Duration::from_millis(1));
    cx.run_until_parked();
    assert!(toast_messages(&app, cx).is_empty());
    assert_eq!(
        app.read_with(cx, |app, _| app.status.clone()),
        "Checkpoint restored"
    );
}

fn upsert_thread(
    app: &Entity<NativeApp>,
    cx: &mut VisualTestContext,
    sequence: u64,
    thread: Value,
) {
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            let mut thread = thread;
            thread.as_object_mut().unwrap().extend(
                json!({"projectId": "p", "modelSelection": {"instanceId": "codex", "model": "gpt-5.4"}})
                    .as_object()
                    .unwrap()
                    .clone(),
            );
            let item = serde_json::from_value(
                json!({"kind": "thread-upserted", "sequence": sequence, "thread": thread}),
            )
            .unwrap();
            app.apply_shell_item(item, cx).unwrap();
        })
    });
    cx.run_until_parked();
}

#[gpui_kit::test]
fn approval_on_another_thread_notifies_and_the_click_selects_it(cx: &mut TestAppContext) {
    let (app, cx, _requests) = harness(cx);
    seed_second_thread(&app, cx, false);
    cx.update(|window, _| window.activate_window());
    cx.run_until_parked();
    assert!(app.read_with(cx, |app, _| app.window_active));

    upsert_thread(
        &app,
        cx,
        2,
        json!({"id": "t", "title": "Thread", "hasPendingApprovals": true}),
    );
    assert!(
        cx.shown_system_notifications().is_empty(),
        "the selected thread is in view"
    );

    upsert_thread(
        &app,
        cx,
        3,
        json!({"id": "u", "title": "Other", "hasPendingApprovals": true}),
    );
    let shown = cx.shown_system_notifications();
    assert_eq!(shown.len(), 1);
    assert_eq!(shown[0].title.as_ref(), "Approval needed");
    assert_eq!(shown[0].body.as_ref(), "Other");

    cx.simulate_system_notification_response(gpui_kit::SystemNotificationResponse {
        tag: shown[0].tag.clone(),
        action_id: None,
    });
    cx.run_until_parked();
    assert_eq!(
        app.read_with(cx, |app, _| app.selected_thread.clone())
            .as_deref(),
        Some("u")
    );
}
