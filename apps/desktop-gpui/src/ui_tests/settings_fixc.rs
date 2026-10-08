//! Regression tests use the actual palette input and native key dispatch.
use crate::*;
use gpui_kit::{AppContext, TestAppContext, VisualTestContext, component::Root};

fn harness(cx: &mut TestAppContext) -> (Entity<NativeApp>, &mut VisualTestContext) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        theme::init(cx);
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
    cx.update(|window, cx| app.update(cx, |app, cx| {
        app.shell.apply(serde_json::from_value(json!({"kind":"snapshot","snapshot":{"snapshotSequence":1,"projects":[{"id":"p","title":"First","workspaceRoot":"/tmp/first"},{"id":"p2","title":"Second","workspaceRoot":"/tmp/second","defaultModelSelection":{"instanceId":"codex","model":"gpt-5.4"}}],"threads":[{"id":"t","projectId":"p","title":"First task","modelSelection":{"instanceId":"codex","model":"gpt-5.4"}}]}})).unwrap()).unwrap();
        app.selected_project = Some("p".into());
        app.selected_thread = Some("t".into());
        app.provider.update(cx, |input, cx| input.set_value("", window, cx));
        app.model.update(cx, |input, cx| input.set_value("", window, cx));
        app.composer.update(cx, |input, cx| input.focus(window, cx));
        cx.notify();
    }));
    cx.run_until_parked();
    (app, cx)
}

#[gpui_kit::test]
fn settings_fixc_dirty_palette_navigation_keeps_entire_context(cx: &mut TestAppContext) {
    let (app, cx) = harness(cx);
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.file_editor.loaded(
                "/tmp/first".into(),
                files::Read {
                    relative_path: "notes.txt".into(),
                    contents: "saved".into(),
                    truncated: false,
                },
            );
            app.file_contents
                .update(cx, |input, cx| input.set_value("unsaved", window, cx));
        })
    });
    for query in ["New thread in Second", "Open project: Second"] {
        cx.simulate_keystrokes("cmd-k");
        cx.simulate_input(query);
        cx.simulate_keystrokes("enter");
        cx.run_until_parked();
        app.read_with(cx, |app, cx| {
            assert_eq!(app.selected_project.as_deref(), Some("p"));
            assert_eq!(app.selected_thread.as_deref(), Some("t"));
            assert_eq!(app.selected_workspace().as_deref(), Some("/tmp/first"));
            assert!(app.pending_threads.is_empty());
            assert!(app.file_editor.dirty(&app.file_contents.read(cx).value()));
        });
    }
}

#[gpui_kit::test]
fn settings_fixc_invalid_worktree_base_rolls_back_project(cx: &mut TestAppContext) {
    let (app, cx) = harness(cx);
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.prepare_new_worktree = true;
            app.worktree_ref
                .update(cx, |input, cx| input.set_value("", window, cx));
        })
    });
    cx.simulate_keystrokes("cmd-k");
    cx.simulate_input("New thread in Second");
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    app.read_with(cx, |app, _| {
        assert_eq!(app.selected_project.as_deref(), Some("p"));
        assert_eq!(app.selected_thread.as_deref(), Some("t"));
        assert!(app.pending_threads.is_empty());
    });
}

#[gpui_kit::test]
fn settings_fixc_palette_cache_reuses_revision_and_keyboard_reveals_row(cx: &mut TestAppContext) {
    let (app, cx) = harness(cx);
    cx.update(|_, cx| app.update(cx, |app, cx| {
        for i in 0..1000 {
            app.shell.threads.push(serde_json::from_value(json!({"id":format!("t{i}"),"projectId":"p","title":format!("Task {i:04}")})).unwrap());
        }
        app.shell.revision += 1;
        let first = app.palette_items(cx);
        assert!(std::rc::Rc::ptr_eq(&first, &app.palette_items(cx)));
    }));
    cx.simulate_keystrokes("cmd-k");
    cx.simulate_input("Task");
    for _ in 0..40 {
        cx.simulate_keystrokes("down");
    }
    cx.run_until_parked();
    let index = app.read_with(cx, |app, _| app.palette_index);
    assert_eq!(index, 40);
    assert!(cx.debug_bounds("palette-40").is_some());
    assert!(cx.debug_bounds("palette-500").is_none());
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    assert_eq!(
        app.read_with(cx, |app, _| app.selected_thread.clone())
            .as_deref(),
        Some("t39")
    );
}

