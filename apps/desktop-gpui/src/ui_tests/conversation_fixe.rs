use super::*;
use std::sync::Arc;

fn preview() -> Arc<gpui_kit::Image> {
    let mut png = std::io::Cursor::new(Vec::new());
    image::DynamicImage::new_rgba8(8, 8)
        .write_to(&mut png, image::ImageFormat::Png)
        .unwrap();
    crate::views::message_media::bounded_preview(&png.into_inner(), gpui_kit::ImageFormat::Png)
        .unwrap()
}

#[gpui_kit::test]
fn conversation_fixe_reset_releases_real_gpui_decoded_asset(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    let image = preview();
    cx.update(|_, cx| app.update(cx, |app, cx| {
        app.apply_thread_update("t", snapshot(vec![json!({"id":"u", "role":"user", "text":"image", "attachments":[{"id":"pic", "type":"image", "mimeType":"image/png"}]})], "idle"), cx).unwrap();
        cx.notify();
    }));
    cx.run_until_parked();
    let connection::Command::Call { reply, .. } = requests.try_recv().unwrap() else {
        panic!("sign image")
    };
    reply.send_blocking(Err("fixture".into())).unwrap();
    cx.run_until_parked();
    let key = app.read_with(cx, |app, _| {
        crate::views::message_media::media_key(&app.environment_scope, "t", "pic")
    });
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.conversation_presentation.media.insert(
                key,
                crate::views::message_media::MediaPreview::Ready(image.clone()),
            );
            app.conversation_scroll.remeasure();
            cx.notify();
        })
    });
    cx.run_until_parked();
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            assert!(image.is_asset_cached(cx));
            let owner = (app.environment_scope.clone(), "composer-owner".into());
            app.composer_state
                .previews
                .insert(owner.clone(), image.clone());
            app.conversation_presentation
                .needs_preparation(1, 1, "a", &Default::default(), None);
            for image in std::mem::take(&mut app.conversation_presentation.retired_images) {
                app.release_message_image(image, cx);
            }
            assert!(
                image.is_asset_cached(cx),
                "conversation must not evict composer-owned hash"
            );
            assert_eq!(app.conversation_presentation.released_images, 0);
            app.composer_state.previews.remove(&owner);
            app.release_message_image(image.clone(), cx);
            assert!(!image.is_asset_cached(cx));
            assert_eq!(app.conversation_presentation.released_images, 1);
        })
    });
    cx.run_until_parked();
    cx.update(|_, cx| assert!(!image.is_asset_cached(cx)));
}

#[gpui_kit::test]
fn conversation_fixe_old_same_thread_receipt_cannot_overwrite_new_preview(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    let resource = json!({"_tag":"attachment", "threadId":"t", "attachmentId":"pic"});
    let key = "same-key".to_owned();
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.request_message_image(key.clone(), resource.clone(), "image/png", cx);
        })
    });
    cx.run_until_parked();
    let connection::Command::Call { reply: old, .. } = requests.try_recv().unwrap() else {
        panic!("old image request")
    };
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.conversation_presentation
                .needs_preparation(1, 1, "other", &Default::default(), None);
            app.conversation_presentation
                .needs_preparation(1, 1, "t", &Default::default(), None);
            app.request_message_image(key.clone(), resource.clone(), "image/png", cx);
        })
    });
    cx.run_until_parked();
    let connection::Command::Call { reply: new, .. } = requests.try_recv().unwrap() else {
        panic!("new image request")
    };
    new.send_blocking(Err("new fixture download".into()))
        .unwrap();
    cx.run_until_parked();
    let image = preview();
    cx.update(|_, cx| {
        app.update(cx, |app, _| {
            app.conversation_presentation.media.insert(
                key.clone(),
                crate::views::message_media::MediaPreview::Ready(image.clone()),
            );
        })
    });
    old.send_blocking(Err("old failed download".into()))
        .unwrap();
    cx.run_until_parked();
    app.read_with(cx, |app, _| {
        assert!(matches!(app.conversation_presentation.media.get(&key), Some(crate::views::message_media::MediaPreview::Ready(live)) if live.id() == image.id()));
    });
}

