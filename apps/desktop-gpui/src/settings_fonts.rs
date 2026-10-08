//! Independent device font families. Explicit Save validates availability before any reflow.
use crate::NativeApp;
use gpui_kit::component::{
    ActiveTheme, Sizable,
    button::{Button, ButtonVariants},
    input::{Input, InputState},
};
use gpui_kit::{
    App, Context, Entity, Global, IntoElement, SharedString, Window, div, font, prelude::*, px,
};
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct Fonts {
    pub families: [String; 4],
}
impl Global for Fonts {}
impl Fonts {
    pub(crate) fn validate(
        &self,
        available: &[String],
        monospace: impl Fn(&str) -> bool,
    ) -> Result<(), String> {
        for (index, name) in self.families.iter().enumerate() {
            if name.is_empty() {
                continue;
            }
            if name.len() > 200 || !available.iter().any(|installed| installed == name) {
                return Err(format!("{} font is not installed: {name}", LABELS[index]));
            }
            if index >= 2 && !monospace(name) {
                return Err(format!("{} requires a monospace family", LABELS[index]));
            }
        }
        Ok(())
    }
    fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let temporary = path.with_extension("json.tmp");
        std::fs::write(&temporary, serde_json::to_vec_pretty(self)?)?;
        std::fs::rename(temporary, path)
    }
}
const LABELS: [&str; 4] = ["UI", "Prose", "Code", "Terminal"];
fn path() -> std::path::PathBuf {
    t3code_desktop_gpui::drafts::default_path().with_file_name("fonts.json")
}
pub(crate) fn current(cx: &App) -> Fonts {
    cx.try_global::<Fonts>().cloned().unwrap_or_default()
}
fn monospace(name: &str, cx: &App) -> bool {
    let text = cx.text_system();
    let id = text.resolve_font(&font(SharedString::from(name.to_string())));
    let Ok(baseline) = text.advance(id, px(14.), 'i') else {
        return false;
    };
    ['W', '0', '.'].into_iter().all(|ch| {
        text.advance(id, px(14.), ch)
            .is_ok_and(|size| (size.width - baseline.width).abs() < px(0.1))
    })
}
pub(crate) fn init(cx: &mut App) {
    let mut fonts = if cfg!(test) {
        Fonts::default()
    } else {
        std::fs::read(path())
            .ok()
            .filter(|bytes| bytes.len() <= 8192)
            .and_then(|bytes| serde_json::from_slice::<Fonts>(&bytes).ok())
            .unwrap_or_default()
    };
    let available = cx.text_system().all_font_names();
    // A font removed between launches must not silently claim to be applied.
    for index in 0..4 {
        let mut candidate = Fonts::default();
        candidate.families[index] = fonts.families[index].clone();
        if candidate
            .validate(&available, |name| monospace(name, cx))
            .is_err()
        {
            fonts.families[index].clear();
        }
    }
    cx.set_global(fonts);
}
pub(crate) fn family(index: usize, fallback: SharedString, cx: &App) -> SharedString {
    current(cx)
        .families
        .get(index)
        .filter(|s| !s.is_empty())
        .cloned()
        .map(SharedString::from)
        .unwrap_or(fallback)
}
pub(crate) struct Editor {
    inputs: [Entity<InputState>; 4],
    error: Option<String>,
}
impl Editor {
    pub fn new(window: &mut Window, cx: &mut App) -> Self {
        let fonts = current(cx);
        Self {
            inputs: std::array::from_fn(|index| {
                cx.new(|cx| {
                    InputState::new(window, cx)
                        .placeholder("Theme default")
                        .default_value(fonts.families[index].clone())
                })
            }),
            error: None,
        }
    }
}
impl NativeApp {
    pub(crate) fn conversation_typography_style(
        &self,
        cx: &App,
    ) -> gpui_kit::component::text::TextViewStyle {
        let mut style = gpui_kit::component::text::TextViewStyle {
            heading_base_font_size: self.prose_font_size(cx),
            ..Default::default()
        };
        style.code_block.text.font_family =
            Some(family(2, cx.theme().mono_font_family.clone(), cx));
        style.code_block.text.font_size = Some(self.code_font_size(cx).into());
        style
    }
    fn save_font_families(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let fonts = Fonts {
            families: std::array::from_fn(|index| {
                self.font_editor.inputs[index]
                    .read(cx)
                    .value()
                    .trim()
                    .to_string()
            }),
        };
        let result = fonts
            .validate(&cx.text_system().all_font_names(), |name| {
                monospace(name, cx)
            })
            .and_then(|()| {
                if cfg!(test) {
                    Ok(())
                } else {
                    fonts
                        .save(&path())
                        .map_err(|e| format!("Could not save fonts: {e}"))
                }
            });
        match result {
            Ok(()) => {
                cx.set_global(fonts);
                self.font_editor.error = None;
                crate::theme::apply(Some(window), cx);
                self.report_success("Font families saved", cx);
            }
            Err(error) => self.font_editor.error = Some(error),
        }
        cx.notify();
    }
    pub(crate) fn render_font_families(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        div().flex().flex_col().gap_2()
            .child(div().text_sm().child("Font families · device-local"))
            .child(div().text_xs().text_color(cx.theme().muted_foreground).child("Use an installed family name. Empty restores the theme default. Code must be monospace. Save validates without changing the current font on failure."))
            .children(LABELS.into_iter().enumerate().map(|(index, label)| div().flex().items_center().gap_2().child(div().w_24().text_sm().child(label)).child(div().flex_1().debug_selector(move || format!("font-family-{index}")).child(Input::new(&self.font_editor.inputs[index]).small()))))
            .when_some(self.font_editor.error.clone(), |view, error| view.child(div().text_sm().text_color(cx.theme().danger).child(error)))
            .child(div().flex().gap_1()
                .child(div().debug_selector(|| "font-family-save".into()).child(Button::new("font-family-save").label("Save families").ghost().small().on_click(cx.listener(|app, _, window, cx| app.save_font_families(window, cx)))))
                .child(Button::new("font-family-reset").debug_selector(|| "font-family-reset".into()).label("Reset families").ghost().small().on_click(cx.listener(|app, _, window, cx| {
                    for input in &app.font_editor.inputs { input.update(cx, |input, cx| input.set_value("", window, cx)); }
                    app.save_font_families(window, cx);
                }))))
            .child(div().font_family(family(1, cx.theme().font_family.clone(), cx)).text_size(self.prose_font_size(cx)).child("Prose preview — independent from interface text."))
            .child(div().font_family(family(1, cx.theme().font_family.clone(), cx)).text_size(self.prose_font_size(cx)).child(gpui_kit::component::text::TextView::markdown("font-markdown-preview", "Typography preview\n\n```rust\nlet answer = 42;\n```".to_string()).style(self.conversation_typography_style(cx))))
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn r13_f_fonts_validate_available_monospace_and_restart() {
        let available = vec!["Sans".into(), "Mono".into()];
        let mut fonts = Fonts {
            families: ["Sans".into(), "Sans".into(), "Mono".into(), "Mono".into()],
        };
        fonts.validate(&available, |name| name == "Mono").unwrap();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("fonts.json");
        fonts.save(&path).unwrap();
        let restored: Fonts = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        assert_eq!(restored.families, fonts.families);
        fonts.families[3] = "Sans".into();
        assert!(fonts.validate(&available, |name| name == "Mono").is_err());
        fonts.families[3] = "Missing".into();
        assert!(fonts.validate(&available, |_| true).is_err());
        fonts.families = Default::default();
        fonts.validate(&[], |_| false).unwrap();
    }
}
