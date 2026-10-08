use super::*;

fn assert_overlay_focus(app: &Entity<NativeApp>, cx: &mut VisualTestContext) {
    cx.update(|window, cx| {
        app.read_with(cx, |app, cx| {
            assert!(
                app.settings_shortcut_focus.contains_focused(window, cx),
                "palette={} branch={:?} focused={:?}",
                app.palette_open,
                app.branch_picker,
                window.focused(cx)
            );
            assert!(!app.composer.read(cx).focus_handle(cx).is_focused(window));
        });
    });
}

#[gpui_kit::test]
fn polx_settings_entry_points_capture_typing_trap_and_restore_focus(cx: &mut TestAppContext) {
    let (app, cx, _) = harness(cx);
    for entry in [
        "settings-open",
        "sidebar-settings",
        "environment-chip",
        "shortcut",
    ] {
        if entry == "shortcut" {
            cx.simulate_keystrokes("cmd-,");
        } else {
            let bounds = cx.debug_bounds(entry).unwrap();
            cx.simulate_click(bounds.center(), Default::default());
        }
        cx.run_until_parked();
        cx.update(|window, cx| {
            app.read_with(cx, |app, cx| {
                assert!(
                    app.settings_query
                        .read(cx)
                        .focus_handle(cx)
                        .is_focused(window),
                    "{entry}"
                );
            })
        });
        cx.simulate_input("notif");
        cx.run_until_parked();
        app.read_with(cx, |app, cx| {
            assert_eq!(app.settings_query.read(cx).value().as_ref(), "notif");
            assert_eq!(app.composer.read(cx).value().as_ref(), "");
        });
        let settings = cx.debug_bounds("settings").unwrap();
        let search = cx.debug_bounds("settings-search").unwrap();
        assert!(settings.contains(&search.center()));
        for _ in 0..20 {
            cx.simulate_keystrokes("tab");
            cx.run_until_parked();
            assert_overlay_focus(&app, cx);
        }
        for _ in 0..20 {
            cx.simulate_keystrokes("shift-tab");
            cx.run_until_parked();
            assert_overlay_focus(&app, cx);
        }
        cx.simulate_keystrokes("escape");
        cx.run_until_parked();
        assert!(cx.debug_bounds("settings").is_none());
        cx.update(|window, cx| {
            app.read_with(cx, |app, cx| {
                assert!(app.composer.read(cx).focus_handle(cx).is_focused(window));
            })
        });
    }
}

#[gpui_kit::test]
fn polx_settings_close_button_and_backdrop_restore_the_previous_input(cx: &mut TestAppContext) {
    let (app, cx, _) = harness(cx);
    for backdrop in [false, true] {
        cx.update(|window, cx| {
            app.update(cx, |app, cx| {
                app.search.update(cx, |input, cx| input.focus(window, cx))
            })
        });
        let open = cx.debug_bounds("settings-open").unwrap();
        cx.simulate_click(open.center(), Default::default());
        cx.run_until_parked();
        if backdrop {
            cx.simulate_click(point(px(1200.), px(950.)), Default::default());
        } else {
            let close = cx.debug_bounds("settings-close").unwrap();
            cx.simulate_click(close.center(), Default::default());
        }
        cx.run_until_parked();
        cx.update(|window, cx| {
            app.read_with(cx, |app, cx| {
                assert!(!app.settings_open);
                assert!(app.search.read(cx).focus_handle(cx).is_focused(window));
            })
        });
    }
}

