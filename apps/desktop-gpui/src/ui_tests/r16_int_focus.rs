use super::*;

#[gpui_kit::test]
fn r16_int_collapsed_header_picker_has_one_active_trap_and_cycles_inside_it(
    cx: &mut TestAppContext,
) {
    let (app, cx, _) = harness(cx);
    r14_e_header::ensure_control(cx, "checkout-selector");
    let trigger = cx.debug_bounds("checkout-selector").unwrap();
    cx.simulate_click(trigger.center(), Default::default());
    cx.run_until_parked();
    cx.update(|window, cx| app.read_with(cx, |app, cx| {
        assert!(app.settings_shortcut_focus.contains_focused(window, cx));
        assert!(!app.header_rename.overflow_focus.get().unwrap().contains_focused(window, cx), "an outer header trap must not contain the branch picker; Kit chooses active traps from an unordered map");
        assert_eq!(gpui_kit::base::active_focus_trap(window, cx), Some(app.settings_shortcut_focus.clone()));
    }));
    for key in ["tab", "shift-tab"] {
        for _ in 0..24 {
            cx.simulate_keystrokes(key);
            cx.run_until_parked();
            cx.update(|window, cx| {
                app.read_with(cx, |app, cx| {
                    assert!(app.settings_shortcut_focus.contains_focused(window, cx))
                })
            });
        }
    }
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    assert!(app.read_with(cx, |app, _| app.branch_picker.is_none()));
}
