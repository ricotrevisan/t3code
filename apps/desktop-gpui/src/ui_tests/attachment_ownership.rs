use super::*;

fn attachment_harness(
    cx: &mut TestAppContext,
) -> (Entity<NativeApp>, &mut VisualTestContext, Requests) {
    let (app, cx, requests) = harness(cx);
    cx.update(|_, cx| {
        app.update(cx, |app, _| {
            app.environment_scope = "https://attachments.example".into();
        })
    });
    (app, cx, requests)
}

fn fixture_file() -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("notes.txt");
    std::fs::write(&path, b"attachment body").unwrap();
    (dir, path)
}

fn start_upload(app: &Entity<NativeApp>, cx: &mut VisualTestContext, path: std::path::PathBuf) {
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            let scope = app.attachment_scope().unwrap();
            app.attach_paths(scope, Some(vec![path]), cx);
        })
    });
    cx.run_until_parked();
}

fn issue_upload(
    requests: &Requests,
    cx: &mut VisualTestContext,
) -> async_channel::Sender<Result<(), String>> {
    let connection::Command::Call {
        tag,
        payload,
        reply,
    } = requests.try_recv().unwrap()
    else {
        panic!("upload URL request");
    };
    assert_eq!(tag, "attachments.createUploadUrl");
    assert_eq!(payload["sizeBytes"], 15);
    reply.try_send(Ok(json!({"attachmentId":"pending-00000000-0000-4000-8000-000000000001-txt", "relativeUrl":"/api/attachments/upload/claims.signature"}))).unwrap();
    cx.run_until_parked();
    let connection::Command::Upload {
        bytes, url, reply, ..
    } = requests.try_recv().unwrap()
    else {
        panic!("upload body");
    };
    assert_eq!(bytes, b"attachment body");
    assert!(url.ends_with("/api/attachments/upload/claims.signature"));
    reply
}

fn select_other(app: &Entity<NativeApp>, cx: &mut VisualTestContext) {
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            let mut other = app.shell.threads[0].clone();
            other.id = "other".into();
            app.shell.threads.push(other);
            app.select_thread("other", window, cx);
            app.composer
                .update(cx, |input, cx| input.set_value("other prompt", window, cx));
            app.attachments.push(json!({"id":"other-ref"}));
        })
    });
    cx.run_until_parked();
}

#[gpui_kit::test]
fn delayed_upload_returns_to_original_draft_without_replacing_other_context(
    cx: &mut TestAppContext,
) {
    let (app, cx, requests) = attachment_harness(cx);
    let (_dir, path) = fixture_file();
    start_upload(&app, cx, path);
    let receipt = issue_upload(&requests, cx);
    select_other(&app, cx);
    let status = app.read_with(cx, |app, _| app.status.clone());
    receipt.try_send(Ok(())).unwrap();
    cx.run_until_parked();
    app.read_with(cx, |app, cx| {
        assert_eq!(app.attachments, vec![json!({"id":"other-ref"})]);
        assert_eq!(app.composer.read(cx).value().as_str(), "other prompt");
        assert_eq!(app.status, status);
        assert_eq!(app.pending_actions, 0);
    });
    cx.update(|window, cx| app.update(cx, |app, cx| app.select_thread("t", window, cx)));
    cx.run_until_parked();
    app.read_with(cx, |app, _| {
        assert_eq!(
            app.attachments[0]["id"],
            "pending-00000000-0000-4000-8000-000000000001-txt"
        )
    });
    cx.update(|window, cx| app.update(cx, |app, cx| app.select_thread("other", window, cx)));
    app.read_with(cx, |app, _| {
        assert_eq!(app.attachments, vec![json!({"id":"other-ref"})])
    });
}

#[gpui_kit::test]
fn transmitted_upload_survives_environment_and_generation_change_without_touching_new_work(
    cx: &mut TestAppContext,
) {
    let (app, cx, requests) = attachment_harness(cx);
    let (_dir, path) = fixture_file();
    let origin = app.read_with(cx, |app, _| app.environment_scope.clone());
    start_upload(&app, cx, path);
    let receipt = issue_upload(&requests, cx);
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.disconnect(cx);
            app.environment_scope = "https://different.example".into();
            app.selected_thread = Some("t".into());
            app.restore_attachment_draft();
            app.pending_actions = 7;
            app.attachments.push(json!({"id":"new-work"}));
            app.status = "New environment".into();
        })
    });
    receipt.try_send(Ok(())).unwrap();
    cx.run_until_parked();
    app.read_with(cx, |app, _| {
        assert_eq!(app.pending_actions, 7);
        assert_eq!(app.status, "New environment");
        assert_eq!(app.attachments, vec![json!({"id":"new-work"})]);
    });
    cx.update(|_, cx| {
        app.update(cx, |app, _| {
            app.stash_attachment_draft();
            app.environment_scope = origin;
            app.restore_attachment_draft();
            assert_eq!(
                app.attachments[0]["id"],
                "pending-00000000-0000-4000-8000-000000000001-txt"
            );
        })
    });
}