#[gpui_kit::test]
fn conversation_fixe_streaming_prepares_only_the_tail_turn(cx: &mut TestAppContext) {
    let (app, cx, _) = harness(cx);
    cx.update(|_, cx| app.update(cx, |app, cx| {
        let messages = (0..30).flat_map(|index| {
            let created = format!("2026-04-01T00:{index:02}:00Z");
            [json!({"id":format!("u{index}"),"role":"user","turnId":format!("turn{index}"),"createdAt":created,"text":"question"}),
             json!({"id":format!("m{index}"),"role":"assistant","turnId":format!("turn{index}"),"createdAt":created,"text":"answer","streaming":index == 29})]
        }).collect();
        let mut item = snapshot(messages, "running");
        item["snapshot"]["thread"]["checkpoints"] = json!([]);
        item["snapshot"]["thread"]["activities"] = json!([{"id":"old-work","turnId":"turn28","createdAt":"2026-04-01T00:28:01Z","kind":"log","summary":"previous work"}]);
        item["snapshot"]["thread"]["proposedPlans"] = json!([]);
        item["snapshot"]["thread"]["latestTurn"] = json!({"turnId":"turn29","state":"running","startedAt":"2026-04-01T00:29:00Z"});
        item["snapshot"]["thread"]["session"]["activeTurnId"] = json!("turn29");
        app.apply_thread_update("t", item, cx).unwrap();
        cx.notify();
    }));
    cx.run_until_parked();
    let before = app.read_with(cx, |app, _| app.conversation_presentation.prepared_turns);
    assert_eq!(before, 30);
    for (offset, id) in [(1, "m29"), (2, "new-tail-message")] {
        cx.update(|_, cx| app.update(cx, |app, cx| {
            let sequence = app.detail.sequence.unwrap() + 1;
            app.apply_thread_update("t", json!({"kind":"event","event":{"sequence":sequence,"type":"thread.message-sent","occurredAt":"2026-04-01T00:29:01Z","payload":{"threadId":"t","messageId":id,"role":"assistant","turnId":"turn29","streaming":true,"text":" append","createdAt":"2026-04-01T00:29:01Z","updatedAt":"2026-04-01T00:29:01Z"}}}), cx).unwrap();
            cx.notify();
        }));
        cx.run_until_parked();
        app.read_with(cx, |app, _| {
            assert_eq!(
                app.conversation_presentation.prepared_turns,
                before + offset
            );
            let full = t3code_desktop_gpui::conversation::timeline_rows(
                app.detail.thread.as_ref().unwrap(),
                &app.activity_expanded,
            );
            assert_eq!(*app.conversation_rows, full);
        });
    }
    for (offset, kind, field, item) in [
        (
            3,
            "thread.activity-appended",
            "activity",
            json!({"id":"new-work","turnId":"turn29","createdAt":"2026-04-01T00:29:02Z","kind":"log","summary":"new work"}),
        ),
        (
            4,
            "thread.proposed-plan-upserted",
            "proposedPlan",
            json!({"id":"new-plan","turnId":"turn29","createdAt":"2026-04-01T00:29:03Z","planMarkdown":"Do the work"}),
        ),
    ] {
        cx.update(|_, cx| app.update(cx, |app, cx| {
            let sequence = app.detail.sequence.unwrap() + 1;
            let mut payload = json!({"threadId":"t"});
            payload[field] = item;
            app.apply_thread_update("t", json!({"kind":"event","event":{"sequence":sequence,"type":kind,"occurredAt":"2026-04-01T00:29:03Z","payload":payload}}), cx).unwrap();
            cx.notify();
        }));
        cx.run_until_parked();
        app.read_with(cx, |app, _| {
            assert_eq!(
                app.conversation_presentation.prepared_turns,
                before + offset
            );
            assert_eq!(
                *app.conversation_rows,
                t3code_desktop_gpui::conversation::timeline_rows(
                    app.detail.thread.as_ref().unwrap(),
                    &app.activity_expanded
                )
            );
        });
    }
}

