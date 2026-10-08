//! Browser chrome is driven through a GPUI test window. TestPlatform cannot
//! host NSViews; the physical WebKit → GPUI responder handoff needs the manual
//! acceptance path in the lane report, not an assertion about a fake WebKit.
use super::*;
use gpui_kit::component::Root;
use gpui_kit::{TestAppContext, VisualTestContext, px, size};

#[gpui_kit::test]
fn r14_k_rw23_address_unicode_input_submits_and_recovers_from_invalid_url(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let mut panel = None;
    let (_, cx) = cx.add_window_view(|window, cx| {
        let entity = cx.new(|cx| BrowserPanel::new(window, cx));
        panel = Some(entity.clone());
        Root::new(entity, window, cx)
    });
    let panel = panel.unwrap();
    cx.simulate_resize(size(px(1000.), px(700.)));
    cx.run_until_parked();
    click(cx, "browser-add");
    click(cx, "browser-address");
    cx.simulate_input("file:///private/not-a-preview");
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    assert!(panel.read_with(cx, |panel, _| panel.browsers[&1].navigation.error.is_some()));
    assert!(panel.read_with(cx, |panel, _| panel.browsers[&1].pending_url.is_none()));
    cx.simulate_keystrokes("cmd-a");
    cx.simulate_input("https://example.com/café?q=🙂");
    // A repaint must not replace the focused URL draft with the page URL.
    cx.update(|_, cx| panel.update(cx, |_, cx| cx.notify()));
    cx.run_until_parked();
    assert_eq!(
        panel.read_with(cx, |panel, cx| panel.browsers[&1]
            .address
            .read(cx)
            .value()
            .to_string()),
        "https://example.com/café?q=🙂"
    );
    click(cx, "browser-go");
    assert!(panel.read_with(cx, |panel, _| panel.browsers[&1].navigation.error.is_none()));
    assert_eq!(
        panel.read_with(cx, |panel, _| panel.browsers[&1]
            .pending_url
            .as_ref()
            .unwrap()
            .as_str()
            .to_owned()),
        "https://example.com/caf%C3%A9?q=%F0%9F%99%82"
    );
    click(cx, "browser-close-1");
    assert!(panel.read_with(cx, |panel, _| panel.browsers.is_empty()));
}

fn click(cx: &mut VisualTestContext, selector: &'static str) {
    let bounds = cx.debug_bounds(selector).unwrap();
    cx.simulate_click(bounds.center(), Default::default());
    cx.run_until_parked();
}
