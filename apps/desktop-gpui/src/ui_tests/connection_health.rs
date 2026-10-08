use super::*;

fn synchronized(app: &Entity<NativeApp>, cx: &mut VisualTestContext) {
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.apply_shell_item(shell::ShellItem::Synchronized, cx)
                .unwrap();
            cx.notify();
        })
    });
}

#[gpui_kit::test]
fn connection_health_sender_is_not_readiness_and_resync_restores_ready(cx: &mut TestAppContext) {
    let (app, cx, _) = harness(cx);
    for phase in [
        connection::Phase::Connecting,
        connection::Phase::Authenticating,
        connection::Phase::Syncing,
    ] {
        cx.update(|_, cx| app.update(cx, |app, cx| app.apply_connection_phase(phase, cx)));
        assert!(!app.read_with(cx, |app, _| app.connection_ready()));
    }
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.shell.synchronized = false;
            app.apply_connection_phase(connection::Phase::Ready, cx);
        })
    });
    assert!(!app.read_with(cx, |app, _| app.connection_ready()));
    synchronized(&app, cx);
    assert!(app.read_with(cx, |app, _| app.connection_ready()));
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.shell.synchronized = false;
            app.apply_connection_phase(
                connection::Phase::Reconnecting {
                    attempt: 3,
                    retry_in: std::time::Duration::from_secs(8),
                    reason: "Transport closed".into(),
                },
                cx,
            );
        })
    });
    assert!(!app.read_with(cx, |app, _| app.connection_ready()));
    assert!(
        app.read_with(cx, |app, _| app.connection_state_text())
            .contains("retry in 8s")
    );
    synchronized(&app, cx);
    assert!(app.read_with(cx, |app, _| app.connection_ready()));
}

#[gpui_kit::test]
fn connection_health_retry_click_queues_immediate_attempt(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.apply_connection_phase(
                connection::Phase::Reconnecting {
                    attempt: 2,
                    retry_in: std::time::Duration::from_secs(30),
                    reason: "Socket closed".into(),
                },
                cx,
            );
        })
    });
    cx.run_until_parked();
    let retry = cx
        .debug_bounds("connection-retry-now")
        .expect("visible recovery action");
    cx.simulate_click(retry.center(), Default::default());
    cx.run_until_parked();
    assert!(matches!(
        requests.try_recv().unwrap(),
        connection::Command::RetryNow
    ));
}

#[gpui_kit::test]
fn connection_health_environment_uses_descriptor_not_endpoint(cx: &mut TestAppContext) {
    let (app, cx, _) = harness(cx);
    synchronized(&app, cx);
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.update_environment_identity(&json!({"environment":{"label":"Build Mac"}}));
            cx.notify();
        })
    });
    app.read_with(cx, |app, _| {
        assert_eq!(app.environment_name.as_deref(), Some("Build Mac"));
        assert!(app.connection_ready());
    });
    assert!(cx.debug_bounds("environment-chip").is_some());
}
