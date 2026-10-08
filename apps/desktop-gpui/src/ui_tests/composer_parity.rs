use super::*;

pub(super) fn provider_fixture(instance: &str, label: &str) -> providers::Provider {
    serde_json::from_value(provider_fixture_value(instance, label)).unwrap()
}

pub(super) fn provider_fixture_value(instance: &str, label: &str) -> Value {
    json!({"instanceId":instance,"driver":"codex","displayName":label,"enabled":true,"installed":true,"status":"ready","models":[{"slug":"gpt-5.4","name":"Shared model","capabilities":{"optionDescriptors":[{"id":"effort","label":"Effort","type":"select","options":[{"id":"normal","label":"Normal","isDefault":true}]},{"id":"fast","label":"Fast","type":"boolean"}]}}],"supportedRuntimeModes":["approval-required","full-access"],"showInteractionModeToggle":true,"slashCommands":[{"name":"compact"}],"skills":[]})
}

#[gpui_kit::test]
fn searchable_model_picker_enter_routes_duplicate_slug_to_exact_instance(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.provider_catalog = vec![
                provider_fixture("codex", "Personal account"),
                provider_fixture("work", "Work account"),
            ];
            cx.notify();
        })
    });
    cx.run_until_parked();
    let bounds = cx.debug_bounds("model-catalog").unwrap();
    cx.simulate_click(bounds.center(), gpui_kit::Modifiers::none());
    cx.simulate_input("Work");
    cx.run_until_parked();
    app.read_with(cx, |app, _| {
        assert_eq!(app.catalog_rows(), vec![("work".into(), "gpt-5.4".into())])
    });
    cx.simulate_keystrokes("down enter");
    cx.run_until_parked();
    app.read_with(cx, |app, cx| {
        assert!(!app.catalog_open);
        assert_eq!(app.provider.read(cx).value().as_str(), "work");
    });
    cx.simulate_input("Use selected account");
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    let connection::Command::Call { payload, .. } = requests.try_recv().unwrap() else {
        panic!("send");
    };
    assert_eq!(payload["modelSelection"]["instanceId"], "work");
    assert_eq!(payload["modelSelection"]["model"], "gpt-5.4");
}

#[gpui_kit::test]
fn modifier_send_inserts_newline_and_history_restores_empty_draft(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    cx.update(|_, cx| app.update(cx, |app, cx| {
        app.draft_store = None;
        app.set_send_shortcut(drafts::SendShortcut::ModEnter, cx);
        app.detail.thread = Some(json!({"id":"t","messages":[{"id":"m1","role":"user","text":"First"},{"id":"m2","role":"user","text":"Second"}]}).into());
    }));
    cx.simulate_keystrokes("up");
    app.read_with(cx, |app, cx| {
        assert_eq!(app.composer.read(cx).value().as_str(), "Second")
    });
    cx.simulate_keystrokes("up");
    app.read_with(cx, |app, cx| {
        assert_eq!(app.composer.read(cx).value().as_str(), "First")
    });
    cx.simulate_keystrokes("down down");
    app.read_with(cx, |app, cx| {
        assert!(app.composer.read(cx).value().is_empty())
    });
    cx.simulate_input("First line");
    cx.simulate_keystrokes("enter");
    cx.simulate_input("Second line");
    cx.run_until_parked();
    assert!(requests.try_recv().is_err());
    app.read_with(cx, |app, cx| {
        assert_eq!(
            app.composer.read(cx).value().as_str(),
            "First line\nSecond line"
        )
    });
    cx.simulate_keystrokes("cmd-enter");
    cx.run_until_parked();
    let connection::Command::Call { payload, .. } = requests.try_recv().unwrap() else {
        panic!("modifier send");
    };
    assert_eq!(payload["message"]["text"], "First line\nSecond line");
}

