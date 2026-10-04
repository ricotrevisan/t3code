//! Headless GPUI interactions: real input dispatch and asynchronous receipts, not markup assertions.
use super::*;
use gpui::{AppContext, TestAppContext, VisualTestContext};

fn harness(
    cx: &mut TestAppContext,
) -> (
    Entity<NativeApp>,
    &mut VisualTestContext,
    async_channel::Receiver<connection::Command>,
) {
    cx.update(|cx| {
        gpui_component::init(cx);
        bind_native_keys(cx);
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
        matches!(requests.try_recv().unwrap(),connection::Command::SelectThread(Some(id)) if id == "t")
    );
    cx.run_until_parked();
    (app, cx, requests)
}

#[gpui::test]
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

#[gpui::test]
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

#[gpui::test]
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

#[gpui::test]
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

#[gpui::test]
fn choosing_catalog_model_uses_instance_routing_and_preserves_unchanged_options(
    cx: &mut TestAppContext,
) {
    let (app, cx, requests) = harness(cx);
    cx.update(|_,cx|app.update(cx,|app,cx| {
        app.shell.threads[0].model_selection=json!({"instanceId":"codex","model":"gpt-5.4","options":[{"id":"reasoningEffort","value":"high"}]});
        app.provider_catalog=serde_json::from_value(json!([{"instanceId":"codex","driver":"codex","enabled":true,"installed":true,"status":"ready","models":[{"slug":"gpt-5.4","name":"Current model"}]}])).unwrap();app.catalog_open=true;cx.notify();
    }));
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

#[gpui::test]
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

#[gpui::test]
fn loaded_file_editor_accepts_real_input_dispatch(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.files_open = true;
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

#[gpui::test]
fn file_save_uses_loaded_target_preserves_later_edits_and_failed_reads(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.files_open = true;
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

#[gpui::test]
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

#[gpui::test]
fn changing_server_cannot_reuse_another_origins_access_credential(cx: &mut TestAppContext) {
    let (app, cx, _) = harness(cx);
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.use_dev_credential = false;
            app.access = Some(auth::Access {
                origin: "http://localhost:1234".into(),
                token: "private-first-origin".into(),
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

#[gpui::test]
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

#[gpui::test]
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

#[gpui::test]
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

#[gpui::test]
fn delete_requires_confirmation_before_any_request(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
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

#[gpui::test]
fn clipboard_roundtrip_preserves_multiline_unicode(cx: &mut TestAppContext) {
    let (app, cx, _requests) = harness(cx);
    cx.write_to_clipboard(gpui::ClipboardItem::new_string("Hello 🌍\n世界".into()));
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

#[gpui::test]
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

#[gpui::test]
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

#[gpui::test]
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

#[gpui::test]
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
