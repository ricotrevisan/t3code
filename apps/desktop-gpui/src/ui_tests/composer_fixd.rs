use super::*;
use gpui_kit::InputEvent as _;

#[gpui_kit::test]
fn restored_resources_survive_shutdown_reopen_and_accepted_consumption(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("drafts.json");
    for prompt in ["", "Inspect $ui"] {
        cx.update(|window, cx| {
            app.update(cx, |app, cx| {
                let mut store = drafts::Drafts::open(path.clone()).unwrap();
                let record = drafts::StashEntry::new(
                    prompt.into(),
                    vec![json!({"id":"durable-image","type":"image"})],
                );
                let id = record.id.clone();
                store.stash_record(&app.environment_scope, record);
                store.write_snapshot().unwrap().persist().unwrap();
                app.draft_store = Some(store);
                app.restore_stash(&id, window, cx);
                assert_eq!(app.composer_stashes().len(), 1);
            })
        });
        cx.run_until_parked();
        let connection::Command::Call { tag, reply, .. } = requests.try_recv().unwrap() else {
            panic!("preflight");
        };
        assert_eq!(tag, "assets.createUrl");
        reply
            .try_send(Ok(json!({"relativeUrl":"/api/assets/verified"})))
            .unwrap();
        cx.run_until_parked();
        let origin = app.read_with(cx, |app, _| {
            app.draft_store
                .as_ref()
                .unwrap()
                .write_snapshot()
                .unwrap()
                .persist()
                .unwrap();
            app.environment_scope.clone()
        });
        let reopened = drafts::Drafts::open(path.clone()).unwrap();
        assert!(reopened.stashes(&origin).is_empty());
        assert_eq!(reopened.get(&origin, "t"), prompt);
        assert_eq!(reopened.resources(&origin, "t")[0]["id"], "durable-image");
        cx.update(|window, cx| {
            app.update(cx, |app, cx| {
                app.draft_store = Some(reopened);
                app.attachments.clear();
                app.attachment_drafts.shelves.clear();
                app.restore_attachment_draft();
                assert_eq!(app.attachments[0]["id"], "durable-image");
                let sent = app.attachments.clone();
                app.consume_sent_attachments(&origin, "t", &sent);
                app.draft_store
                    .as_ref()
                    .unwrap()
                    .write_snapshot()
                    .unwrap()
                    .persist()
                    .unwrap();
                app.composer
                    .update(cx, |input, cx| input.set_value("", window, cx));
            })
        });
        assert!(
            drafts::Drafts::open(path.clone())
                .unwrap()
                .resources(&origin, "t")
                .is_empty()
        );
    }
}

#[gpui_kit::test]
fn failed_text_attachment_keeps_original_pending_paste_for_retry(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    let text = "original clipboard".repeat(3000);
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.paste_composer(&gpui_kit::ClipboardItem::new_string(text.clone()), cx);
            app.finish_large_paste(false, window, cx);
        })
    });
    cx.run_until_parked();
    let connection::Command::Call { tag, reply, .. } = requests.try_recv().unwrap() else {
        panic!("upload prepare");
    };
    assert_eq!(tag, "attachments.createUploadUrl");
    reply.try_send(Err("offline".into())).unwrap();
    cx.run_until_parked();
    app.read_with(cx, |app, _| {
        assert_eq!(app.composer_state.pending_paste.as_ref().unwrap().1, text);
        assert!(!app.attachment_drafts.uploading);
    });
    let bounds = cx.debug_bounds("paste-as-text").unwrap();
    cx.simulate_click(bounds.center(), gpui_kit::Modifiers::none());
    app.read_with(cx, |app, cx| {
        assert_eq!(app.composer.read(cx).value().as_str(), text)
    });
}