#[gpui_kit::test]
fn polx_palette_and_branch_picker_trap_search_focus(cx: &mut TestAppContext) {
    let (app, cx, _) = harness(cx);
    cx.simulate_keystrokes("cmd-k");
    cx.run_until_parked();
    cx.update(|window, cx| {
        app.read_with(cx, |app, cx| {
            assert!(
                app.palette_query
                    .read(cx)
                    .focus_handle(cx)
                    .is_focused(window)
            )
        })
    });
    for _ in 0..8 {
        cx.simulate_keystrokes("tab shift-tab");
        cx.run_until_parked();
        assert_overlay_focus(&app, cx);
    }
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    let trigger = cx.debug_bounds("checkout-selector").unwrap();
    cx.simulate_click(trigger.center(), Default::default());
    cx.run_until_parked();
    assert!(
        cx.debug_bounds("branch-picker").is_some(),
        "branch picker must paint"
    );
    cx.update(|window, cx| {
        app.read_with(cx, |app, cx| {
            assert!(
                app.branch_query
                    .read(cx)
                    .focus_handle(cx)
                    .is_focused(window)
            )
        })
    });
    for _ in 0..12 {
        cx.simulate_keystrokes("tab");
        cx.run_until_parked();
        assert_overlay_focus(&app, cx);
    }
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    cx.update(|window, cx| {
        app.read_with(cx, |app, cx| {
            assert!(app.composer.read(cx).focus_handle(cx).is_focused(window))
        })
    });
}

#[gpui_kit::test]
fn polx_model_picker_traps_and_restores_focus_on_escape_and_selection(cx: &mut TestAppContext) {
    let (app, cx, _requests) = harness(cx);
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.provider_catalog = vec![composer_parity::provider_fixture("codex", "Personal")];
            cx.notify();
        })
    });
    cx.run_until_parked();
    for choose in [false, true] {
        cx.update(|window, cx| {
            app.update(cx, |app, cx| {
                app.search.update(cx, |input, cx| input.focus(window, cx))
            })
        });
        let trigger = cx.debug_bounds("model-catalog").unwrap();
        cx.simulate_click(trigger.center(), Default::default());
        cx.run_until_parked();
        assert!(cx.debug_bounds("provider-model-catalog").is_some());
        cx.update(|window, cx| {
            app.read_with(cx, |app, cx| {
                assert!(
                    app.composer_state
                        .catalog_search
                        .as_ref()
                        .unwrap()
                        .read(cx)
                        .focus_handle(cx)
                        .is_focused(window)
                )
            })
        });
        for _ in 0..12 {
            cx.simulate_keystrokes("tab shift-tab");
            cx.run_until_parked();
            assert_overlay_focus(&app, cx);
        }
        cx.update(|window, cx| {
            app.update(cx, |app, cx| {
                app.composer_state
                    .catalog_search
                    .as_ref()
                    .unwrap()
                    .update(cx, |input, cx| input.focus(window, cx))
            })
        });
        cx.simulate_keystrokes(if choose { "enter" } else { "escape" });
        cx.run_until_parked();
        cx.update(|window, cx| {
            app.read_with(cx, |app, cx| {
                assert!(!app.catalog_open);
                assert!(app.search.read(cx).focus_handle(cx).is_focused(window));
            })
        });
    }
}

#[gpui_kit::test]
fn polx_priority_shelf_labels_share_the_left_row_inset_and_collapse(cx: &mut TestAppContext) {
    let (_, cx, _) = harness(cx);
    let priority = cx.debug_bounds("priority-view").unwrap();
    cx.simulate_click(priority.center(), Default::default());
    cx.run_until_parked();
    let mut left = None;
    for (header, label) in [
        ("priority-pinned", "priority-pinned-label"),
        ("priority-active", "priority-active-label"),
        ("priority-snoozed", "priority-snoozed-label"),
        ("priority-settled", "priority-settled-label"),
    ] {
        let header = cx.debug_bounds(header).unwrap();
        let label = cx.debug_bounds(label).unwrap();
        assert!(
            label.left() - header.left() <= px(32.),
            "left row inset, not centred: {label:?} {header:?}"
        );
        if let Some(left) = left {
            assert!((label.left() - left).abs() <= px(1.));
        }
        left = Some(label.left());
    }
    let row = cx.debug_bounds("thread-t").unwrap();
    let active = cx.debug_bounds("priority-active").unwrap();
    assert!(row.top() >= active.bottom());
    cx.simulate_click(active.center(), Default::default());
    cx.run_until_parked();
    assert!(cx.debug_bounds("thread-t").is_none());
    let active = cx.debug_bounds("priority-active").unwrap();
    cx.simulate_click(active.center(), Default::default());
    cx.run_until_parked();
    assert!(cx.debug_bounds("thread-t").is_some());
}