#[gpui_kit::test]
fn canceled_picker_and_stale_path_choice_leave_context_untouched(cx: &mut TestAppContext) {
    let (app, cx, requests) = attachment_harness(cx);
    let (_dir, path) = fixture_file();
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.attachments.push(json!({"id":"existing"}));
            app.composer
                .update(cx, |input, cx| input.set_value("keep prompt", window, cx));
            app.status = "keep status".into();
            let scope = app.attachment_scope().unwrap();
            app.attach_paths(scope.clone(), None, cx);
            app.attach_paths(scope.clone(), Some(vec![]), cx);
            app.generation += 1;
            app.pending_actions = 9;
            app.attach_paths(scope, Some(vec![path]), cx);
        })
    });
    cx.run_until_parked();
    assert!(requests.try_recv().is_err());
    app.read_with(cx, |app, cx| {
        assert_eq!(app.attachments, vec![json!({"id":"existing"})]);
        assert_eq!(app.composer.read(cx).value().as_str(), "keep prompt");
        assert_eq!(app.status, "keep status");
        assert_eq!(app.pending_actions, 9);
    });
}

#[gpui_kit::test]
fn invalid_upload_target_never_dispatches_body_or_stages_refs(cx: &mut TestAppContext) {
    let (app, cx, requests) = attachment_harness(cx);
    let (_dir, path) = fixture_file();
    for target in [
        json!({"attachmentId":"", "relativeUrl":"/api/attachments/upload/claims.signature"}),
        json!({"attachmentId":"pending-00000000-0000-4000-8000-000000000001-txt", "relativeUrl":"https://foreign.example/api/attachments/upload/claims.signature"}),
        json!({"attachmentId":"pending-00000000-0000-4000-8000-000000000001-txt", "relativeUrl":"/api/attachments/upload/../escape"}),
    ] {
        start_upload(&app, cx, path.clone());
        let connection::Command::Call { reply, .. } = requests.try_recv().unwrap() else {
            panic!("create upload");
        };
        reply.try_send(Ok(target)).unwrap();
        cx.run_until_parked();
        assert!(requests.try_recv().is_err());
        app.read_with(cx, |app, _| {
            assert!(app.attachments.is_empty());
            assert_eq!(app.pending_actions, 0);
            assert!(!app.attachment_drafts.uploading);
        });
    }
}

#[gpui_kit::test]
fn background_read_error_is_scoped_to_original_selected_draft(cx: &mut TestAppContext) {
    let (app, cx, requests) = attachment_harness(cx);
    let (dir, _) = fixture_file();
    // Dispatch on behalf of A while B is selected. Read errors must not become
    // B's toast/status; the path seam uses the exact picker-captured scope.
    let scope = app.read_with(cx, |app, _| app.attachment_scope().unwrap());
    select_other(&app, cx);
    while requests.try_recv().is_ok() {}
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.status = "B status".into();
            app.attach_paths(scope, Some(vec![dir.path().join("missing.txt")]), cx);
        })
    });
    cx.run_until_parked();
    assert!(requests.try_recv().is_err());
    app.read_with(cx, |app, _| {
        assert_eq!(app.status, "B status");
        assert_eq!(app.attachments, vec![json!({"id":"other-ref"})]);
        assert_eq!(app.pending_actions, 0);
    });
    start_upload(&app, cx, dir.path().join("missing.txt"));
    app.read_with(cx, |app, _| {
        assert_eq!(app.status, "Could not open attachment")
    });
}

#[gpui_kit::test]
fn send_failure_retains_refs_and_late_success_consumes_only_original_refs(cx: &mut TestAppContext) {
    let (app, cx, requests) = attachment_harness(cx);
    for success in [false, true] {
        cx.update(|window, cx| {
            app.update(cx, |app, cx| {
                if app.attachments.is_empty() {
                    app.attachments.push(json!({"id":"sent-ref"}));
                }
                app.composer
                    .update(cx, |input, cx| input.set_value("send prompt", window, cx));
                app.send_prompt(window, cx);
            })
        });
        cx.run_until_parked();
        let connection::Command::Call {
            tag,
            payload,
            reply,
        } = requests.try_recv().unwrap()
        else {
            panic!("send");
        };
        assert_eq!(tag, "orchestration.dispatchCommand");
        assert_eq!(payload["type"], "thread.turn.start");
        assert_eq!(payload["message"]["attachments"][0]["id"], "sent-ref");
        if success {
            select_other(&app, cx);
        }
        reply
            .try_send(if success {
                Ok(json!({"sequence":1}))
            } else {
                Err("rejected".into())
            })
            .unwrap();
        cx.run_until_parked();
        if !success {
            app.read_with(cx, |app, cx| {
                assert_eq!(app.attachments[0]["id"], "sent-ref");
                assert_eq!(app.composer.read(cx).value().as_str(), "send prompt");
            });
        } else {
            app.read_with(cx, |app, _| {
                assert_eq!(app.attachments, vec![json!({"id":"other-ref"})])
            });
            cx.update(|window, cx| app.update(cx, |app, cx| app.select_thread("t", window, cx)));
            app.read_with(cx, |app, _| assert!(app.attachments.is_empty()));
        }
    }
}