#[gpui_kit::test]
fn conversation_fixe_gallery_arrows_wrap_in_both_directions(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    cx.update(|_, cx| app.update(cx, |app, cx| {
        app.apply_thread_update("t", snapshot(vec![json!({"id":"u", "role":"user", "text":"images", "attachments":[
            {"id":"one", "type":"image", "mimeType":"image/png"}, {"id":"two", "type":"image", "mimeType":"image/png"}
        ]})], "idle"), cx).unwrap();
        cx.notify();
    }));
    cx.run_until_parked();
    while let Ok(connection::Command::Call { reply, .. }) = requests.try_recv() {
        reply.send_blocking(Err("fixture".into())).unwrap();
    }
    cx.run_until_parked();
    let (one, two) = app.read_with(cx, |app, _| {
        (
            crate::views::message_media::media_key(&app.environment_scope, "t", "one"),
            crate::views::message_media::media_key(&app.environment_scope, "t", "two"),
        )
    });
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            for key in [&one, &two] {
                app.conversation_presentation.media.insert(
                    key.clone(),
                    crate::views::message_media::MediaPreview::Ready(preview()),
                );
            }
            app.expand_message_image(
                one.clone(),
                json!({"_tag":"attachment","threadId":"t","attachmentId":"one"}),
                window,
                cx,
            );
        })
    });
    cx.run_until_parked();
    let bounds = cx.debug_bounds("image-next").unwrap();
    cx.simulate_click(bounds.center(), gpui_kit::Modifiers::none());
    cx.run_until_parked();
    assert_eq!(
        app.read_with(cx, |app, _| app
            .conversation_presentation
            .gallery
            .as_ref()
            .unwrap()
            .0
            .clone()),
        two
    );
    cx.simulate_keystrokes("right");
    cx.run_until_parked();
    assert_eq!(
        app.read_with(cx, |app, _| app
            .conversation_presentation
            .gallery
            .as_ref()
            .unwrap()
            .0
            .clone()),
        one
    );
    cx.simulate_keystrokes("left");
    cx.run_until_parked();
    assert_eq!(
        app.read_with(cx, |app, _| app
            .conversation_presentation
            .gallery
            .as_ref()
            .unwrap()
            .0
            .clone()),
        two
    );
}

#[gpui_kit::test]
fn conversation_fixe_code_wrap_changes_real_layout(cx: &mut TestAppContext) {
    let (app, cx, _) = harness(cx);
    cx.update(|_, cx| app.update(cx, |app, cx| {
        app.apply_thread_update("t", snapshot(vec![json!({"id":"code", "role":"assistant", "text":format!("```rust\n{}\n```", "let value = long_identifier; ".repeat(12))})], "idle"), cx).unwrap();
        cx.notify();
    }));
    cx.run_until_parked();
    let wrapped_height = cx.debug_bounds("message-code").unwrap().size.height;
    let bounds = cx.debug_bounds("code-wrap").unwrap();
    cx.simulate_click(bounds.center(), gpui_kit::Modifiers::none());
    cx.run_until_parked();
    let unwrapped_height = cx.debug_bounds("message-code").unwrap().size.height;
    assert!(
        unwrapped_height < wrapped_height,
        "wrap must change painted message height: {wrapped_height:?} -> {unwrapped_height:?}"
    );
    let bounds = cx.debug_bounds("code-wrap").unwrap();
    cx.simulate_click(bounds.center(), gpui_kit::Modifiers::none());
    cx.run_until_parked();
    assert_eq!(
        cx.debug_bounds("message-code").unwrap().size.height,
        wrapped_height
    );
}

#[gpui_kit::test]
fn conversation_fixe_plan_send_waits_for_acceptance_and_uses_confirmed_mode_and_source(
    cx: &mut TestAppContext,
) {
    let (app, cx, requests) = harness(cx);
    for (implement, accepted, mode) in [
        (false, true, "plan"),
        (true, true, "default"),
        (false, false, "plan"),
    ] {
        cx.update(|window, cx| {
            app.update(cx, |app, cx| {
                let mut item = snapshot(vec![], "idle");
                item["snapshot"]["thread"]["proposedPlans"] =
                    json!([{"id":"p", "planMarkdown":"# Plan\n\nDo the work"}]);
                item["snapshot"]["thread"]["runtimeMode"] = json!("full-access");
                item["snapshot"]["thread"]["interactionMode"] =
                    json!(if mode == "plan" { "default" } else { "plan" });
                app.apply_thread_update("t", item, cx).unwrap();
                app.provider_catalog = vec![super::composer_parity::provider_fixture(
                    "codex", "Personal",
                )];
                app.composer
                    .update(cx, |input, cx| input.set_value("", window, cx));
                cx.notify();
            })
        });
        cx.run_until_parked();
        let bounds = cx
            .debug_bounds(if implement {
                "plan-implement"
            } else {
                "plan-refine"
            })
            .unwrap();
        cx.simulate_click(bounds.center(), gpui_kit::Modifiers::none());
        cx.run_until_parked();
        let connection::Command::Call { payload, reply, .. } = requests.try_recv().unwrap() else {
            panic!("mode request")
        };
        assert_eq!(payload["interactionMode"], mode);
        cx.update(|window, cx| app.update(cx, |app, cx| app.send_prompt(window, cx)));
        cx.run_until_parked();
        assert!(
            requests.try_recv().is_err(),
            "Send must wait for mode receipt"
        );
        reply
            .send_blocking(if accepted {
                Ok(json!({}))
            } else {
                Err("mode denied".into())
            })
            .unwrap();
        cx.run_until_parked();
        if !implement {
            cx.update(|window, cx| {
                app.update(cx, |app, cx| {
                    let edited =
                        format!("{}\n\nMy follow-up comment", app.composer.read(cx).value());
                    app.composer
                        .update(cx, |input, cx| input.set_value(edited, window, cx));
                })
            });
        }
        // Deliberately no projection event: selected_modes still returns the old value.
        assert_ne!(app.read_with(cx, |app, _| app.selected_modes().1), mode);
        cx.update(|window, cx| app.update(cx, |app, cx| app.send_prompt(window, cx)));
        cx.run_until_parked();
        if accepted {
            let connection::Command::Call { payload, reply, .. } =
                requests.try_recv().unwrap_or_else(|_| {
                    panic!(
                        "turn request: {}",
                        app.read_with(cx, |app, _| app.status.clone())
                    )
                })
            else {
                panic!("turn request")
            };
            assert_eq!(payload["type"], "thread.turn.start");
            assert_eq!(payload["interactionMode"], mode);
            if implement {
                assert_eq!(
                    payload["sourceProposedPlan"],
                    json!({"threadId":"t", "planId":"p"})
                );
            } else {
                assert!(payload.get("sourceProposedPlan").is_none());
            }
            reply.send_blocking(Ok(json!({}))).unwrap();
            cx.run_until_parked();
        } else {
            assert!(
                requests.try_recv().is_err(),
                "Rejected mode must not permit a turn"
            );
            assert!(app.read_with(cx, |app, cx| !app.composer.read(cx).value().is_empty()));
        }
    }
}

