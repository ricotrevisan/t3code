//! Device-local settings drive the actual editor, view actions and application theme.
use super::*;
use gpui_kit::component::{ActiveTheme, Theme};
fn click(cx: &mut VisualTestContext, selector: &'static str) {
    let target = cx.debug_bounds(selector).unwrap().center();
    cx.simulate_click(target, Default::default());
    cx.run_until_parked();
}
fn settings(app: &Entity<NativeApp>, cx: &mut VisualTestContext, query: &str) {
    cx.simulate_resize(size(px(1600.), px(1600.)));
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.settings_open = true;
            app.settings_query
                .update(cx, |input, cx| input.set_value(query, window, cx));
            cx.notify();
        })
    });
    cx.run_until_parked();
}
#[gpui_kit::test]
fn r13_f_theme_editor_save_use_edit_reject_remove_and_reset(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    settings(&app, cx, "theme library");
    click(cx, "theme-create");
    cx.simulate_keystrokes("cmd-a");
    cx.simulate_input(r##"{"themes":[{"name":"Night test","mode":"dark","colors":{"background":"#121212","foreground":"#ffffff"}}]}"##);
    click(cx, "theme-save");
    assert!(!app.read_with(cx, |app, _| app.theme_editor.open));
    click(cx, "theme-use-4");
    cx.update(|_, cx| {
        assert_eq!(Theme::global(cx).theme_name().as_ref(), "Night test");
        assert!(cx.theme().is_dark());
    });
    click(cx, "theme-edit-4");
    cx.simulate_keystrokes("cmd-a");
    cx.simulate_input("invalid JSON");
    click(cx, "theme-save");
    assert!(app.read_with(cx, |app, _| app.theme_editor.error.is_some()));
    cx.update(|_, cx| assert_eq!(Theme::global(cx).theme_name().as_ref(), "Night test"));
    cx.simulate_keystrokes("cmd-a");
    // Reacquire the real editor after the save action owns focus.
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.theme_editor
                .input
                .update(cx, |input, cx| input.focus(window, cx))
        })
    });
    cx.simulate_keystrokes("cmd-a");
    cx.simulate_input(r##"{"themes":[{"name":"Renamed night","mode":"dark","colors":{"background":"#202020","foreground":"#eeeeee"}}]}"##);
    click(cx, "theme-save");
    cx.update(|_, cx| assert_eq!(Theme::global(cx).theme_name().as_ref(), "Renamed night"));
    click(cx, "theme-use-1");
    cx.update(|window, cx| {
        theme::update(window, cx, |p| {
            p.appearance = settings_appearance::Appearance::Light
        })
        .unwrap()
    });
    cx.update(|_, cx| assert_eq!(Theme::global(cx).theme_name().as_ref(), "One Light"));
    cx.update(|window, cx| {
        theme::update(window, cx, |p| {
            p.appearance = settings_appearance::Appearance::Dark
        })
        .unwrap()
    });
    cx.update(|_, cx| assert_eq!(Theme::global(cx).theme_name().as_ref(), "Renamed night"));
    click(cx, "theme-reset");
    cx.update(|_, cx| assert_eq!(Theme::global(cx).theme_name().as_ref(), "One Dark"));
    click(cx, "theme-use-4");
    click(cx, "theme-remove-4");
    click(cx, "theme-remove-cancel");
    cx.update(|_, cx| assert_eq!(Theme::global(cx).theme_name().as_ref(), "Renamed night"));
    click(cx, "theme-remove-4");
    click(cx, "theme-remove-confirm");
    cx.update(|_, cx| {
        assert!(
            cx.global::<settings_theme_library::Library>()
                .themes
                .is_empty()
        );
        assert_eq!(Theme::global(cx).theme_name().as_ref(), "One Dark");
    });
    assert!(
        requests.is_empty(),
        "device preferences must not mutate the selected server"
    );
}
#[gpui_kit::test]
fn r13_f_font_unknown_family_keeps_applied_typography(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    settings(&app, cx, "font families");
    let before = cx.update(|_, cx| cx.theme().font_family.clone());
    click(cx, "font-family-0");
    cx.simulate_keystrokes("cmd-a");
    cx.simulate_input("Not Installed R13 Font");
    click(cx, "font-family-save");
    cx.update(|_, cx| {
        assert_eq!(cx.theme().font_family, before);
        assert!(settings_fonts::current(cx).families[0].is_empty());
    });
    assert!(requests.is_empty());
}

#[gpui_kit::test]
fn r13_f_theme_file_picker_import_export_cancel_and_bad_input(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    settings(&app, cx, "theme library");
    let directory = tempfile::tempdir().unwrap();
    let import = directory.path().join("theme.json");
    std::fs::write(&import, r##"{"themes":[{"name":"File night","mode":"dark","colors":{"background":"#232323","foreground":"#eeeeee"}}]}"##).unwrap();
    click(cx, "theme-import");
    cx.simulate_path_prompt_response(|options| {
        assert!(options.files && !options.multiple && !options.directories);
        Some(vec![import.clone()])
    });
    cx.run_until_parked();
    cx.update(|_, cx| {
        assert_eq!(
            cx.global::<settings_theme_library::Library>().themes.len(),
            1
        )
    });
    let export = directory.path().join("chosen.json");
    click(cx, "theme-export-4");
    cx.simulate_new_path_selection(|_| Some(export.clone()));
    cx.run_until_parked();
    let saved: gpui_kit::component::ThemeSet =
        serde_json::from_slice(&std::fs::read(&export).unwrap()).unwrap();
    assert_eq!(saved.themes[0].name.as_ref(), "File night");
    assert_eq!(
        saved.themes[0].colors.background.as_ref().unwrap().as_ref(),
        "#232323"
    );
    click(cx, "theme-import");
    cx.simulate_path_prompt_response(|_| None);
    cx.run_until_parked();
    std::fs::write(&import, "not JSON").unwrap();
    click(cx, "theme-import");
    cx.simulate_path_prompt_response(|_| Some(vec![import.clone()]));
    cx.run_until_parked();
    cx.update(|_, cx| {
        assert_eq!(
            cx.global::<settings_theme_library::Library>().themes.len(),
            1
        )
    });
    assert!(requests.is_empty());
}

#[gpui_kit::test]
fn r13_f_font_validated_family_applies_and_reset_does_not_leak(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    settings(&app, cx, "font families");
    let before = cx.update(|_, cx| cx.theme().font_family.clone());
    // TestPlatform has no installed font enumeration. Validate against an explicit
    // installed-family fixture, then exercise theme application and the real reset control.
    let installed = "Fixture Sans".to_string();
    let fonts = settings_fonts::Fonts {
        families: [
            installed.clone(),
            String::new(),
            String::new(),
            String::new(),
        ],
    };
    fonts
        .validate(std::slice::from_ref(&installed), |_| false)
        .unwrap();
    cx.update(|window, cx| {
        cx.set_global(fonts);
        theme::apply(Some(window), cx);
    });
    cx.update(|_, cx| assert_eq!(cx.theme().font_family.as_ref(), installed));
    click(cx, "font-family-reset");
    cx.update(|_, cx| {
        assert_eq!(cx.theme().font_family, before);
        assert!(
            settings_fonts::current(cx)
                .families
                .iter()
                .all(String::is_empty)
        );
    });
    assert!(requests.is_empty());
}
