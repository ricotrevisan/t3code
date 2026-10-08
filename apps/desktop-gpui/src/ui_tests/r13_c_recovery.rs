use super::*;

#[gpui_kit::test]
fn r13_c_uploaded_composer_image_expands_and_closes_without_consuming_ref(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    let mut encoded = std::io::Cursor::new(Vec::new());
    image::DynamicImage::new_rgb8(8, 8)
        .write_to(&mut encoded, image::ImageFormat::Png)
        .unwrap();
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.environment_scope = "https://attachments.example".into();
            let scope = app.attachment_scope().unwrap();
            app.attach_inputs(
                scope,
                vec![crate::app::attachments::AttachmentInput::Bytes {
                    name: "image.png".into(),
                    bytes: encoded.into_inner(),
                    source: None,
                }],
                cx,
            );
        })
    });
    cx.run_until_parked();
    let connection::Command::Call {
        tag,
        payload,
        reply,
    } = requests.try_recv().unwrap()
    else {
        panic!("image preparation");
    };
    assert_eq!(tag, "attachments.createUploadUrl");
    assert_eq!(payload["type"], "image");
    reply.try_send(Ok(json!({"attachmentId":"pending-00000000-0000-4000-8000-000000000001-png","relativeUrl":"/api/attachments/upload/claims.signature"}))).unwrap();
    cx.run_until_parked();
    let connection::Command::Upload { reply, .. } = requests.try_recv().unwrap() else {
        panic!("image POST");
    };
    reply.try_send(Ok(())).unwrap();
    cx.run_until_parked();
    let bounds = cx.debug_bounds("expand-preview-0").unwrap();
    cx.simulate_click(bounds.center(), gpui_kit::Modifiers::none());
    cx.run_until_parked();
    assert!(cx.debug_bounds("composer-preview-close").is_some());
    app.read_with(cx, |app, _| {
        assert!(app.composer_state.expanded_preview.is_some())
    });
    let bounds = cx.debug_bounds("composer-preview-close").unwrap();
    cx.simulate_click(bounds.center(), gpui_kit::Modifiers::none());
    cx.run_until_parked();
    app.read_with(cx, |app, _| {
        assert!(app.composer_state.expanded_preview.is_none());
        assert_eq!(app.attachments.len(), 1);
    });
}

#[gpui_kit::test]
fn r13_c_stash_checks_each_resource_and_explicitly_removes_only_missing_one(
    cx: &mut TestAppContext,
) {
    let (app, cx, requests) = harness(cx);
    let id = cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.draft_store = None;
            let mut entry = t3code_desktop_gpui::drafts::StashEntry::new(
                "restore context".into(),
                vec![
                    json!({"id":"missing","name":"missing.txt"}),
                    json!({"id":"present","name":"present.txt"}),
                ],
            );
            let id = entry.id.clone();
            entry.context = None;
            app.attachment_drafts
                .stashes
                .entry(app.environment_scope.clone())
                .or_default()
                .push(entry);
            app.restore_stash(&id, window, cx);
            id
        })
    });
    cx.run_until_parked();
    for (expected, result) in [
        ("missing", Err("Missing".into())),
        ("present", Ok(json!({"relativeUrl":"/api/assets/present"}))),
    ] {
        let connection::Command::Call {
            tag,
            payload,
            reply,
        } = requests.try_recv().unwrap()
        else {
            panic!("resource verification");
        };
        assert_eq!(tag, "assets.createUrl");
        assert_eq!(payload["resource"]["attachmentId"], expected);
        reply.try_send(result).unwrap();
        cx.run_until_parked();
    }
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            let key = (app.environment_scope.clone(), id.clone());
            assert_eq!(app.attachment_drafts.missing_stash_resources[&key].len(), 1);
            assert!(app.composer.read(cx).value().is_empty());
            app.discard_unavailable_stash_resource(&id, "missing", cx);
            assert_eq!(app.composer_stashes()[0].attachments.len(), 1);
            app.restore_stash(&id, window, cx);
        })
    });
    cx.run_until_parked();
    let connection::Command::Call {
        tag,
        payload,
        reply,
    } = requests.try_recv().unwrap()
    else {
        panic!("verify available on restore");
    };
    assert_eq!(tag, "assets.createUrl");
    assert_eq!(payload["resource"]["attachmentId"], "present");
    reply
        .try_send(Ok(json!({"relativeUrl":"/api/assets/present"})))
        .unwrap();
    cx.run_until_parked();
    app.read_with(cx, |app, cx| {
        assert_eq!(
            app.attachments,
            vec![json!({"id":"present","name":"present.txt"})]
        );
        assert_eq!(app.composer.read(cx).value().as_str(), "restore context");
        assert!(app.composer_stashes().is_empty());
    });
}

