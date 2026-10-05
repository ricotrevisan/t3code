//! Zed's `One Dark` palette for the GPUI desktop client.
//!
//! The colors are ported from Zed's bundled `One Dark` theme. Keeping them in a
//! theme file instead of literals at call sites lets the interface resolve
//! semantic roles through `cx.theme()`.

use gpui_kit::component::{Theme, ThemeMode, ThemeRegistry};
use gpui_kit::{App, SharedString};

const ONE_DARK: &str = include_str!("../themes/one-dark.json");

/// Install the app's default theme. Call after `gpui_kit::init`.
pub fn init(cx: &mut App) {
    if let Err(error) = ThemeRegistry::global_mut(cx).load_themes_from_str(ONE_DARK) {
        eprintln!("desktop-gpui: failed to load the One Dark theme: {error:#}");
        Theme::change(ThemeMode::Dark, None, cx);
        return;
    }

    let theme = ThemeRegistry::global(cx)
        .themes()
        .get(&SharedString::from("One Dark"))
        .cloned();
    if let Some(theme) = theme {
        Theme::update(cx, |current| current.apply_config(&theme));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::component::ThemeSet;

    #[test]
    fn one_dark_theme_parses_and_is_dark() {
        let set: ThemeSet = serde_json::from_str(ONE_DARK).expect("One Dark theme is valid");
        let one_dark = set
            .themes
            .iter()
            .find(|theme| &*theme.name == "One Dark")
            .expect("One Dark entry is present");
        assert!(one_dark.mode.is_dark());
    }
}