#[gpui_kit::test]
fn delayed_upload_after_same_environment_reconnect_is_shelved_not_staged(cx: &mut TestAppContext) {
    let (app, cx, requests) = attachment_harness(cx);
    let (_dir, path) = fixture_file();
    start_upload(&app, cx, path);
    let receipt = issue_upload(&requests, cx);
    cx.update(|_, cx| {
        app.update(cx, |app, _| {
            app.generation += 1;
            app.pending_actions = 4;
            app.attachments.push(json!({"id":"new-generation"}));
            app.status = "new-generation status".into();
        })
    });
    receipt.try_send(Ok(())).unwrap();
    cx.run_until_parked();
    app.read_with(cx, |app, _| {
        assert_eq!(app.pending_actions, 4);
        assert_eq!(app.status, "new-generation status");
        assert_eq!(app.attachments, vec![json!({"id":"new-generation"})]);
    });
    cx.update(|_, cx| {
        app.update(cx, |app, _| {
            app.stash_attachment_draft();
            app.restore_attachment_draft();
            assert_eq!(app.attachments.len(), 2);
            assert!(
                app.attachments
                    .iter()
                    .any(|item| item["id"] == "pending-00000000-0000-4000-8000-000000000001-txt")
            );
        })
    });
}

#[gpui_kit::test]
fn stale_connection_between_signing_and_upload_never_transmits_body(cx: &mut TestAppContext) {
    let (app, cx, requests) = attachment_harness(cx);
    let (_dir, path) = fixture_file();
    start_upload(&app, cx, path.clone());
    let connection::Command::Call { reply, .. } = requests.try_recv().unwrap() else {
        panic!("signing request");
    };
    cx.update(|_, cx| {
        app.update(cx, |app, _| {
            app.generation += 1;
            app.pending_actions = 8;
        })
    });
    reply
        .try_send(Ok(
            json!({"attachmentId":"pending-00000000-0000-4000-8000-000000000001-txt", "relativeUrl":"/api/attachments/upload/claims.signature"}),
        ))
        .unwrap();
    cx.run_until_parked();
    assert!(requests.try_recv().is_err());
    app.read_with(cx, |app, _| {
        assert!(app.attachments.is_empty());
        assert_eq!(app.pending_actions, 8);
    });
    // A closed captured sender also rejects a returned picker choice before IO/RPC.
    let scope = app.read_with(cx, |app, _| app.attachment_scope().unwrap());
    requests.close();
    cx.update(|_, cx| app.update(cx, |app, cx| app.attach_paths(scope, Some(vec![path]), cx)));
    app.read_with(cx, |app, _| assert!(!app.attachment_drafts.uploading));
}

#[gpui_kit::test]
fn no_selected_thread_rejects_attachment_before_picker(cx: &mut TestAppContext) {
    let (app, cx, requests) = attachment_harness(cx);
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.stash_attachment_draft();
            app.selected_thread = None;
            app.restore_attachment_draft();
            app.attach_files(cx);
        })
    });
    cx.run_until_parked();
    assert!(requests.try_recv().is_err());
    app.read_with(cx, |app, _| {
        assert_eq!(app.status, "Select a thread first");
        assert_eq!(app.pending_actions, 0);
    });
}

#[gpui_kit::test]
fn explicit_detach_failure_retains_original_resource_and_retry_deletes_only_it(
    cx: &mut TestAppContext,
) {
    let (app, cx, requests) = attachment_harness(cx);
    let scope = cx.update(|_, cx| {
        app.update(cx, |app, _| {
            app.attachments.push(json!({"id":"delete-ref"}));
            app.attachment_scope().unwrap()
        })
    });
    select_other(&app, cx);
    while requests.try_recv().is_ok() {}
    for success in [false, true] {
        cx.update(|_, cx| {
            app.update(cx, |app, cx| {
                app.detach_attachment(scope.clone(), "delete-ref".into(), cx)
            })
        });
        cx.run_until_parked();
        let connection::Command::Call {
            tag,
            payload,
            reply,
        } = requests.try_recv().unwrap()
        else {
            panic!("delete request");
        };
        assert_eq!(tag, "attachments.delete");
        assert_eq!(payload["attachmentId"], "delete-ref");
        reply
            .try_send(if success {
                Ok(json!({}))
            } else {
                Err("failure".into())
            })
            .unwrap();
        cx.run_until_parked();
        app.read_with(cx, |app, _| {
            assert_eq!(app.attachments, vec![json!({"id":"other-ref"})]);
            let resources = app.attachment_drafts.shelves.get(&scope.key());
            assert_eq!(resources.map_or(0, Vec::len), usize::from(!success));
        });
    }
}

