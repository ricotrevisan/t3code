use super::*;
use t3code_desktop_gpui::notification_platform::{Call, DeliveryOutcome, Sound};

fn receipt(
    app: &Entity<NativeApp>,
    cx: &mut VisualTestContext,
) -> async_channel::Sender<DeliveryOutcome> {
    cx.update(|_, cx| app.update(cx, |app, _| app.notification_platform.queue_delivery()))
}

fn accept_next(app: &Entity<NativeApp>, cx: &mut VisualTestContext) {
    receipt(app, cx)
        .try_send(DeliveryOutcome::Accepted)
        .unwrap();
}

fn ready(
    app: &Entity<NativeApp>,
    cx: &mut VisualTestContext,
    mode: notifications::NotificationMode,
    in_app: bool,
    focused: bool,
) {
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.apply_shell_item(shell::ShellItem::Synchronized, cx)
                .unwrap();
            app.notification_tracker.baseline(app.shell.threads.iter());
            app.window_active = focused;
            app.set_notification_preferences(notifications::Preferences { mode, in_app }, cx);
            app.notification_platform.calls.clear();
        })
    });
}

#[gpui_kit::test]
fn focused_other_thread_toast_only_selected_thread_sound_only(cx: &mut TestAppContext) {
    let (app, cx, _) = harness(cx);
    seed_second_thread(&app, cx, false);
    ready(
        &app,
        cx,
        notifications::NotificationMode::NotificationsAndSound,
        true,
        true,
    );
    upsert_thread(
        &app,
        cx,
        2,
        json!({"id":"t","title":"Thread","hasPendingApprovals":true}),
    );
    assert!(app.read_with(cx, |app, _| app.toasts.is_empty()));
    upsert_thread(
        &app,
        cx,
        3,
        json!({"id":"u","title":"Other","latestTurn":{"turnId":"a","state":"error"}}),
    );
    assert!(cx.shown_system_notifications().is_empty());
    app.read_with(cx, |app, _| {
        assert_eq!(app.toasts.len(), 1);
        assert_eq!(app.toasts[0].kind, notifications::ToastKind::Error);
        assert!(app.toasts[0].message.starts_with("Thread failed"));
        assert_eq!(
            app.notification_platform
                .calls
                .iter()
                .filter(|call| **call == Call::Sound(Sound::Input))
                .count(),
            2
        );
    });
}

#[gpui_kit::test]
fn background_badge_counts_threads_and_settings_disconnect_clear(cx: &mut TestAppContext) {
    let (app, cx, _) = harness(cx);
    seed_second_thread(&app, cx, false);
    ready(
        &app,
        cx,
        notifications::NotificationMode::NotificationsAndSound,
        true,
        false,
    );
    accept_next(&app, cx);
    upsert_thread(
        &app,
        cx,
        2,
        json!({"id":"u","title":"Other","hasPendingApprovals":true,"latestTurn":{"turnId":"a","state":"running"}}),
    );
    accept_next(&app, cx);
    upsert_thread(
        &app,
        cx,
        3,
        json!({"id":"u","title":"Other","hasPendingApprovals":true,"latestTurn":{"turnId":"b","state":"running"}}),
    );
    cx.run_until_parked();
    assert_eq!(app.read_with(cx, |app, _| app.notification_badge.len()), 1);
    accept_next(&app, cx);
    upsert_thread(
        &app,
        cx,
        4,
        json!({"id":"t","title":"Thread","latestTurn":{"turnId":"a","state":"completed","completedAt":"2026-01-02T00:00:00Z"}}),
    );
    cx.run_until_parked();
    app.read_with(cx, |app, _| {
        assert!(app.toasts.is_empty());
        assert_eq!(app.notification_badge.len(), 2);
        assert!(app.notification_platform.calls.contains(&Call::Badge(1)));
        assert!(app.notification_platform.calls.contains(&Call::Badge(2)));
        assert!(
            app.notification_platform
                .calls
                .contains(&Call::Sound(Sound::Completion))
        );
    });
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            // Desktop-to-desktop mode changes clear delivered alerts too.
            app.set_notification_preferences(
                notifications::Preferences {
                    mode: notifications::NotificationMode::Notifications,
                    in_app: true,
                },
                cx,
            );
            assert!(app.notification_badge.is_empty());
            app.set_notification_preferences(notifications::Preferences::default(), cx)
        })
    });
    app.read_with(cx, |app, _| {
        assert!(app.notification_badge.is_empty());
        assert_eq!(
            app.notification_platform.calls.last(),
            Some(&Call::Badge(0))
        );
    });
    upsert_thread(
        &app,
        cx,
        5,
        json!({"id":"u","title":"Other","hasPendingUserInput":true}),
    );
    cx.run_until_parked();
    let before = app.read_with(cx, |app, _| app.notification_platform.calls.len());
    cx.update(|_, cx| app.update(cx, |app, cx| app.disconnect(cx)));
    assert_eq!(
        app.read_with(cx, |app, _| app.notification_platform.calls.len()),
        before + 1
    );
}

