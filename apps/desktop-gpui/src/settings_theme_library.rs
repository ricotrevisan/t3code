//! Device-owned Kit theme collections. Mutations publish only after the atomic write succeeds.
use crate::NativeApp;
use gpui_kit::component::{
    ActiveTheme, Selectable, Sizable, ThemeConfig, ThemeSet,
    button::{Button, ButtonVariants},
    input::{Textarea, TextareaState},
};
use gpui_kit::{App, Context, Entity, Global, Window, div, prelude::*};
use serde::{Deserialize, Serialize};
use std::{
    io::{self, Read},
    path::{Path, PathBuf},
    rc::Rc,
};

const MAX_BYTES: usize = 256 * 1024;
const MAX_THEMES: usize = 64;

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct Library {
    pub themes: Vec<ThemeConfig>,
    pub light: Option<String>,
    pub dark: Option<String>,
    #[serde(skip)]
    load_error: Option<String>,
}
impl Global for Library {}
impl Library {
    fn import(&mut self, source: &str, replacing: Option<&str>) -> Result<(), String> {
        if source.len() > MAX_BYTES {
            return Err("Theme file exceeds 256 KiB".into());
        }
        let value: serde_json::Value =
            serde_json::from_str(source).map_err(|e| format!("Invalid theme JSON: {e}"))?;
        if !value.get("themes").is_some_and(serde_json::Value::is_array) {
            return Err("Expected a GPUI Kit theme collection with a themes array. Web theme files are not yet supported.".into());
        }
        let set: ThemeSet =
            serde_json::from_value(value.clone()).map_err(|e| format!("Invalid Kit theme: {e}"))?;
        if set.themes.is_empty() || set.themes.len() > MAX_THEMES {
            return Err("Import 1–64 variants at a time".into());
        }
        if replacing
            .is_some_and(|name| !self.themes.iter().any(|theme| theme.name.as_ref() == name))
        {
            return Err("The edited variant was removed; duplicate or import it instead".into());
        }
        if replacing.is_some() && set.themes.len() != 1 {
            return Err("Edit one variant at a time".into());
        }
        let mut names = std::collections::HashSet::new();
        for (index, theme) in set.themes.iter().enumerate() {
            let name = theme.name.as_ref();
            if name.trim() != name || name.is_empty() || name.chars().count() > 64 {
                return Err(
                    "Variant names must contain 1–64 characters, without surrounding whitespace"
                        .into(),
                );
            }
            if !names.insert(name) || crate::theme::bundled_names().contains(&name) {
                return Err(format!("Reserved or duplicate variant name: {name}"));
            }
            if self
                .themes
                .iter()
                .any(|t| t.name == theme.name && Some(t.name.as_ref()) != replacing)
            {
                return Err(format!(
                    "{name} already exists; edit it or choose another name"
                ));
            }
            let colors = value["themes"][index]["colors"]
                .as_object()
                .ok_or("Every variant needs a colors object")?;
            if colors.values().all(serde_json::Value::is_null) {
                return Err("Every variant needs at least one semantic color".into());
            }
            // Kit normally falls back silently for malformed colors. Imports must not claim success for them.
            for (role, color) in colors {
                if color.is_null() {
                    continue;
                }
                let color = color
                    .as_str()
                    .ok_or_else(|| format!("{role} must be a color string"))?;
                if gpui_kit::component::try_parse_color(color).is_err() {
                    return Err(format!("Unsupported or invalid solid color for {role}"));
                }
                let accepted = serde_json::to_value(&theme.colors).map_err(|e| e.to_string())?;
                if accepted.get(role).is_none() {
                    return Err(format!("Unknown Kit color role: {role}"));
                }
            }
            if theme
                .font_size
                .is_some_and(|n| !n.is_finite() || !(10. ..=32.).contains(&n))
                || theme
                    .mono_font_size
                    .is_some_and(|n| !n.is_finite() || !(10. ..=32.).contains(&n))
                || theme.radius.is_some_and(|n| n > 24)
                || theme.radius_lg.is_some_and(|n| n > 24)
            {
                return Err("Font sizes must be 10–32 and radii 0–24".into());
            }
        }
        let mut next = self.clone();
        if let Some(old) = replacing {
            next.themes.retain(|t| t.name.as_ref() != old);
            let theme = &set.themes[0];
            // Renaming keeps the owned selection only for the same appearance.
            if next.light.as_deref() == Some(old) {
                next.light = (!theme.mode.is_dark()).then(|| theme.name.to_string());
            }
            if next.dark.as_deref() == Some(old) {
                next.dark = theme.mode.is_dark().then(|| theme.name.to_string());
            }
        }
        next.themes.extend(set.themes);
        if next.themes.len() > MAX_THEMES {
            return Err("The library is limited to 64 variants".into());
        }
        *self = next;
        Ok(())
    }
    fn remove(&mut self, name: &str) {
        self.themes.retain(|t| t.name.as_ref() != name);
        if self.light.as_deref() == Some(name) {
            self.light = None;
        }
        if self.dark.as_deref() == Some(name) {
            self.dark = None;
        }
    }
    pub fn selected(&self, dark: bool) -> Option<Rc<ThemeConfig>> {
        let name = if dark { &self.dark } else { &self.light };
        if let Some(name) = name
            .as_deref()
            .filter(|name| crate::theme::bundled_names().contains(name))
        {
            let theme = crate::theme::bundled_config(name);
            return (theme.mode.is_dark() == dark).then(|| Rc::new(theme));
        }
        self.themes
            .iter()
            .find(|t| Some(t.name.as_ref()) == name.as_deref() && t.mode.is_dark() == dark)
            .cloned()
            .map(Rc::new)
    }
    fn save(&self, path: &Path) -> io::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let bytes = serde_json::to_vec_pretty(self)?;
        if bytes.len() > MAX_BYTES {
            return Err(io::Error::other("Theme library exceeds 256 KiB"));
        }
        let temporary = path.with_extension("json.tmp");
        std::fs::write(&temporary, bytes)?;
        std::fs::rename(temporary, path)
    }
    fn load(path: &Path) -> Result<Self, String> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let text = read_file(path)?;
        let stored: Self = serde_json::from_str(&text).map_err(|e| e.to_string())?;
        let source = serde_json::to_string(&ThemeSet {
            themes: stored.themes,
            ..Default::default()
        })
        .map_err(|e| e.to_string())?;
        let mut valid = Self::default();
        if !serde_json::from_str::<ThemeSet>(&source)
            .map_err(|e| e.to_string())?
            .themes
            .is_empty()
        {
            valid.import(&source, None)?;
        }
        valid.light = stored.light;
        valid.dark = stored.dark;
        if valid.selected(false).is_none() {
            valid.light = None;
        }
        if valid.selected(true).is_none() {
            valid.dark = None;
        }
        Ok(valid)
    }
}
fn path() -> PathBuf {
    t3code_desktop_gpui::drafts::default_path().with_file_name("theme-library.json")
}
pub(crate) fn init(cx: &mut App) {
    let library = if cfg!(test) {
        Library::default()
    } else {
        Library::load(&path()).unwrap_or_else(|error| {
            eprintln!("desktop-gpui: could not load theme library (file preserved): {error}");
            Library {
                load_error: Some(error),
                ..Default::default()
            }
        })
    };
    cx.set_global(library);
}
fn current(cx: &App) -> Library {
    cx.try_global::<Library>().cloned().unwrap_or_default()
}
fn mutate(
    window: &mut Window,
    cx: &mut App,
    edit: impl FnOnce(&mut Library) -> Result<(), String>,
) -> Result<(), String> {
    let mut next = current(cx);
    if let Some(error) = &next.load_error {
        return Err(format!("Library file is unavailable (preserved): {error}"));
    }
    edit(&mut next)?;
    if !cfg!(test) {
        next.save(&path())
            .map_err(|e| format!("Could not save theme library: {e}"))?;
    }
    cx.set_global(next);
    crate::theme::apply(Some(window), cx);
    Ok(())
}
fn read_file(path: &Path) -> Result<String, String> {
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .map_err(|e| e.to_string())?
        .take((MAX_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > MAX_BYTES {
        return Err("Theme file exceeds 256 KiB".into());
    }
    String::from_utf8(bytes).map_err(|_| "Theme file must be UTF-8".into())
}
fn source(theme: ThemeConfig) -> String {
    serde_json::to_string_pretty(&ThemeSet {
        name: theme.name.clone(),
        themes: vec![theme],
        ..Default::default()
    })
    .expect("serializable theme")
}

pub(crate) struct Editor {
    pub(crate) input: Entity<TextareaState>,
    pub(crate) open: bool,
    replacing: Option<String>,
    pub(crate) error: Option<String>,
    previous_focus: Option<gpui_kit::FocusHandle>,
    remove: Option<String>,
}
impl Editor {
    pub fn new(window: &mut Window, cx: &mut App) -> Self {
        Self {
            input: cx.new(|cx| TextareaState::new(window, cx).auto_grow(6, 12)),
            open: false,
            replacing: None,
            error: None,
            previous_focus: None,
            remove: None,
        }
    }
}
impl NativeApp {
    pub(crate) fn edit_theme(
        &mut self,
        theme: Option<ThemeConfig>,
        duplicate: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let mut theme = theme.unwrap_or_else(|| crate::theme::bundled_config("One Dark"));
        self.theme_editor.replacing = (!duplicate).then(|| theme.name.to_string());
        if duplicate {
            let library = current(cx);
            let base = theme.name.to_string();
            let mut number = 1;
            loop {
                let name = format!("{base} copy {number}");
                if !library.themes.iter().any(|t| t.name.as_ref() == name) {
                    theme.name = name.into();
                    break;
                }
                number += 1;
            }
        }
        self.theme_editor.previous_focus = window.focused(cx);
        self.theme_editor.input.update(cx, |input, cx| {
            input.set_value(source(theme), window, cx);
            input.focus(window, cx);
        });
        self.theme_editor.open = true;
        self.theme_editor.error = None;
        cx.notify();
    }
    fn close_theme_editor(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.theme_editor.open = false;
        self.theme_editor.error = None;
        if let Some(focus) = self.theme_editor.previous_focus.take() {
            focus.focus(window, cx);
        }
        cx.notify();
    }
    fn save_theme_editor(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.theme_editor.input.read(cx).value().to_string();
        let replacing = self.theme_editor.replacing.clone();
        match mutate(window, cx, |library| {
            library.import(&text, replacing.as_deref())
        }) {
            Ok(()) => {
                self.close_theme_editor(window, cx);
                self.report_success("Theme saved", cx);
            }
            Err(error) => {
                self.theme_editor.error = Some(error);
                cx.notify();
            }
        }
    }
    fn import_theme_file(&mut self, cx: &mut Context<Self>) {
        let picker = cx.prompt_for_paths(gpui_kit::PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("Import Kit theme JSON".into()),
        });
        cx.spawn(async move |entity, cx| {
            let paths = match picker.await {
                Ok(Ok(Some(paths))) => paths,
                Ok(Ok(None)) => return,
                _ => {
                    let _ = entity.update(cx, |app, cx| {
                        app.report_error("Could not open theme picker", cx)
                    });
                    return;
                }
            };
            let Some(path) = paths.into_iter().next() else {
                return;
            };
            let result = cx
                .background_executor()
                .spawn(async move { read_file(&path) })
                .await;
            let _ = entity.update_in(cx, |app, window, cx| {
                match result
                    .and_then(|text| mutate(window, cx, |library| library.import(&text, None)))
                {
                    Ok(()) => {
                        app.report_success("Themes imported; select a variant to apply it", cx)
                    }
                    Err(error) => app.report_error(error, cx),
                }
            });
        })
        .detach();
    }
    fn export_theme_file(&mut self, theme: ThemeConfig, cx: &mut Context<Self>) {
        let text = source(theme);
        let directory = std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| ".".into());
        let picker = cx.prompt_for_new_path(&directory, Some("theme.json"));
        cx.spawn(async move |entity, cx| {
            let path = match picker.await {
                Ok(Ok(Some(path))) => path,
                Ok(Ok(None)) => return,
                _ => {
                    let _ = entity.update(cx, |app, cx| {
                        app.report_error("Could not open theme save dialog", cx)
                    });
                    return;
                }
            };
            let result = cx
                .background_executor()
                .spawn(async move { std::fs::write(path, text) })
                .await;
            let _ = entity.update(cx, |app, cx| match result {
                Ok(()) => app.report_success("Theme exported", cx),
                Err(error) => app.report_error(format!("Could not export theme: {error}"), cx),
            });
        })
        .detach();
    }
    pub(crate) fn render_theme_library(&self, cx: &mut Context<Self>) -> gpui_kit::Div {
        let library = current(cx);
        if let Some(error) = &library.load_error {
            return div().text_sm().text_color(cx.theme().danger).child(format!("Theme library could not be read. Its file has been preserved; repair or move theme-library.json before restarting. {error}"));
        }
        let rows = crate::theme::bundled_names()
            .iter()
            .map(|name| (crate::theme::bundled_config(name), false))
            .chain(library.themes.iter().cloned().map(|theme| (theme, true)))
            .enumerate()
            .map(|(index, (theme, custom))| {
                let name = theme.name.to_string();
                let dark = theme.mode.is_dark();
                let selected_name = if dark {
                    library.dark.as_deref()
                } else {
                    library.light.as_deref()
                };
                let contrast = crate::settings_appearance::current(cx).variant
                    == crate::settings_appearance::Variant::HighContrast;
                let selected = selected_name == Some(&name)
                    || (!custom
                        && selected_name.is_none()
                        && name.starts_with("High Contrast") == contrast);
                let use_name = name.clone();
                let edit = theme.clone();
                let duplicate = theme.clone();
                let export = theme.clone();
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(div().text_sm().child(format!(
                        "{} · {}{}",
                        name,
                        if dark { "dark" } else { "light" },
                        if selected { " · active" } else { "" }
                    )))
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .gap_1()
                            .child(
                                div()
                                    .debug_selector(move || format!("theme-use-{index}"))
                                    .child(
                                        Button::new(("theme-use", index))
                                            .label("Use")
                                            .selected(selected)
                                            .ghost()
                                            .small()
                                            .on_click(cx.listener(move |app, _, window, cx| {
                                                let result = mutate(window, cx, |library| {
                                                    let selection = if dark {
                                                        &mut library.dark
                                                    } else {
                                                        &mut library.light
                                                    };
                                                    *selection = Some(use_name.clone());
                                                    Ok(())
                                                });
                                                if let Err(error) = result {
                                                    app.report_error(error, cx);
                                                }
                                                cx.notify();
                                            })),
                                    ),
                            )
                            .child(
                                Button::new(("theme-duplicate", index))
                                    .label("Duplicate…")
                                    .ghost()
                                    .small()
                                    .on_click(cx.listener(move |app, _, window, cx| {
                                        app.edit_theme(Some(duplicate.clone()), true, window, cx)
                                    })),
                            )
                            .when(custom, |row| {
                                row.child(
                                    Button::new(("theme-edit", index))
                                        .debug_selector(move || format!("theme-edit-{index}"))
                                        .label("Edit JSON…")
                                        .ghost()
                                        .small()
                                        .on_click(cx.listener(move |app, _, window, cx| {
                                            app.edit_theme(Some(edit.clone()), false, window, cx)
                                        })),
                                )
                            })
                            .child(
                                Button::new(("theme-export", index))
                                    .debug_selector(move || format!("theme-export-{index}"))
                                    .label("Export…")
                                    .ghost()
                                    .small()
                                    .on_click(cx.listener(move |app, _, _, cx| {
                                        app.export_theme_file(export.clone(), cx)
                                    })),
                            )
                            .when(custom, |row| {
                                row.child(
                                    Button::new(("theme-remove", index))
                                        .debug_selector(move || format!("theme-remove-{index}"))
                                        .label("Remove")
                                        .danger()
                                        .small()
                                        .on_click(cx.listener(move |app, _, _, cx| {
                                            app.theme_editor.remove = Some(name.clone());
                                            cx.notify();
                                        })),
                                )
                            }),
                    )
            });
        div().flex().flex_col().gap_2()
            .child(div().text_sm().child("Theme library · device-local"))
            .child(div().text_xs().text_color(cx.theme().muted_foreground).child("Solid-color Kit JSON collections. Light and dark selections follow appearance mode independently. Web theme files are not yet supported."))
            .child(div().flex().gap_1()
                .child(Button::new("theme-import").debug_selector(|| "theme-import".into()).label("Import file…").ghost().small().on_click(cx.listener(|app, _, _, cx| app.import_theme_file(cx))))
                .child(div().debug_selector(|| "theme-create".into()).child(Button::new("theme-create").label("Create from One Dark…").ghost().small().on_click(cx.listener(|app, _, window, cx| app.edit_theme(None, true, window, cx)))))
                .child(Button::new("theme-reset").debug_selector(|| "theme-reset".into()).label("Reset selections").ghost().small().on_click(cx.listener(|app, _, window, cx| { if let Err(error) = mutate(window, cx, |library| { library.light = None; library.dark = None; Ok(()) }) { app.report_error(error, cx); } cx.notify(); }))))
            .when(self.theme_editor.open, |view| view.child(div().flex().flex_col().gap_2()
                .child(div().text_sm().child("Edit Kit theme JSON · name, mode, semantic colors"))
                .child(Textarea::new(&self.theme_editor.input).font_family(cx.theme().mono_font_family.clone()))
                .when_some(self.theme_editor.error.clone(), |view, error| view.child(div().text_sm().text_color(cx.theme().danger).child(error)))
                .child(div().flex().gap_1()
                    .child(div().debug_selector(|| "theme-save".into()).child(Button::new("theme-save").label("Save theme").primary().small().on_click(cx.listener(|app, _, window, cx| app.save_theme_editor(window, cx)))))
                    .child(Button::new("theme-cancel").label("Cancel").ghost().small().on_click(cx.listener(|app, _, window, cx| app.close_theme_editor(window, cx)))))))
            .when_some(self.theme_editor.remove.clone(), |view, name| view.child(div().flex().flex_col().gap_1()
                .child(div().text_sm().child(format!("Remove {name}? Its active appearance falls back to your default family. Export first if you want to keep a copy.")))
                .child(div().flex().gap_1()
                    .child(Button::new("theme-remove-confirm").debug_selector(|| "theme-remove-confirm".into()).label("Remove theme").danger().small().on_click(cx.listener(move |app, _, window, cx| {
                        match mutate(window, cx, |library| { library.remove(&name); Ok(()) }) {
                            Ok(()) => app.theme_editor.remove = None,
                            Err(error) => app.report_error(error, cx),
                        }
                        cx.notify();
                    })))
                    .child(Button::new("theme-remove-cancel").debug_selector(|| "theme-remove-cancel".into()).label("Cancel").ghost().small().on_click(cx.listener(|app, _, _, cx| { app.theme_editor.remove = None; cx.notify(); }))))))
            .children(rows)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn sample(name: &str) -> String {
        format!(
            r##"{{"themes":[{{"name":"{name}","mode":"dark","colors":{{"background":"#121212","foreground":"#ffffff"}}}}]}}"##
        )
    }
    #[test]
    fn r13_f_theme_library_roundtrip_rename_remove_and_failure() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("library.json");
        let mut library = Library::default();
        library.import(&sample("Night"), None).unwrap();
        library.dark = Some("Night".into());
        library.save(&path).unwrap();
        let mut restored = Library::load(&path).unwrap();
        assert_eq!(restored.selected(true).unwrap().name.as_ref(), "Night");
        assert!(restored.selected(false).is_none());
        assert!(restored.import(&sample("Night"), None).is_err());
        assert!(
            restored
                .import(
                    &sample("Bad").replace("#121212", "not-a-color"),
                    Some("Night")
                )
                .is_err()
        );
        assert_eq!(restored.selected(true).unwrap().name.as_ref(), "Night");
        restored.import(&sample("Renamed"), Some("Night")).unwrap();
        assert_eq!(restored.selected(true).unwrap().name.as_ref(), "Renamed");
        restored.remove("Renamed");
        assert!(restored.selected(true).is_none());
        assert!(restored.save(dir.path()).is_err());
    }
    #[test]
    fn r13_f_theme_import_rejects_reserved_unknown_and_oversized_sources() {
        let mut library = Library::default();
        for text in [
            sample("One Dark"),
            sample("Bad").replace("background", "misspelled"),
            "{}".into(),
            " ".repeat(MAX_BYTES + 1),
        ] {
            assert!(library.import(&text, None).is_err());
        }
        assert!(library.themes.is_empty());
    }
}
