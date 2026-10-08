//! Zed One appearance variants and device-local scaling.
use crate::settings_appearance::{self, Appearance, Preferences};
use gpui_kit::component::{Theme, ThemeMode, ThemeRegistry};
use gpui_kit::{App, SharedString, Window, WindowAppearance, px};

const ONE_DARK: &str = include_str!("../themes/one-dark.json");
// Zed One Light palette, mapped to the kit's semantic UI roles.
const ONE_LIGHT: &str = r##"{"name":"Zed One Light","themes":[{"name":"One Light","mode":"light","font.size":14,"radius":4,"radius.lg":6,"colors":{
"background":"#fafafa","foreground":"#383a42","border":"#d3d3d3","input.border":"#d3d3d3",
"primary.background":"#4078f2","primary.foreground":"#ffffff","ring":"#4078f2",
"secondary.background":"#eaeaeb","secondary.foreground":"#383a42",
"muted.background":"#eaeaeb","muted.foreground":"#696c77",
"popover.background":"#fafafa","popover.foreground":"#383a42",
"list.background":"#fafafa","list.hover.background":"#eaeaeb","list.active.background":"#dbdbdc",
"sidebar.background":"#eaeaeb","sidebar.foreground":"#383a42","sidebar.border":"#d3d3d3",
"title_bar.background":"#eaeaeb","title_bar.border":"#d3d3d3","status_bar.background":"#eaeaeb","status_bar.border":"#d3d3d3",
"tab.background":"#eaeaeb","tab.active.background":"#fafafa","tab.foreground":"#696c77","tab.active.foreground":"#383a42",
"selection.background":"#4078f233","caret":"#4078f2","danger.background":"#e45649","danger.foreground":"#ffffff",
"success.background":"#50a14f","success.foreground":"#ffffff","warning.background":"#c18401","warning.foreground":"#ffffff"
}}]}"##;

fn contrast_source(source: &str, dark: bool) -> String {
    let mut set: serde_json::Value = serde_json::from_str(source).expect("bundled theme");
    set["themes"][0]["name"] = if dark {
        "High Contrast Dark"
    } else {
        "High Contrast Light"
    }
    .into();
    let colors = &mut set["themes"][0]["colors"];
    let (background, foreground, secondary, border) = if dark {
        ("#000000", "#ffffff", "#191919", "#a0a0a0")
    } else {
        ("#ffffff", "#000000", "#eeeeee", "#505050")
    };
    for role in [
        "background",
        "popover.background",
        "list.background",
        "tab.active.background",
    ] {
        colors[role] = background.into();
    }
    for role in [
        "foreground",
        "popover.foreground",
        "muted.foreground",
        "sidebar.foreground",
        "tab.foreground",
        "tab.active.foreground",
    ] {
        colors[role] = foreground.into();
    }
    for role in [
        "muted.background",
        "secondary.background",
        "sidebar.background",
        "title_bar.background",
        "status_bar.background",
        "tab.background",
    ] {
        colors[role] = secondary.into();
    }
    for role in [
        "border",
        "input.border",
        "sidebar.border",
        "title_bar.border",
        "status_bar.border",
    ] {
        colors[role] = border.into();
    }
    set.to_string()
}

pub(crate) fn bundled_names() -> &'static [&'static str] {
    &[
        "One Dark",
        "One Light",
        "High Contrast Dark",
        "High Contrast Light",
    ]
}

pub(crate) fn bundled_config(name: &str) -> gpui_kit::component::ThemeConfig {
    let source = match name {
        "One Light" => ONE_LIGHT.to_string(),
        "High Contrast Dark" => contrast_source(ONE_DARK, true),
        "High Contrast Light" => contrast_source(ONE_LIGHT, false),
        _ => ONE_DARK.to_string(),
    };
    serde_json::from_str::<gpui_kit::component::ThemeSet>(&source)
        .expect("bundled theme")
        .themes
        .remove(0)
}

fn path() -> std::path::PathBuf {
    t3code_desktop_gpui::drafts::default_path().with_file_name("appearance.json")
}