#[gpui_kit::test]
fn only_accepted_receipts_badge_and_late_receipts_are_invalidated(cx: &mut TestAppContext) {
    let (app, cx, _) = harness(cx);
    ready(
        &app,
        cx,
        notifications::NotificationMode::Notifications,
        false,
        false,
    );
    let mut sequence = 2;
    for outcome in [
        DeliveryOutcome::Denied,
        DeliveryOutcome::Unavailable,
        DeliveryOutcome::AuthorizationError,
        DeliveryOutcome::DeliveryError,
    ] {
        receipt(&app, cx).try_send(outcome).unwrap();
        upsert_thread(
            &app,
            cx,
            sequence,
            json!({"id":"t","title":"Thread","hasPendingApprovals":true,"latestTurn":{"turnId":sequence.to_string(),"state":"running"}}),
        );
        sequence += 1;
        cx.run_until_parked();
        app.read_with(cx, |app, _| {
            assert!(app.notification_badge.is_empty());
            assert!(
                app.notification_platform
                    .calls
                    .iter()
                    .any(|call| matches!(call, Call::Receipt(_, result) if *result == outcome))
            );
            assert!(!app.notification_platform.calls.contains(&Call::Badge(1)));
            let text = app.notification_state_text();
            match outcome {
                DeliveryOutcome::Denied => assert!(text.contains("denied")),
                DeliveryOutcome::Unavailable => assert!(text.contains("unavailable")),
                DeliveryOutcome::AuthorizationError => {
                    assert!(text.contains("Could not determine"))
                }
                DeliveryOutcome::DeliveryError => assert!(text.contains("delivery failed")),
                _ => unreachable!(),
            }
            assert!(!text.contains("OS accepted"));
        });
    }
    // Every reset invalidates an in-flight accepted receipt, including changing
    // between two desktop-enabled modes and switching environment.
    for reset in 0..3 {
        ready(
            &app,
            cx,
            notifications::NotificationMode::Notifications,
            false,
            false,
        );
        let pending = receipt(&app, cx);
        upsert_thread(
            &app,
            cx,
            sequence,
            json!({"id":"t","title":"Thread","hasPendingApprovals":true,"latestTurn":{"turnId":sequence.to_string(),"state":"running"}}),
        );
        sequence += 1;
        cx.run_until_parked();
        assert!(app.read_with(cx, |app, _| app.notification_badge.is_empty()));
        cx.update(|_, cx| {
            app.update(cx, |app, cx| match reset {
                0 => app.set_notification_preferences(
                    notifications::Preferences {
                        mode: notifications::NotificationMode::NotificationsAndSound,
                        in_app: false,
                    },
                    cx,
                ),
                1 => {
                    app.clear_notifications(cx);
                    app.environment_scope = "another-environment".into();
                }
                _ => app.clear_notification_badge(cx),
            })
        });
        pending.try_send(DeliveryOutcome::Accepted).unwrap();
        cx.run_until_parked();
        app.read_with(cx, |app, _| {
            assert!(app.notification_badge.is_empty());
            assert!(
                app.notification_platform
                    .calls
                    .iter()
                    .any(|call| matches!(call, Call::Receipt(_, DeliveryOutcome::Invalidated)))
            );
        });
    }
}

