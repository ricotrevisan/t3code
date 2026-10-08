use super::*;
use gpui_kit::TextRun;

#[gpui_kit::test]
fn statusbar_polb_full_host_fits_at_1280(cx: &mut TestAppContext) {
    let (app, cx, _) = harness(cx);
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.environment_scope = "http://127.0.0.1:3773".into();
            app.environment_name = None;
            app.shell.threads[0].branch = Some("main".into());
            app.shell.threads[0].worktree_path = Some("/tmp/fixture-checkout".into());
            cx.notify();
        })
    });
    cx.simulate_resize(size(px(1280.), px(983.)));
    cx.run_until_parked();
    let chip = cx.debug_bounds("environment-chip").unwrap();
    let label = app.read_with(cx, |app, _| {
        format!("127.0.0.1 · {}", app.connection_state_text())
    });
    // Measure the actual text against its painted control, including kit's
    // xs icon, gap and horizontal padding. This catches real ellipsis pressure.
    let required = cx.update(|window, cx| {
        let run = TextRun {
            len: label.len(),
            font: gpui_kit::font(cx.theme().font_family.clone()),
            color: cx.theme().foreground,
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        window
            .text_system()
            .shape_line(label.into(), window.rem_size() * 0.75, &[run], None)
            .width
            + window.rem_size() * 1.75
    });
    assert!(
        chip.size.width >= required,
        "full host must fit, not ellipsize: {chip:?}, needs {required:?}"
    );
    let message = cx.debug_bounds("status-message").unwrap();
    assert!(message.size.width >= px(96.));
    cx.simulate_click(chip.center(), Default::default());
    cx.run_until_parked();
    click_connection_settings(cx);
    assert!(app.read_with(cx, |app, _| app.settings_open));
}