#[gpui_kit::test]
fn attachment_only_send_reaches_wire_and_waits_for_accepted_receipt(cx: &mut TestAppContext) {
    let (app, cx, requests) = attachment_harness(cx);
    cx.update(|window, cx| app.update(cx, |app, cx| {
        app.composer.update(cx, |input, cx| input.set_value("", window, cx));
        app.attachments.push(json!({"id":"only-file","name":"notes.txt","mimeType":"text/plain","sizeBytes":12,"type":"file"}));
    }));
    cx.run_until_parked();
    let send_bounds = cx.debug_bounds("send").unwrap();
    cx.simulate_click(send_bounds.center(), gpui_kit::Modifiers::none());
    cx.run_until_parked();
    let connection::Command::Call {
        tag,
        payload,
        reply,
    } = requests.try_recv().unwrap()
    else {
        panic!("turn request");
    };
    assert_eq!(tag, "orchestration.dispatchCommand");
    assert_eq!(payload["message"]["text"], "");
    assert_eq!(payload["message"]["attachments"][0]["id"], "only-file");
    app.read_with(cx, |app, _| assert_eq!(app.attachments.len(), 1));
    reply.try_send(Ok(json!({"sequence":1}))).unwrap();
    cx.run_until_parked();
    app.read_with(cx, |app, _| assert!(app.attachments.is_empty()));
}

#[gpui_kit::test]
fn stash_moves_attachment_and_context_ownership_and_restores_selected_record(
    cx: &mut TestAppContext,
) {
    let (app, cx, requests) = attachment_harness(cx);
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.draft_store = None;
            app.composer.update(cx, |input, cx| {
                input.set_value("Inspect [main.rs](src/main.rs)", window, cx)
            });
            app.attachments.push(json!({"id":"stash-file"}));
            app.stash_prompt(false, window, cx);
            assert!(app.attachments.is_empty());
            assert!(app.composer.read(cx).value().is_empty());
            app.stash_attachment_draft();
            app.restore_attachment_draft();
            assert!(app.attachments.is_empty());
            let first = app.composer_stashes()[0].id.clone();
            assert_eq!(
                app.composer_stashes()[0].context.as_ref().unwrap()["records"][0]["path"],
                "src/main.rs"
            );
            app.composer
                .update(cx, |input, cx| input.set_value("Second", window, cx));
            app.stash_prompt(false, window, cx);
            assert_eq!(app.composer_stashes().len(), 2);
            app.restore_stash(&first, window, cx);
            assert!(app.attachments.is_empty());
        })
    });
    cx.run_until_parked();
    let connection::Command::Call { tag, reply, .. } = requests.try_recv().unwrap() else {
        panic!("verify");
    };
    assert_eq!(tag, "assets.createUrl");
    reply
        .try_send(Ok(json!({"relativeUrl":"/api/assets/test"})))
        .unwrap();
    cx.run_until_parked();
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            assert_eq!(app.attachments[0]["id"], "stash-file");
            assert_eq!(
                app.composer.read(cx).value().as_str(),
                "Inspect [main.rs](src/main.rs)"
            );
            assert_eq!(app.composer_stashes()[0].prompt, "Second");
            let second = app.composer_stashes()[0].id.clone();
            app.restore_stash(&second, window, cx);
            assert_eq!(
                app.composer_stashes().len(),
                1,
                "nonempty draft must not lose the stash"
            );
        })
    });
}

#[test]
fn bounded_read_rejects_empty_directory_and_oversize_without_loading_body() {
    let (dir, path) = fixture_file();
    use crate::app::attachments::read_attachment;
    assert!(read_attachment(dir.path(), false).is_err());
    let file = std::fs::File::create(&path).unwrap();
    assert!(read_attachment(&path, false).is_err());
    file.set_len(50 * 1024 * 1024 + 1).unwrap();
    assert!(read_attachment(&path, false).is_err());
    file.set_len(10 * 1024 * 1024 + 1).unwrap();
    let source = read_attachment(&path, true).unwrap();
    assert_eq!(source.len(), 10 * 1024 * 1024 + 1);
    assert!(
        t3code_desktop_gpui::composer_image::prepare(
            "oversize.png".into(),
            "image/png".into(),
            source
        )
        .is_err()
    );
    file.set_len(50 * 1024 * 1024 + 1).unwrap();
    assert!(read_attachment(&path, true).is_err());
}
