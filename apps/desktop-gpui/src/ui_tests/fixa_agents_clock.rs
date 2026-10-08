use super::*;

#[gpui_kit::test]
fn fixa_elapsed_ticks_do_not_notify_the_global_app(cx: &mut TestAppContext) {
    let (app, cx, _) = harness(cx);
    cx.update(|_, cx| app.update(cx, |app, cx| {
        app.detail.thread = Some(json!({"id":"t", "session":{"status":"running"},"activities":[{"id":"spawn","kind":"task.started","createdAt":"2026-01-01T00:00:00Z","payload":{"taskId":"child","agentKind":"agent","title":"Research"}}]}).into());
        cx.notify();
    }));
    cx.run_until_parked();
    let tab = cx.debug_bounds("right-tab-agents").unwrap();
    cx.simulate_click(tab.center(), Default::default());
    cx.run_until_parked();
    assert!(cx.debug_bounds("agent-timeline-child").is_some());
    let notifications = std::rc::Rc::new(std::cell::Cell::new(0));
    let count = notifications.clone();
    let _observer = cx.update(|_, cx| cx.observe(&app, move |_, _| count.set(count.get() + 1)));
    // Advance the deterministic executor, never sleep/poll for presentation timers.
    for _ in 0..3 {
        cx.background_executor
            .advance_clock(std::time::Duration::from_secs(1));
        cx.run_until_parked();
    }
    assert_eq!(
        notifications.get(),
        0,
        "elapsed labels, not the shell/composer/sidebar, receive tick notifications"
    );
}
