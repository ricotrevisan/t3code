use super::*;
use t3code_desktop_gpui::notification_platform::{Authorization, Call};

#[gpui_kit::test]
fn r13_a_shared_execution_rejects_live_sender_without_synchronized_projection(
    cx: &mut TestAppContext,
) {
    let (app, cx, requests) = harness(cx);
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.apply_connection_phase(connection::Phase::Syncing, cx);
            app.perform("vcs.pull", json!({"cwd":"/tmp/fixture"}), None, cx);
        })
    });
    cx.run_until_parked();
    assert!(requests.try_recv().is_err());
    assert_eq!(app.read_with(cx, |app, _| app.pending_actions), 0);
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.apply_connection_phase(connection::Phase::Ready, cx);
            app.perform("vcs.pull", json!({"cwd":"/tmp/fixture"}), None, cx);
        })
    });
    cx.run_until_parked();
    let connection::Command::Call {
        tag,
        payload,
        reply,
    } = requests.try_recv().unwrap()
    else {
        panic!("pull receipt")
    };
    assert_eq!(tag, "vcs.pull");
    assert_eq!(payload, json!({"cwd":"/tmp/fixture"}));
    reply.send_blocking(Err("pull rejected".into())).unwrap();
    cx.run_until_parked();
    app.read_with(cx, |app, _| {
        assert_eq!(app.pending_actions, 0);
        assert_eq!(app.status, "pull rejected");
        assert_eq!(app.selected_thread.as_deref(), Some("t"));
    });
}

#[gpui_kit::test]
fn r13_a_sidebar_divider_double_click_restores_default_width(cx: &mut TestAppContext) {
    let (app, cx, _) = harness(cx);
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.sidebar_width = 400.;
            app.reset_splits(cx);
            cx.notify();
        })
    });
    cx.run_until_parked();
    let divider = cx.debug_bounds("sidebar-divider-reset").unwrap();
    cx.simulate_event(gpui_kit::MouseDownEvent {
        button: MouseButton::Left,
        position: divider.center(),
        modifiers: Default::default(),
        click_count: 2,
        first_mouse: false,
    });
    cx.run_until_parked();
    assert_eq!(
        app.read_with(cx, |app, _| app.sidebar_width),
        window_state::SIDEBAR_WIDTH.1
    );
    let sidebar = cx.debug_bounds("sidebar").unwrap();
    assert!((sidebar.size.width - px(window_state::SIDEBAR_WIDTH.1)).abs() <= px(2.));
}

#[gpui_kit::test]
fn r13_a_permission_enable_waits_for_os_receipt_and_preserves_mode_on_denial(
    cx: &mut TestAppContext,
) {
    use gpui_kit::test::TestWindowExt as _;
    let (app, cx, _) = harness(cx);
    cx.simulate_keystrokes("cmd-,");
    cx.simulate_input("notifications");
    cx.run_until_parked();
    for authorization in [
        Authorization::Denied,
        Authorization::Unavailable,
        Authorization::Failed,
        Authorization::Authorized,
    ] {
        let receipt = cx.update(|_, cx| {
            app.update(cx, |app, _| app.notification_platform.queue_authorization())
        });
        let mode = cx.update(|window, _| {
            window
                .within("settings-notification-mode")
                .find(1usize)
                .bounds()
        });
        cx.simulate_click(mode.center(), Default::default());
        cx.update(|_, cx| {
            app.update(cx, |app, cx| {
                // Activation while the system prompt closes must not supersede it.
                app.refresh_notification_authorization(cx);
            })
        });
        cx.run_until_parked();
        app.read_with(cx, |app, _| {
            assert_eq!(
                app.notification_preferences.mode,
                notifications::NotificationMode::Off
            );
            assert_eq!(
                app.notification_state_text(),
                "Requesting notification authorization…"
            );
        });
        assert!(cx.debug_bounds("settings-notification-status").is_some());
        receipt.try_send(authorization).unwrap();
        cx.run_until_parked();
        app.read_with(cx, |app, _| {
            assert_eq!(
                app.notification_preferences.mode,
                if authorization == Authorization::Authorized {
                    notifications::NotificationMode::Notifications
                } else {
                    notifications::NotificationMode::Off
                }
            );
            assert_ne!(
                app.notification_state_text(),
                "Requesting notification authorization…"
            );
            assert!(
                !app.notification_platform
                    .calls
                    .iter()
                    .any(|call| matches!(call, Call::Delivery(_)))
            );
        });
    }
    assert_eq!(
        app.read_with(cx, |app, _| app
            .notification_platform
            .calls
            .iter()
            .filter(|call| **call == Call::AuthorizationRequested)
            .count()),
        4
    );
}

#[gpui_kit::test]
fn r13_a_permission_old_grant_cannot_reenable_after_sound_only_choice(cx: &mut TestAppContext) {
    let (app, cx, _) = harness(cx);
    let receipt = cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            let receipt = app.notification_platform.queue_authorization();
            app.set_notification_preferences(
                notifications::Preferences {
                    mode: notifications::NotificationMode::NotificationsAndSound,
                    in_app: true,
                },
                cx,
            );
            app.set_notification_preferences(
                notifications::Preferences {
                    mode: notifications::NotificationMode::Sound,
                    in_app: true,
                },
                cx,
            );
            receipt
        })
    });
    receipt.try_send(Authorization::Authorized).unwrap();
    cx.run_until_parked();
    assert_eq!(
        app.read_with(cx, |app, _| app.notification_preferences.mode),
        notifications::NotificationMode::Sound
    );
}

#[gpui_kit::test]
fn r13_a_condition_resolution_keeps_identically_worded_rpc_failure(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.selected_thread = None;
            app.report_error("Select a thread first", cx);
            app.report_thread_required(cx);
            app.report_error("Select a thread first", cx);
        })
    });
    cx.run_until_parked();
    cx.update(|window, cx| app.update(cx, |app, cx| app.select_thread("t", window, cx)));
    assert!(
        matches!(requests.try_recv().unwrap(), connection::Command::Subscribe { tag, .. } if tag == "subscribeWorktreeSetup")
    );
    assert!(
        matches!(requests.try_recv().unwrap(), connection::Command::SelectThread(Some(id)) if id == "t")
    );
    cx.run_until_parked();
    app.read_with(cx, |app, _| {
        assert!(app.thread_required_toast.is_none());
        assert_eq!(app.toasts.len(), 1);
        assert_eq!(app.toasts[0].count, 2);
        assert_eq!(app.toasts[0].message, "Select a thread first");
    });
}