pub fn init(cx: &mut App) {
    crate::settings_environments::init(cx);
    crate::settings_theme_library::init(cx);
    crate::settings_fonts::init(cx);
    let contrast_dark = contrast_source(ONE_DARK, true);
    let contrast_light = contrast_source(ONE_LIGHT, false);
    for source in [ONE_DARK, ONE_LIGHT, &contrast_dark, &contrast_light] {
        if let Err(error) = ThemeRegistry::global_mut(cx).load_themes_from_str(source) {
            eprintln!("desktop-gpui: failed to load Zed theme: {error:#}");
        }
    }
    cx.set_global(if cfg!(test) {
        Preferences::default()
    } else {
        Preferences::load(&path())
    });
    apply(None, cx);
}

pub fn observe(window: &mut Window, cx: &mut App) {
    apply(Some(window), cx);
    window
        .observe_window_appearance(|window, cx| {
            if settings_appearance::current(cx).appearance == Appearance::System {
                apply(Some(window), cx);
            }
        })
        .detach();
}

pub(crate) fn apply(window: Option<&mut Window>, cx: &mut App) {
    let preferences = settings_appearance::current(cx);
    let appearance = window
        .as_ref()
        .map(|window| window.appearance())
        .unwrap_or_else(|| cx.window_appearance());
    let dark = preferences.appearance.dark(matches!(
        appearance,
        WindowAppearance::Dark | WindowAppearance::VibrantDark
    ));
    let name = match (preferences.variant, dark) {
        (settings_appearance::Variant::HighContrast, true) => "High Contrast Dark",
        (settings_appearance::Variant::HighContrast, false) => "High Contrast Light",
        (_, true) => "One Dark",
        (_, false) => "One Light",
    };
    let config = cx
        .try_global::<crate::settings_theme_library::Library>()
        .and_then(|library| library.selected(dark))
        .or_else(|| {
            ThemeRegistry::global(cx)
                .themes()
                .get(&SharedString::from(name))
                .cloned()
        });
    Theme::change(
        if dark {
            ThemeMode::Dark
        } else {
            ThemeMode::Light
        },
        None,
        cx,
    );
    let fonts = crate::settings_fonts::current(cx);
    Theme::update(cx, |theme| {
        // Config fields are optional: reset them before applying another variant,
        // otherwise a removed custom family/radius/highlighter would leak into it.
        let defaults = Theme::default();
        theme.font_family = defaults.font_family;
        theme.mono_font_family = defaults.mono_font_family;
        theme.mono_font_size = defaults.mono_font_size;
        theme.radius = defaults.radius;
        theme.radius_lg = defaults.radius_lg;
        theme.shadow = defaults.shadow;
        theme.highlight_theme = if dark {
            gpui_kit::component::highlighter::HighlightTheme::default_dark()
        } else {
            defaults.highlight_theme
        };
        if let Some(config) = config {
            theme.apply_config(&config);
        }
        // Kit Switch uses this token for its unchecked track. Keep it visible
        // on the popover/background in every appearance and contrast variant.
        theme.switch = theme.muted_foreground;
        theme.switch_thumb = theme.background;
        theme.font_size = px(preferences.effective_size());
        if !fonts.families[0].is_empty() {
            theme.font_family = fonts.families[0].clone().into();
        }
        if !fonts.families[2].is_empty() {
            theme.mono_font_family = fonts.families[2].clone().into();
        }
    });
    if let Some(window) = window {
        window.set_rem_size(px(preferences.effective_size()));
    }
}

pub fn update(
    window: &mut Window,
    cx: &mut App,
    edit: impl FnOnce(&mut Preferences),
) -> std::io::Result<()> {
    let mut preferences = settings_appearance::current(cx);
    edit(&mut preferences);
    let preferences = preferences.normalized();
    // Do not present a saved preference when the device write failed.
    if !cfg!(test) {
        preferences.save(&path())?;
    }
    cx.set_global(preferences);
    apply(Some(window), cx);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::component::ThemeSet;
    #[test]
    fn zed_variants_parse_in_registry_format() {
        for (source, dark) in [(ONE_DARK, true), (ONE_LIGHT, false)] {
            let set: ThemeSet = serde_json::from_str(source).unwrap();
            assert_eq!(set.themes[0].mode.is_dark(), dark);
        }
    }
}
