//! Real GPUI chrome interaction without launching WebKit: visibility remains
//! false, as it does when the shell has an overlay covering the native panel.
use gpui_kit::component::Root;
use gpui_kit::{AppContext as _, Entity, TestAppContext, VisualTestContext, px, size};
use t3code_desktop_gpui::surfaces::browser::{BrowserPanel, model::Profile};

fn harness(cx: &mut TestAppContext) -> (Entity<BrowserPanel>, &mut VisualTestContext) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        crate::theme::init(cx);
    });
    let mut panel = None;
    let (_, cx) = cx.add_window_view(|window, cx| {
        let entity = cx.new(|cx| BrowserPanel::new(window, cx));
        panel = Some(entity.clone());
        Root::new(entity, window, cx)
    });
    cx.simulate_resize(size(px(1400.), px(1000.)));
    cx.run_until_parked();
    (panel.unwrap(), cx)
}
fn click(cx: &mut VisualTestContext, selector: &'static str) {
    let bounds = cx
        .debug_bounds(selector)
        .unwrap_or_else(|| panic!("missing {selector}"));
    cx.simulate_click(bounds.center(), Default::default());
    cx.run_until_parked();
}

#[gpui_kit::test]
fn r14_w_browser_add_activate_rename_reorder_close_and_builtin_return(cx: &mut TestAppContext) {
    let (panel, cx) = harness(cx);
    click(cx, "browser-add");
    click(cx, "browser-add-incognito");
    let (a, b) = panel.read_with(cx, |panel, _| {
        assert_eq!(panel.tabs.entries.len(), 2);
        assert_eq!(panel.tabs.entries[1].profile, Profile::Incognito);
        (panel.tabs.entries[0].id, panel.tabs.entries[1].id)
    });
    assert_eq!((a, b), (1, 2));
    click(cx, "browser-tab-1");
    assert_eq!(panel.read_with(cx, |panel, _| panel.tabs.active), Some(a));
    click(cx, "browser-tab-2");
    click(cx, "browser-rename");
    cx.simulate_keystrokes("cmd-a");
    cx.simulate_input("QA preview");
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    assert_eq!(
        panel.read_with(cx, |panel, _| panel.tabs.entries[1].label.clone()),
        "QA preview"
    );
    click(cx, "browser-move-left");
    assert_eq!(panel.read_with(cx, |panel, _| panel.tabs.entries[0].id), b);
    click(cx, "browser-close-2");
    assert_eq!(panel.read_with(cx, |panel, _| panel.tabs.active), Some(a));
    click(cx, "browser-close-1");
    assert!(!panel.read_with(cx, |panel, _| panel.browser_active()));
}

#[gpui_kit::test]
fn r14_w_browser_preview_remote_route_relay_failure_and_scope_retirement(cx: &mut TestAppContext) {
    let (panel, cx) = harness(cx);
    cx.update(|window, cx| {
        panel.update(cx, |panel, cx| {
            panel.sync_scope("office", cx);
            panel.open_preview(
                "http://localhost:5173/app?q=1#top",
                "http://100.124.111.25:8789",
                Profile::Default,
                window,
                cx,
            );
        })
    });
    cx.run_until_parked();
    assert_eq!(panel.read_with(cx, |panel, _| panel.tabs.entries.len()), 1);
    // Same entry point rejects relay ports before allocating a tab/view/store.
    cx.update(|window, cx| {
        panel.update(cx, |panel, cx| {
            assert!(
                panel
                    .open_preview(
                        "http://localhost:5173/app",
                        "https://relay.example.com",
                        Profile::Incognito,
                        window,
                        cx
                    )
                    .is_none()
            );
        })
    });
    assert_eq!(panel.read_with(cx, |panel, _| panel.tabs.entries.len()), 1);
    assert!(panel.read_with(cx, |panel, _| {
        panel.error.as_ref().unwrap().contains("gateway")
    }));
    cx.update(|_, cx| panel.update(cx, |panel, cx| panel.sync_scope("lab", cx)));
    cx.run_until_parked();
    assert!(panel.read_with(cx, |panel, _| panel.tabs.entries.is_empty()));
    assert!(!panel.read_with(cx, |panel, _| panel.browser_active()));
    click(cx, "browser-add");
    assert_eq!(
        panel.read_with(cx, |panel, _| panel.tabs.entries[0].id),
        2,
        "retiring a scope must not reuse an ID with queued native callbacks"
    );
}

#[gpui_kit::test]
fn r14_w_browser_deferred_preview_fences_scope_switch(cx: &mut TestAppContext) {
    let (panel, cx) = harness(cx);
    cx.update(|_, cx| {
        panel.update(cx, |panel, cx| panel.sync_scope("office", cx));
        BrowserPanel::queue_preview(
            &panel,
            "office",
            "http://localhost:5173",
            "http://100.124.111.25:8789",
            cx,
        );
        panel.update(cx, |panel, cx| panel.sync_scope("lab", cx));
    });
    cx.run_until_parked();
    assert!(
        panel.read_with(cx, |panel, _| panel.tabs.entries.is_empty()),
        "deferred office preview must not open in lab"
    );
    cx.update(|_, cx| {
        BrowserPanel::queue_preview(
            &panel,
            "lab",
            "http://localhost:4000",
            "http://100.87.189.5:8789",
            cx,
        );
    });
    cx.run_until_parked();
    assert_eq!(panel.read_with(cx, |panel, _| panel.tabs.entries.len()), 1);
}