#[gpui_kit::test]
fn command_catalog_late_workspace_receipt_allows_automatic_retry(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    cx.update(|_, cx| {
        app.update(cx, |app, _| {
            app.provider_catalog = vec![super::composer_parity::provider_fixture(
                "codex", "Personal",
            )];
        })
    });
    cx.simulate_input("$");
    cx.run_until_parked();
    let connection::Command::Call { reply, .. } = requests.try_recv().unwrap() else {
        panic!("catalog");
    };
    cx.update(|_, cx| {
        app.update(cx, |app, _| {
            app.shell.projects[0].workspace_root = "/tmp/other".into()
        })
    });
    reply.try_send(Ok(json!({"providers":[]}))).unwrap();
    cx.run_until_parked();
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            assert!(!app.composer_state.command_loading);
            assert!(app.composer_state.command_scope.is_none());
            app.shell.projects[0].workspace_root = "/tmp/fixture".into();
            app.refresh_composer_catalog(false, cx);
        })
    });
    cx.run_until_parked();
    let connection::Command::Call { tag, .. } = requests.try_recv().unwrap() else {
        panic!("retry");
    };
    assert_eq!(tag, "server.refreshProviders");
}

#[gpui_kit::test]
fn shortcut_empty_composer_selects_older_stash_in_picker(cx: &mut TestAppContext) {
    let (app, cx, _) = harness(cx);
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.draft_store = None;
            for prompt in ["older", "newer"] {
                app.composer
                    .update(cx, |input, cx| input.set_value(prompt, window, cx));
                app.stash_prompt(false, window, cx);
            }
            app.stash_prompt(true, window, cx);
        })
    });
    cx.run_until_parked();
    assert!(cx.debug_bounds("shortcut-stash-picker").is_some());
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    app.read_with(cx, |app, cx| {
        assert_eq!(app.composer.read(cx).value().as_str(), "older");
        assert_eq!(app.composer_stashes()[0].prompt, "newer");
    });
}

#[gpui_kit::test]
fn missing_stash_resource_retains_durable_record_and_empty_draft(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.draft_store = None;
            app.attachments.push(json!({"id":"expired"}));
            app.stash_prompt(false, window, cx);
            app.stash_prompt(true, window, cx);
        })
    });
    cx.run_until_parked();
    let connection::Command::Call { reply, .. } = requests.try_recv().unwrap() else {
        panic!("verify");
    };
    reply.try_send(Err("AttachmentNotFound".into())).unwrap();
    cx.run_until_parked();
    app.read_with(cx, |app, cx| {
        assert_eq!(app.composer_stashes().len(), 1);
        assert!(app.attachments.is_empty());
        assert!(app.composer.read(cx).value().is_empty());
    });
}

fn png_preview(color: u8) -> std::sync::Arc<gpui_kit::Image> {
    let image = image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
        16,
        16,
        image::Rgba([color, 0, 0, 255]),
    ));
    let mut bytes = std::io::Cursor::new(Vec::new());
    image.write_to(&mut bytes, image::ImageFormat::Png).unwrap();
    crate::composer_thumbnail::thumbnail(&bytes.into_inner()).unwrap()
}

#[gpui_kit::test]
fn decoded_composer_asset_is_released_on_consumption_and_budget_eviction(cx: &mut TestAppContext) {
    let (app, cx, _) = harness(cx);
    let preview = png_preview(1);
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.composer_state.cache_preview(
                app.environment_scope.clone(),
                "image".into(),
                preview.clone(),
            );
            app.attachments
                .push(json!({"id":"image","type":"image","name":"preview.png"}));
            cx.notify();
        })
    });
    cx.run_until_parked();
    app.read_with(cx, |_, cx| assert!(preview.is_asset_cached(cx)));
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            for color in 2..=130 {
                app.composer_state.cache_preview(
                    app.environment_scope.clone(),
                    format!("image-{color}"),
                    png_preview(color),
                );
            }
            cx.notify();
        })
    });
    cx.run_until_parked();
    app.read_with(cx, |app, cx| {
        assert!(app.composer_state.previews.len() <= 128);
        // HashMap budget eviction is unordered. Explicit consumption must release even if retained.
        if !app
            .composer_state
            .previews
            .contains_key(&(app.environment_scope.clone(), "image".into()))
        {
            assert!(!preview.is_asset_cached(cx));
        }
    });
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            let origin = app.environment_scope.clone();
            app.consume_sent_attachments(&origin, "t", &[json!({"id":"image"})]);
            cx.notify();
        })
    });
    cx.run_until_parked();
    app.read_with(cx, |_, cx| assert!(!preview.is_asset_cached(cx)));
}