#[gpui_kit::test]
fn settings_fixc_recorded_shortcut_conflicts_then_replaces_live_keymap(cx: &mut TestAppContext) {
    let (app, cx) = harness(cx);
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.settings_open = true;
            app.settings_query
                .update(cx, |input, cx| input.set_value("keybindings", window, cx));
            cx.notify();
        })
    });
    cx.run_until_parked();
    let record = cx.debug_bounds("shortcut-record-3").unwrap().center();
    cx.simulate_click(record, gpui_kit::Modifiers::none());
    cx.simulate_keystrokes("cmd-p");
    cx.run_until_parked();
    assert!(
        app.read_with(cx, |app, _| app.status.contains("conflicts")),
        "status: {}",
        app.read_with(cx, |app, _| app.status.clone())
    );
    cx.simulate_keystrokes("ctrl-alt-p");
    cx.run_until_parked();
    assert!(app.read_with(cx, |app, _| app.status.contains("saved")));
    // Verify the live dispatch below, rather than raw keymap equality: parsed
    // keystrokes omit the character metadata carried by recorded OS events.
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.settings_open = false;
            app.composer.update(cx, |input, cx| input.focus(window, cx));
            cx.notify();
        })
    });
    cx.simulate_keystrokes("cmd-k");
    cx.run_until_parked();
    assert!(!app.read_with(cx, |app, _| app.palette_open));
    cx.simulate_keystrokes("ctrl-alt-p");
    cx.run_until_parked();
    assert!(app.read_with(cx, |app, _| app.palette_open));
    cx.simulate_keystrokes("escape");
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.settings_open = true;
            cx.notify();
        })
    });
    cx.run_until_parked();
    let reset = cx.debug_bounds("shortcut-reset-3").unwrap().center();
    cx.simulate_click(reset, gpui_kit::Modifiers::none());
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.settings_open = false;
            app.composer.update(cx, |input, cx| input.focus(window, cx));
            cx.notify();
        })
    });
    cx.simulate_keystrokes("cmd-k");
    cx.run_until_parked();
    assert!(app.read_with(cx, |app, _| app.palette_open));
}

#[gpui_kit::test]
fn settings_fixc_disabled_or_dirty_environment_switch_retains_context(cx: &mut TestAppContext) {
    let (app, cx) = harness(cx);
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.server.update(cx, |input, cx| {
                input.set_value("https://second.example", window, cx)
            });
            app.save_environment(cx);
            app.file_editor.loaded(
                "/tmp/first".into(),
                files::Read {
                    relative_path: "notes.txt".into(),
                    contents: "saved".into(),
                    truncated: false,
                },
            );
            app.file_contents
                .update(cx, |input, cx| input.set_value("unsaved", window, cx));
            let generation = app.generation;
            app.switch_environment("https://second.example", window, cx);
            assert_eq!(app.generation, generation);
            assert_eq!(app.selected_project.as_deref(), Some("p"));
            assert_eq!(app.selected_thread.as_deref(), Some("t"));
            app.switch_environment("https://not-saved.example", window, cx);
            assert_eq!(app.generation, generation);
        })
    });
}

#[gpui_kit::test]
fn settings_fixc_zoom_scales_settings_geometry(cx: &mut TestAppContext) {
    let (app, cx) = harness(cx);
    cx.simulate_resize(size(px(1800.), px(1600.)));
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.settings_open = true;
            cx.notify();
        })
    });
    cx.run_until_parked();
    let original = cx.debug_bounds("settings").unwrap().size.width;
    cx.update(|window, cx| app.update(cx, |app, cx| app.change_scale(Some(1), window, cx)));
    cx.run_until_parked();
    let scaled = cx.debug_bounds("settings").unwrap().size.width;
    assert!(scaled > original * 1.09);
    cx.update(|window, cx| app.update(cx, |app, cx| app.change_scale(None, window, cx)));
    cx.run_until_parked();
    assert_eq!(cx.debug_bounds("settings").unwrap().size.width, original);
}

#[gpui_kit::test]
fn settings_fixc_provider_failures_have_typed_error_receipts(cx: &mut TestAppContext) {
    let (app, cx) = harness(cx);
    let (sender, requests) = async_channel::bounded(4);
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.commands = Some(sender);
            app.refresh_settings_providers(false, cx);
        })
    });
    cx.run_until_parked();
    let connection::Command::Call { tag, reply, .. } = requests.try_recv().unwrap() else {
        panic!("expected config request");
    };
    assert_eq!(tag, "server.getConfig");
    reply
        .send_blocking(Err("Permission denied".into()))
        .unwrap();
    cx.run_until_parked();
    app.read_with(cx, |app, _| {
        assert_eq!(app.status_kind, notifications::Outcome::Error);
        assert!(
            app.toasts
                .iter()
                .any(|toast| toast.kind == notifications::ToastKind::Error
                    && toast.message.contains("Permission denied"))
        );
    });
}

#[gpui_kit::test]
fn settings_fixc_preferences_apply_live_and_survive_restart(cx: &mut TestAppContext) {
    let (app, cx) = harness(cx);
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("preferences.json");
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            theme::update(window, cx, |p| {
                p.variant = settings_appearance::Variant::HighContrast;
                p.confirm_archive = true;
                p.confirm_delete = false;
                p.confirm_unpin = true;
                p.timestamp_format = settings_appearance::TimestampFormat::TwentyFourHour;
                p.prose_size = 20.;
                p.code_size = 18.;
                p.zoom = 2;
            })
            .unwrap();
            let p = settings_appearance::current(cx);
            p.save(&path).unwrap();
            let restored = settings_appearance::Preferences::load(&path);
            assert_eq!(restored.variant, p.variant);
            assert_eq!(restored.timestamp_format, app.timestamp_format(cx));
            assert!(app.confirmation_required("thread.archive", cx));
            assert!(!app.confirmation_required("thread.delete", cx));
            assert!(app.confirmation_required("thread.unpin", cx));
            assert!(app.prose_font_size(cx) > px(20.));
            assert!(app.code_font_size(cx) > px(18.));
            assert_eq!(
                gpui_kit::component::Theme::global(cx).theme_name().as_ref(),
                "High Contrast Dark"
            );
        })
    });
}