#[gpui_kit::test]
fn polx_toasts_do_not_occlude_composer_or_send_at_window_sizes(cx: &mut TestAppContext) {
    let (app, cx, _) = harness(cx);
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.composer.update(cx, |input, cx| {
                input.set_value("first line\nsecond line\nthird line", window, cx)
            });
            for message in ["First failure", "Second failure", "Third failure"] {
                app.report_error(message, cx);
            }
        })
    });
    for (width, height) in [(1280., 983.), (1000., 800.)] {
        cx.simulate_resize(size(px(width), px(height)));
        cx.run_until_parked();
        let stack = cx.debug_bounds("toast-stack").unwrap();
        let composer = cx.debug_bounds("composer").unwrap();
        let send = cx.debug_bounds("send").unwrap();
        assert!(
            stack.bottom() <= composer.top(),
            "{stack:?} overlaps {composer:?}"
        );
        assert!(!stack.intersects(&send));
    }
    let close = cx.debug_bounds("toast-close-3").unwrap();
    cx.simulate_click(close.center(), Default::default());
    cx.run_until_parked();
    assert!(cx.debug_bounds("toast-3").is_none());
}

#[gpui_kit::test]
fn polx_selecting_a_thread_clears_only_its_resolved_validation(cx: &mut TestAppContext) {
    let (app, cx, _requests) = harness(cx);
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.selected_thread = None;
            app.attach_files(cx);
            app.attach_files(cx);
            assert_eq!(app.toasts[0].count, 2);
            app.report_error("Unrelated server failure", cx);
        })
    });
    cx.run_until_parked();
    assert!(cx.debug_bounds("toast-1").is_some());
    assert!(cx.debug_bounds("toast-3").is_some());
    let row = cx.debug_bounds("thread-t").unwrap();
    cx.simulate_click(row.center(), Default::default());
    cx.run_until_parked();
    assert!(cx.debug_bounds("toast-1").is_none());
    assert!(cx.debug_bounds("toast-3").is_some());
    app.read_with(cx, |app, _| {
        assert_eq!(app.selected_thread.as_deref(), Some("t"));
        assert_eq!(app.status, "Unrelated server failure");
    });
}

#[gpui_kit::test]
fn polx_status_chips_pack_left_and_message_owns_spare_width(cx: &mut TestAppContext) {
    let (app, cx, _) = harness(cx);
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.provider
                .update(cx, |input, cx| input.set_value("", window, cx));
            app.model
                .update(cx, |input, cx| input.set_value("", window, cx));
            app.environment_name = Some("Lab".into());
            app.set_status("Provider catalog loaded successfully", cx);
        })
    });
    for width in [1280., 1100.] {
        cx.simulate_resize(size(px(width), px(983.)));
        cx.run_until_parked();
        let bar = cx.debug_bounds("status-bar").unwrap();
        let dot = cx.debug_bounds("connection-status-dot").unwrap();
        let chip = cx.debug_bounds("environment-chip").unwrap();
        let usage = cx.debug_bounds("usage-limits-entry").unwrap();
        let message = cx.debug_bounds("status-message").unwrap();
        let inset = cx.update(|window, _| window.rem_size() * 0.75);
        assert!(
            (dot.left() - bar.left() - inset).abs() <= px(1.),
            "{dot:?} {bar:?}"
        );
        assert!(chip.left() - dot.right() <= px(8.));
        assert!(usage.left() - chip.right() <= px(16.));
        assert!(message.left() - usage.right() <= px(16.));
        assert!((bar.right() - message.right() - inset).abs() <= px(1.));
        assert!(message.size.width >= bar.size.width - chip.size.width - usage.size.width - px(72.));
    }
}