#[test]
fn animated_thumbnail_is_single_static_bounded_frame() {
    let mut bytes = Vec::new();
    {
        let mut encoder = image::codecs::gif::GifEncoder::new(&mut bytes);
        for color in [20, 80, 150] {
            encoder
                .encode_frame(image::Frame::new(image::RgbaImage::from_pixel(
                    512,
                    512,
                    image::Rgba([color, 0, 0, 255]),
                )))
                .unwrap();
        }
    }
    let preview = crate::composer_thumbnail::thumbnail(&bytes).unwrap();
    assert_eq!(preview.format, gpui_kit::ImageFormat::Png);
    let decoded = image::load_from_memory(&preview.bytes).unwrap();
    assert_eq!((decoded.width(), decoded.height()), (256, 256));
    assert_eq!(decoded.to_rgba8().get_pixel(0, 0)[0], 20);
}

#[gpui_kit::test]
fn provider_sidebar_and_arrow_selection_scroll_long_model_list(cx: &mut TestAppContext) {
    let (app, cx, _) = harness(cx);
    cx.update(|_, cx| app.update(cx, |app, cx| {
        let mut work = super::composer_parity::provider_fixture_value("work", "Work");
        work["models"] = json!((0..40).map(|index| json!({"slug":format!("model-{index}"),"name":format!("Model {index}")})).collect::<Vec<_>>());
        app.provider_catalog = vec![super::composer_parity::provider_fixture("codex", "Personal"), serde_json::from_value(work).unwrap()];
        cx.notify();
    }));
    cx.run_until_parked();
    let bounds = cx.debug_bounds("model-catalog").unwrap();
    cx.simulate_click(bounds.center(), gpui_kit::Modifiers::none());
    cx.run_until_parked();
    let bounds = cx.debug_bounds("catalog-provider-work").unwrap();
    cx.simulate_click(bounds.center(), gpui_kit::Modifiers::none());
    cx.run_until_parked();
    for _ in 0..30 {
        cx.simulate_keystrokes("down");
        cx.update(|window, cx| {
            window.simulate_next_frame(cx);
        });
        cx.run_until_parked();
    }
    app.read_with(cx, |app, _| {
        assert_eq!(app.catalog_rows().len(), 40);
        assert_eq!(app.composer_state.catalog_index, 30);
        assert!(app.composer_state.catalog_scroll.offset().y < px(0.));
    });
    let viewport = cx.debug_bounds("catalog-model-scroll").unwrap();
    let row = cx.debug_bounds("catalog-work-30").unwrap();
    assert!(row.intersects(&viewport));
    cx.simulate_keystrokes("enter");
    app.read_with(cx, |app, cx| {
        assert_eq!(app.provider.read(cx).value().as_str(), "work");
        assert_eq!(app.model.read(cx).value().as_str(), "model-30");
    });
}

#[gpui_kit::test]
fn native_external_file_drop_enters_captured_upload_seam(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("dropped.txt");
    std::fs::write(&path, "dropped bytes").unwrap();
    let position = cx.debug_bounds("composer").unwrap().center();
    cx.update(|window, cx| {
        window.dispatch_event(
            gpui_kit::FileDropEvent::Entered {
                position,
                paths: gpui_kit::ExternalPaths(vec![path].into()),
            }
            .to_platform_input(),
            cx,
        );
        window.dispatch_event(
            gpui_kit::FileDropEvent::Submit { position }.to_platform_input(),
            cx,
        );
    });
    cx.run_until_parked();
    let connection::Command::Call {
        tag,
        payload,
        reply,
    } = requests.try_recv().unwrap()
    else {
        panic!("drop prepare");
    };
    assert_eq!(tag, "attachments.createUploadUrl");
    assert_eq!(payload["name"], "dropped.txt");
    reply.try_send(Err("test refusal".into())).unwrap();
    cx.run_until_parked();
    app.read_with(cx, |app, _| assert!(!app.attachment_drafts.uploading));
}