#[gpui_kit::test]
fn r13_c_restored_preview_requests_fresh_url_and_failed_retry_keeps_ref(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    let attachment = json!({"type":"image","id":"image-one","name":"one.png","mimeType":"image/png","sizeBytes":4});
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.attachments.push(attachment.clone());
            app.recover_attachment_preview("image-one".into(), cx);
        })
    });
    cx.run_until_parked();
    let connection::Command::Call {
        tag,
        payload,
        reply,
    } = requests.try_recv().unwrap()
    else {
        panic!("asset request");
    };
    assert_eq!(tag, "assets.createUrl");
    assert_eq!(
        payload,
        json!({"resource":{"_tag":"attachment","attachmentId":"image-one"}})
    );
    reply.try_send(Err("Expired resource".into())).unwrap();
    cx.run_until_parked();
    app.read_with(cx, |app, _| {
        assert_eq!(app.attachments, vec![attachment]);
        assert!(app.composer_state.preview_loading.is_empty());
        assert!(
            app.composer_state
                .preview_errors
                .contains(&(app.environment_scope.clone(), "image-one".into()))
        );
    });
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.recover_attachment_preview("image-one".into(), cx)
        })
    });
    cx.run_until_parked();
    let connection::Command::Call { tag, reply, .. } = requests.try_recv().unwrap() else {
        panic!("retry asset request");
    };
    assert_eq!(tag, "assets.createUrl");
    reply.try_send(Err("Still missing".into())).unwrap();
    cx.run_until_parked();
}

#[gpui_kit::test]
fn r13_c_history_waits_for_visual_wrapped_edge_and_never_replaces_typed_draft(
    cx: &mut TestAppContext,
) {
    let (app, cx, requests) = harness(cx);
    let long = "wrapped word ".repeat(80);
    cx.update(|_, cx| app.update(cx, |app, _| {
        app.detail.thread = Some(json!({"id":"t","messages":[{"id":"old","role":"user","text":"Older"},{"id":"new","role":"user","text":long.trim_end()}]}).into());
    }));
    cx.simulate_input("Typed draft");
    cx.simulate_keystrokes("up");
    cx.run_until_parked();
    app.read_with(cx, |app, cx| {
        assert_eq!(app.composer.read(cx).value().as_str(), "Typed draft")
    });
    cx.simulate_keystrokes("cmd-a backspace up");
    cx.run_until_parked();
    app.read_with(cx, |app, cx| {
        assert_eq!(app.composer.read(cx).value().as_str(), long.trim_end())
    });
    cx.simulate_keystrokes("up");
    cx.run_until_parked();
    app.read_with(cx, |app, cx| {
        assert_eq!(app.composer.read(cx).value().as_str(), long.trim_end())
    });
    cx.simulate_keystrokes("cmd-left up");
    cx.run_until_parked();
    app.read_with(cx, |app, cx| {
        assert_eq!(app.composer.read(cx).value().as_str(), "Older")
    });
    assert!(requests.try_recv().is_err());
}

#[gpui_kit::test]
fn r13_c_shift_enter_continues_and_exits_markdown_list_with_real_input(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    cx.simulate_input("- [x] done");
    cx.simulate_keystrokes("shift-enter");
    cx.run_until_parked();
    app.read_with(cx, |app, cx| {
        assert_eq!(app.composer.read(cx).value().as_str(), "- [x] done\n- [ ] ")
    });
    cx.simulate_keystrokes("shift-enter");
    cx.run_until_parked();
    app.read_with(cx, |app, cx| {
        assert_eq!(app.composer.read(cx).value().as_str(), "- [x] done\n")
    });
    assert!(requests.try_recv().is_err());
}

#[gpui_kit::test]
fn r13_c_prompt_injected_trait_changes_prompt_not_wire_options_and_reverses(
    cx: &mut TestAppContext,
) {
    let (app, cx, requests) = harness(cx);
    cx.update(|window, cx| app.update(cx, |app, cx| {
        let mut provider = super::composer_parity::provider_fixture("codex", "Personal");
        provider.models[0].capabilities = json!({"optionDescriptors":[{"id":"effort","label":"Effort","type":"select","promptInjectedValues":["ultrathink"],"options":[{"id":"high","label":"High"},{"id":"ultrathink","label":"Ultrathink"}]}]});
        app.provider_catalog = vec![provider];
        app.composer.update(cx, |input, cx| input.set_value("Explain", window, cx));
        app.choose_composer_trait("effort", json!("ultrathink"), window, cx);
        assert_eq!(app.composer.read(cx).value().as_str(), "Ultrathink:\nExplain");
        app.choose_composer_trait("effort", json!("high"), window, cx);
        assert_eq!(app.composer.read(cx).value().as_str(), "Explain");
        app.send_prompt(window, cx);
    }));
    cx.run_until_parked();
    let connection::Command::Call {
        tag,
        payload,
        reply,
    } = requests.try_recv().unwrap()
    else {
        panic!("trait turn");
    };
    assert_eq!(tag, "orchestration.dispatchCommand");
    assert_eq!(
        payload["modelSelection"]["options"][0],
        json!({"id":"effort","value":"high"})
    );
    assert_eq!(payload["message"]["text"], "Explain");
    reply.try_send(Err("Test rejection".into())).unwrap();
    cx.run_until_parked();
}