#[gpui_kit::test]
fn workspace_catalog_filters_disabled_skills_and_emits_structured_skill_context(
    cx: &mut TestAppContext,
) {
    let (app, cx, requests) = harness(cx);
    cx.update(|_, cx| {
        app.update(cx, |app, _| {
            app.provider_catalog = vec![provider_fixture("codex", "Personal")]
        })
    });
    cx.simulate_input("$");
    cx.run_until_parked();
    let connection::Command::Call {
        tag,
        payload,
        reply,
    } = requests.try_recv().unwrap()
    else {
        panic!("catalog refresh");
    };
    assert_eq!(tag, "server.refreshProviders");
    assert_eq!(payload["instanceId"], "codex");
    assert_eq!(payload["cwd"], "/tmp/fixture");
    let mut provider = serde_json::to_value(json!({"instanceId":"codex","driver":"codex","enabled":true,"installed":true,"status":"ready","models":[{"slug":"gpt-5.4","name":"Shared model"}]})).unwrap();
    provider["workspaceSnapshots"] = json!([{"cwd":"/tmp/fixture","slashCommands":[],"skills":[{"name":"ui","path":"/tmp/fixture/ui.md","enabled":true},{"name":"disabled","path":"/tmp/no.md","enabled":false},{"name":"agent-only","path":"/tmp/no.md","enabled":true,"userInvocable":false}]}]);
    reply.try_send(Ok(json!({"providers":[provider]}))).unwrap();
    cx.run_until_parked();
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    app.read_with(cx, |app, cx| {
        assert_eq!(app.composer.read(cx).value().as_str(), "$ui ")
    });
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    let connection::Command::Call { payload, .. } = requests.try_recv().unwrap() else {
        panic!("skill send");
    };
    assert_eq!(payload["message"]["context"]["records"][0]["kind"], "skill");
    assert_eq!(payload["message"]["context"]["records"][0]["name"], "ui");
}

#[gpui_kit::test]
fn compact_dispatch_preserves_prompt_and_attachment_ownership(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.provider_catalog = vec![provider_fixture("codex", "Personal")];
            app.composer
                .update(cx, |input, cx| input.set_value("Unsent draft", window, cx));
            app.attachments.push(json!({"id":"keep-ref"}));
            app.compact_context(cx);
        })
    });
    cx.run_until_parked();
    let connection::Command::Call { payload, reply, .. } = requests.try_recv().unwrap() else {
        panic!("compact");
    };
    assert_eq!(payload["message"]["text"], "/compact");
    assert_eq!(payload["message"]["attachments"], json!([]));
    reply.try_send(Ok(json!({"sequence":1}))).unwrap();
    cx.run_until_parked();
    app.read_with(cx, |app, cx| {
        assert_eq!(app.composer.read(cx).value().as_str(), "Unsent draft");
        assert_eq!(app.attachments, vec![json!({"id":"keep-ref"})]);
    });
}

#[gpui_kit::test]
fn large_clipboard_paste_offers_text_override_without_upload(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    let text = "世界".repeat(6000);
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            assert!(app.paste_composer(&gpui_kit::ClipboardItem::new_string(text.clone()), cx))
        })
    });
    cx.run_until_parked();
    let bounds = cx.debug_bounds("paste-as-text").unwrap();
    cx.simulate_click(bounds.center(), gpui_kit::Modifiers::none());
    cx.run_until_parked();
    app.read_with(cx, |app, cx| {
        assert_eq!(app.composer.read(cx).value().as_str(), text);
        assert!(app.attachments.is_empty());
        assert!(app.composer_state.pending_paste.is_none());
    });
    assert!(requests.try_recv().is_err());
}

#[gpui_kit::test]
fn stash_write_failure_retains_original_prompt_and_resources(cx: &mut TestAppContext) {
    let (app, cx, _requests) = harness(cx);
    let dir = tempfile::tempdir().unwrap();
    let not_directory = dir.path().join("file");
    let store = drafts::Drafts::open(not_directory.join("drafts.json")).unwrap();
    std::fs::write(&not_directory, "not a directory").unwrap();
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.draft_store = Some(store);
            app.composer
                .update(cx, |input, cx| input.set_value("Keep me", window, cx));
            app.attachments.push(json!({"id":"owned-ref"}));
            app.stash_prompt(false, window, cx);
            assert_eq!(
                app.composer.read(cx).value().as_str(),
                "Keep me",
                "not cleared before the durable write"
            );
        })
    });
    cx.run_until_parked();
    app.read_with(cx, |app, cx| {
        assert_eq!(app.composer.read(cx).value().as_str(), "Keep me");
        assert_eq!(app.attachments, vec![json!({"id":"owned-ref"})]);
        assert!(app.composer_stashes().is_empty());
        assert!(app.storage_error.is_some());
    });
}