#[gpui_kit::test]
fn conversation_fixe_file_links_use_selected_checkout_and_preserve_dirty_edits(
    cx: &mut TestAppContext,
) {
    let (app, cx, requests) = harness(cx);
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.shell.threads[0].worktree_path = Some("/tmp/worktree-link".into());
            app.open_message_link("src/main.rs#L3", window, cx);
        })
    });
    cx.run_until_parked();
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
        json!({"cwd":"/tmp/worktree-link","relativePath":"src/main.rs"})
    );
    reply
        .send_blocking(Ok(
            json!({"relativePath":"src/main.rs","contents":"old","truncated":false}),
        ))
        .unwrap();
    cx.run_until_parked();
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.file_contents
                .update(cx, |input, cx| input.set_value("unsaved", window, cx));
            app.open_message_link("other.rs", window, cx);
        })
    });
    cx.run_until_parked();
    assert!(requests.try_recv().is_err());
    assert_eq!(
        app.read_with(cx, |app, cx| app.file_contents.read(cx).value().to_string()),
        "unsaved"
    );
}

#[test]
fn conversation_fixe_preview_downsamples_and_rejects_huge_dimensions() {
    for (dimension, accepted) in [(2400, true), (4097, false)] {
        let mut png = std::io::Cursor::new(Vec::new());
        image::DynamicImage::new_rgba8(dimension, 1)
            .write_to(&mut png, image::ImageFormat::Png)
            .unwrap();
        let result = crate::views::message_media::bounded_preview(
            &png.into_inner(),
            gpui_kit::ImageFormat::Png,
        );
        assert_eq!(result.is_ok(), accepted);
        if let Ok(preview) = result {
            let decoded = image::load_from_memory(&preview.bytes).unwrap();
            assert!(decoded.width() <= 1200 && decoded.height() <= 1200);
            assert_eq!(preview.format, gpui_kit::ImageFormat::Png);
        }
    }
}

#[test]
fn conversation_fixe_gif_is_first_frame_only_and_excessive_frames_are_refused() {
    fn animation(count: usize) -> Vec<u8> {
        let mut bytes = Vec::new();
        {
            let mut encoder = image::codecs::gif::GifEncoder::new(&mut bytes);
            for _ in 0..count {
                encoder
                    .encode_frame(image::Frame::new(image::RgbaImage::new(2, 2)))
                    .unwrap();
            }
        }
        bytes
    }
    let preview =
        crate::views::message_media::bounded_preview(&animation(2), gpui_kit::ImageFormat::Gif)
            .unwrap();
    assert_eq!(preview.format, gpui_kit::ImageFormat::Png);
    assert!(
        crate::views::message_media::bounded_preview(&animation(257), gpui_kit::ImageFormat::Gif)
            .is_err()
    );
}