#[gpui_kit::test]
fn background_liveness_defers_outgoing_effects_until_ready(cx: &mut TestAppContext) {
    let (app, cx, _) = harness(cx);
    ready(
        &app,
        cx,
        notifications::NotificationMode::NotificationsAndSound,
        false,
        false,
    );
    for (index, liveness) in ["working", "monitoring"].into_iter().enumerate() {
        let sequence = 2 + index as u64 * 4;
        let turn = format!("turn-{index}");
        upsert_thread(
            &app,
            cx,
            sequence,
            json!({"id":"t","title":"Thread","backgroundLiveness":liveness,"latestTurn":{"turnId":turn,"state":"completed","completedAt":format!("2026-01-0{}T00:00:00Z", index + 1)}}),
        );
        cx.run_until_parked();
        assert!(app.read_with(cx, |app, _| app.notification_platform.calls.is_empty()));
        accept_next(&app, cx);
        upsert_thread(
            &app,
            cx,
            sequence + 1,
            json!({"id":"t","title":"Thread","latestTurn":{"turnId":turn,"state":"completed","completedAt":format!("2026-01-0{}T00:00:00Z", index + 1)}}),
        );
        cx.run_until_parked();
        app.read_with(cx, |app, _| {
            assert!(
                app.notification_platform
                    .calls
                    .contains(&Call::Sound(Sound::Completion))
            );
            assert!(
                app.notification_platform
                    .calls
                    .iter()
                    .any(|call| matches!(call, Call::Receipt(_, DeliveryOutcome::Accepted)))
            );
            assert_eq!(app.notification_badge.len(), 1);
        });
        cx.update(|_, cx| {
            app.update(cx, |app, cx| {
                app.clear_notification_badge(cx);
                app.notification_platform.calls.clear();
            })
        });
        upsert_thread(
            &app,
            cx,
            sequence + 2,
            json!({"id":"t","title":"Thread","backgroundLiveness":liveness,"latestTurn":{"turnId":format!("error-{index}"),"state":"error"}}),
        );
        cx.run_until_parked();
        assert!(app.read_with(cx, |app, _| app.notification_platform.calls.is_empty()));
        accept_next(&app, cx);
        upsert_thread(
            &app,
            cx,
            sequence + 3,
            json!({"id":"t","title":"Thread","latestTurn":{"turnId":format!("error-{index}"),"state":"error"}}),
        );
        cx.run_until_parked();
        app.read_with(cx, |app, _| {
            assert!(
                app.notification_platform
                    .calls
                    .contains(&Call::Sound(Sound::Input))
            );
            assert_eq!(app.notification_badge.len(), 1);
        });
        cx.update(|_, cx| {
            app.update(cx, |app, cx| {
                app.clear_notification_badge(cx);
                app.notification_platform.calls.clear();
            })
        });
    }
}

#[gpui_kit::test]
fn stale_environment_click_and_reconnected_snapshot_do_not_replay(cx: &mut TestAppContext) {
    let (app, cx, _) = harness(cx);
    seed_second_thread(&app, cx, false);
    ready(
        &app,
        cx,
        notifications::NotificationMode::NotificationsAndSound,
        false,
        false,
    );
    let stale = app.read_with(cx, |app, _| notifications::tag(&app.environment_scope, "u"));
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.clear_notifications(cx);
            app.environment_scope = "https://another.example".into();
            app.open_notified_thread(&stale, window, cx);
            assert_eq!(app.selected_thread.as_deref(), Some("t"));
            let snapshot = shell::Snapshot {
                snapshot_sequence: 10,
                projects: app.shell.projects.clone(),
                threads: app.shell.threads.clone(),
            };
            app.apply_shell_item(shell::ShellItem::Snapshot { snapshot }, cx)
                .unwrap();
            app.notification_platform.calls.clear();
        })
    });
    upsert_thread(
        &app,
        cx,
        11,
        json!({"id":"u","title":"Other","hasPendingApprovals":true}),
    );
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.apply_shell_item(shell::ShellItem::Synchronized, cx)
                .unwrap()
        })
    });
    upsert_thread(
        &app,
        cx,
        12,
        json!({"id":"u","title":"Other","hasPendingApprovals":true}),
    );
    assert!(app.read_with(cx, |app, _| app.notification_platform.calls.is_empty()));
    assert!(cx.shown_system_notifications().is_empty());
}

#[gpui_kit::test]
fn authorization_probe_surfaces_denied_and_not_determined_without_delivery(
    cx: &mut TestAppContext,
) {
    use t3code_desktop_gpui::notification_platform::Authorization;
    let (app, cx, _) = harness(cx);
    for authorization in [
        Authorization::NotDetermined,
        Authorization::Denied,
        Authorization::Authorized,
    ] {
        cx.update(|_, cx| {
            app.update(cx, |app, cx| {
                app.notification_platform
                    .record_authorization(authorization);
                app.refresh_notification_authorization(cx);
            })
        });
        cx.run_until_parked();
        app.read_with(cx, |app, _| {
            let text = app.notification_state_text();
            match authorization {
                Authorization::NotDetermined => assert!(text.contains("not determined")),
                Authorization::Denied => assert!(text.contains("denied")),
                Authorization::Authorized => assert!(text.contains("no delivery receipt")),
                _ => unreachable!(),
            }
            assert!(
                !app.notification_platform
                    .calls
                    .iter()
                    .any(|call| matches!(call, Call::Delivery(_)))
            );
        });
    }
}