#[gpui_kit::test]
fn picked_file_chip_deletes_atomically_and_undo_restores_sent_context(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    cx.simulate_input("@src");
    cx.run_until_parked();
    let connection::Command::Call { tag, reply, .. } = requests.try_recv().unwrap() else {
        panic!("file search");
    };
    assert_eq!(tag, "projects.searchEntries");
    reply
        .try_send(Ok(
            json!({"entries":[{"path":"src/main.rs","kind":"file"}],"truncated":false}),
        ))
        .unwrap();
    cx.run_until_parked();
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    app.read_with(cx, |app, cx| {
        assert_eq!(app.composer.read(cx).tokens().len(), 1)
    });
    cx.simulate_keystrokes("backspace backspace");
    cx.run_until_parked();
    app.read_with(cx, |app, cx| {
        assert!(app.composer.read(cx).tokens().is_empty());
        assert!(!app.composer.read(cx).value().contains("main.rs"));
    });
    cx.simulate_keystrokes("cmd-z");
    cx.run_until_parked();
    app.read_with(cx, |app, cx| {
        assert_eq!(app.composer.read(cx).tokens().len(), 1)
    });
    cx.update(|window, cx| app.update(cx, |app, cx| app.send_prompt(window, cx)));
    cx.run_until_parked();
    let connection::Command::Call { payload, .. } = requests.try_recv().unwrap() else {
        panic!("send");
    };
    assert_eq!(
        payload["message"]["context"]["records"][0]["path"],
        "src/main.rs"
    );
}

#[gpui_kit::test]
fn durable_stash_clears_only_after_write_and_rehydrates_chip(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("drafts.json");
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.draft_store = Some(drafts::Drafts::open(path.clone()).unwrap());
            app.composer.update(cx, |input, cx| {
                input.set_value("See [main.rs](src/main.rs)", window, cx)
            });
            app.attachments.push(json!({"id":"stashed-file"}));
            app.stash_prompt(false, window, cx);
            assert!(!app.composer.read(cx).value().is_empty());
            assert_eq!(app.attachments.len(), 1);
        })
    });
    cx.run_until_parked();
    let saved = drafts::Drafts::open(path).unwrap();
    let origin = app.read_with(cx, |app, _| app.environment_scope.clone());
    assert_eq!(
        saved.stashes(&origin)[0].attachments[0]["id"],
        "stashed-file"
    );
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            assert!(app.composer.read(cx).value().is_empty());
            assert!(app.attachments.is_empty());
            app.stash_prompt(true, window, cx);
            assert!(app.attachments.is_empty(), "preflight has not accepted yet");
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
    app.read_with(cx, |app, cx| {
        assert_eq!(app.composer.read(cx).tokens().len(), 1);
        assert_eq!(app.attachments[0]["id"], "stashed-file");
    });
}

#[gpui_kit::test]
fn clipboard_file_upload_remains_owned_by_original_draft(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("notes.txt");
    std::fs::write(&path, "clipboard bytes").unwrap();
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.environment_scope = "https://clipboard.example".into();
            let item = gpui_kit::ClipboardItem {
                entries: vec![gpui_kit::ClipboardEntry::ExternalPaths(
                    gpui_kit::ExternalPaths(vec![path].into()),
                )],
            };
            assert!(app.paste_composer(&item, cx));
        })
    });
    cx.run_until_parked();
    let connection::Command::Call { tag, reply, .. } = requests.try_recv().unwrap() else {
        panic!("sign upload");
    };
    assert_eq!(tag, "attachments.createUploadUrl");
    reply.try_send(Ok(json!({"attachmentId":"pending-00000000-0000-4000-8000-000000000001-txt","relativeUrl":"/api/attachments/upload/claims.signature"}))).unwrap();
    cx.run_until_parked();
    let connection::Command::Upload { bytes, reply, .. } = requests.try_recv().unwrap() else {
        panic!("upload");
    };
    assert_eq!(bytes, b"clipboard bytes");
    cx.update(|_, cx| {
        app.update(cx, |app, _| {
            app.stash_attachment_draft();
            app.selected_thread = Some("other".into());
            app.restore_attachment_draft();
            app.attachments.push(json!({"id":"other-file"}));
        })
    });
    reply.try_send(Ok(())).unwrap();
    cx.run_until_parked();
    app.read_with(cx, |app, _| {
        assert_eq!(app.attachments[0]["id"], "other-file");
        assert_eq!(app.attachment_drafts.shelves[&("https://clipboard.example".into(), "t".into())][0]["name"], "notes.txt");
    });
}

#[gpui_kit::test]
fn clipboard_image_emits_image_reference_and_keeps_thumbnail_with_stash(cx: &mut TestAppContext) {
    use base64::Engine as _;
    let (app, cx, requests) = harness(cx);
    let bytes = base64::engine::general_purpose::STANDARD.decode("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=").unwrap();
    let image = gpui_kit::Image::from_bytes(gpui_kit::ImageFormat::Png, bytes.clone());
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.environment_scope = "https://clipboard.example".into();
            app.draft_store = None;
            assert!(app.paste_composer(&gpui_kit::ClipboardItem::new_image(&image), cx));
        })
    });
    cx.run_until_parked();
    let connection::Command::Call { payload, reply, .. } = requests.try_recv().unwrap() else {
        panic!("image sign");
    };
    assert_eq!(payload["type"], "image");
    assert_eq!(payload["mimeType"], "image/png");
    reply.try_send(Ok(json!({"attachmentId":"pending-00000000-0000-4000-8000-000000000001","relativeUrl":"/api/attachments/upload/claims.signature"}))).unwrap();
    cx.run_until_parked();
    let connection::Command::Upload {
        bytes: sent, reply, ..
    } = requests.try_recv().unwrap()
    else {
        panic!("image upload");
    };
    assert_eq!(sent, bytes);
    reply.try_send(Ok(())).unwrap();
    cx.run_until_parked();
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            assert_eq!(app.attachments[0]["type"], "image");
            assert_eq!(app.composer_state.previews.len(), 1);
            app.stash_prompt(false, window, cx);
            assert!(app.attachments.is_empty());
            assert_eq!(app.composer_state.previews.len(), 1);
            app.stash_prompt(true, window, cx);
            assert!(app.attachments.is_empty());
        })
    });
    cx.run_until_parked();
    let connection::Command::Call { tag, reply, .. } = requests.try_recv().unwrap() else {
        panic!("verify image");
    };
    assert_eq!(tag, "assets.createUrl");
    reply
        .try_send(Ok(json!({"relativeUrl":"/api/assets/test"})))
        .unwrap();
    cx.run_until_parked();
    app.read_with(cx, |app, _| {
        assert_eq!(app.attachments[0]["type"], "image");
        assert_eq!(app.composer_state.previews.len(), 1);
    });
}

#[gpui_kit::test]
fn unavailable_saved_trait_reset_removes_it_from_emitted_turn(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    cx.update(|_, cx| app.update(cx, |app, cx| {
        app.provider_catalog = vec![provider_fixture("codex", "Personal")];
        app.model_override = Some(json!({"instanceId":"codex","model":"gpt-5.4","options":[{"id":"effort","value":"retired"}]}));
        cx.notify();
    }));
    cx.run_until_parked();
    let bounds = cx.debug_bounds("model-catalog").unwrap();
    cx.simulate_click(bounds.center(), gpui_kit::Modifiers::none());
    cx.run_until_parked();
    let bounds = cx.debug_bounds("reset-unavailable-options").unwrap();
    cx.simulate_click(bounds.center(), gpui_kit::Modifiers::none());
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            assert!(!app.unavailable_model_options(cx));
            app.catalog_open = false;
            app.composer.update(cx, |input, cx| {
                input.set_value("Send after reset", window, cx)
            });
            app.send_prompt(window, cx);
        })
    });
    cx.run_until_parked();
    let connection::Command::Call { payload, .. } = requests.try_recv().unwrap() else {
        panic!("send reset");
    };
    assert!(payload["modelSelection"].get("options").is_none());
}

#[gpui_kit::test]
fn prompt_length_and_provider_modes_are_validated_before_dispatch(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            let mut provider = provider_fixture("codex", "Personal");
            provider.supported_runtime_modes = Some(vec!["auto-accept-edits".into()]);
            provider.show_interaction_mode_toggle = Some(false);
            app.provider_catalog = vec![provider];
            app.composer.update(cx, |input, cx| {
                input.set_value("😀".repeat(60_001), window, cx)
            });
            assert!(app.composer_message(cx).is_err());
            app.change_thread_mode("thread.interaction-mode.set", "interactionMode", "plan", cx);
            app.composer
                .update(cx, |input, cx| input.set_value("Small prompt", window, cx));
            let (_, message) = app.composer_message(cx).unwrap();
            assert_eq!(message.runtime_mode, "auto-accept-edits");
            assert_eq!(message.interaction_mode, "default");
        })
    });
    assert!(requests.try_recv().is_err());
}
