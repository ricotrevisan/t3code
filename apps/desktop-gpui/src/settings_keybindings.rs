//! NativeApp shortcuts are device-local; input/editor keymaps retain their own contexts.
use crate::NativeApp;
use gpui_kit::component::{
    Sizable,
    button::{Button, ButtonVariants},
};
use gpui_kit::{App, Context, Global, KeyBinding, KeystrokeEvent, Window, div, prelude::*};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, path::Path};

#[derive(Default, Clone, Serialize, Deserialize)]
struct Overrides(BTreeMap<String, String>);
#[derive(Default)]
struct State {
    defaults: Vec<KeyBinding>,
    overrides: Overrides,
    recording: Option<String>,
    strokes: Vec<String>,
    chord: bool,
    interceptor: Option<gpui_kit::Subscription>,
}
impl Global for State {}
fn path() -> std::path::PathBuf {
    t3code_desktop_gpui::drafts::default_path().with_file_name("keybindings.json")
}
impl Overrides {
    fn load(path: &Path) -> Self {
        std::fs::read(path)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default()
    }
    fn save(&self, path: &Path) -> Result<(), String> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let temporary = path.with_extension("json.tmp");
        std::fs::write(
            &temporary,
            serde_json::to_vec_pretty(self).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
        std::fs::rename(temporary, path).map_err(|e| e.to_string())
    }
}
fn validate_chord(shortcut: &str) -> Result<(), String> {
    let strokes = shortcut.split_whitespace().collect::<Vec<_>>();
    if strokes.is_empty() || strokes.len() > 4 {
        return Err("Use one to four shortcut strokes".into());
    }
    for stroke in strokes {
        gpui_kit::Keystroke::parse(stroke).map_err(|e| e.to_string())?;
    }
    Ok(())
}
fn chord_conflicts(a: &str, b: &str) -> bool {
    let a = a.split_whitespace().collect::<Vec<_>>();
    let b = b.split_whitespace().collect::<Vec<_>>();
    a.starts_with(&b) || b.starts_with(&a)
}
fn native(binding: &KeyBinding) -> bool {
    binding
        .predicate()
        .is_some_and(|p| p.to_string() == "NativeApp")
}
fn keys(binding: &KeyBinding) -> String {
    binding
        .keystrokes()
        .iter()
        .map(|stroke| stroke.unparse())
        .collect::<Vec<_>>()
        .join(" ")
}
pub(crate) fn init(cx: &mut App) {
    let defaults = cx
        .key_bindings()
        .borrow()
        .bindings()
        .filter(|b| native(b))
        .cloned()
        .collect();
    cx.set_global(State {
        defaults,
        overrides: if cfg!(test) {
            Overrides::default()
        } else {
            Overrides::load(&path())
        },
        recording: None,
        strokes: Vec::new(),
        chord: false,
        interceptor: None,
    });
    apply(cx);
}
fn apply(cx: &mut App) {
    let state = cx.global::<State>();
    let mut bindings = cx
        .key_bindings()
        .borrow()
        .bindings()
        .filter(|b| !native(b))
        .cloned()
        .collect::<Vec<_>>();
    let mut added = std::collections::HashSet::new();
    for original in &state.defaults {
        let name = original.action().name();
        if let Some(shortcut) = state.overrides.0.get(name) {
            if !added.insert(name) {
                continue;
            }
            match KeyBinding::load(
                shortcut,
                original.action().boxed_clone(),
                original.predicate(),
                false,
                None,
                &gpui_kit::DummyKeyboardMapper,
            ) {
                Ok(binding) => bindings.push(binding),
                Err(_) => bindings.push(original.clone()),
            }
        } else {
            bindings.push(original.clone());
        }
    }
    cx.clear_key_bindings();
    cx.bind_keys(bindings);
}
impl NativeApp {
    fn save_shortcut(
        &mut self,
        name: &str,
        shortcut: Option<String>,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        let state = cx.global::<State>();
        if !state.defaults.iter().any(|b| b.action().name() == name) {
            return Err("Unknown shortcut action".into());
        }
        let requested = if let Some(shortcut) = &shortcut {
            validate_chord(shortcut)?;
            vec![shortcut.clone()]
        } else {
            state
                .defaults
                .iter()
                .filter(|b| b.action().name() == name)
                .map(keys)
                .collect()
        };
        let conflict = cx
            .key_bindings()
            .borrow()
            .bindings()
            .find(|b| {
                requested.iter().any(|key| chord_conflicts(key, &keys(b)))
                    && b.action().name() != name
            })
            .map(|b| b.action().name().to_owned());
        if let Some(conflict) = conflict {
            return Err(format!(
                "Shortcut conflicts with {conflict}; choose a different shortcut"
            ));
        }
        let mut overrides = state.overrides.clone();
        if let Some(shortcut) = shortcut {
            overrides.0.insert(name.into(), shortcut);
        } else {
            overrides.0.remove(name);
        }
        if !cfg!(test) {
            overrides.save(&path())?;
        }
        cx.global_mut::<State>().overrides = overrides;
        apply(cx);
        Ok(())
    }
    pub(crate) fn record_shortcut(
        &mut self,
        event: &KeystrokeEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(_) = cx.global::<State>().recording.clone() else {
            return;
        };
        if !self.settings_open {
            cx.global_mut::<State>().recording = None;
            cx.global_mut::<State>().interceptor = None;
            return;
        }
        cx.stop_propagation();
        if ["shift", "control", "alt", "platform", "function"]
            .contains(&event.keystroke.key.as_str())
        {
            return;
        }
        let shortcut = event.keystroke.unparse();
        if shortcut == "escape" {
            cx.global_mut::<State>().recording = None;
        } else if !cx.global::<State>().chord {
            cx.global_mut::<State>().strokes = vec![shortcut];
            self.finish_shortcut(cx);
        } else if shortcut == "enter" {
            self.finish_shortcut(cx);
        } else if shortcut == "backspace" {
            cx.global_mut::<State>().strokes.pop();
        } else if cx.global::<State>().strokes.len() < 4 {
            cx.global_mut::<State>().strokes.push(shortcut);
        }
        if cx.global::<State>().recording.is_none() {
            cx.global_mut::<State>().interceptor = None;
        }
        cx.notify();
    }
    fn finish_shortcut(&mut self, cx: &mut Context<Self>) {
        let Some(name) = cx.global::<State>().recording.clone() else {
            return;
        };
        let shortcut = cx.global::<State>().strokes.join(" ");
        match self.save_shortcut(&name, Some(shortcut), cx) {
            Ok(()) => {
                cx.global_mut::<State>().recording = None;
                cx.global_mut::<State>().interceptor = None;
                self.report_success("Shortcut saved on this device", cx);
            }
            Err(error) => self.report_error(error, cx),
        }
    }
    pub(crate) fn render_keybinding_preferences(
        &self,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let state = cx.global::<State>();
        let recording = state.recording.clone();
        let mut names = std::collections::HashSet::new();
        let rows = state
            .defaults
            .iter()
            .filter(|binding| names.insert(binding.action().name()))
            .map(|binding| {
                let name = binding.action().name().to_owned();
                let shortcut = state
                    .overrides
                    .0
                    .get(&name)
                    .cloned()
                    .unwrap_or_else(|| keys(binding));
                (name, shortcut)
            })
            .collect::<Vec<_>>();
        div()
            .flex()
            .flex_col()
            .gap_2()
            .child("Device-local shortcuts · native app context. Escape cancels recording.")
            .when_some(recording, |view, name| {
                view.child(
                    div()
                        .text_sm()
                        .child(format!("{name}: {} · Enter saves, Backspace removes, Escape cancels (up to four strokes)", state.strokes.join(" ")))
                        .child(Button::new("shortcut-save-chord").label("Save chord").ghost().small()
                            .on_click(cx.listener(|app, _, _, cx| { app.finish_shortcut(cx); cx.notify(); }))),
                )
            })
            .children(
                rows.into_iter()
                    .enumerate()
                    .map(|(index, (name, shortcut))| {
                        let reset = name.clone();
                        let chord_name = name.clone();
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .gap_2()
                            .text_sm()
                            .child(div().flex().flex_col().child(name.clone()).child(shortcut).child("When: NativeApp"))
                            .child(
                                div()
                                    .flex()
                                    .gap_1()
                                    .child(
                                        Button::new(("shortcut-record", index))
                                            .debug_selector(move || {
                                                format!("shortcut-record-{index}")
                                            })
                                            .label("Record")
                                            .ghost()
                                            .small()
                                            .on_click(cx.listener(move |app, _, window, cx| {
                                                cx.global_mut::<State>().recording =
                                                    Some(name.clone());
                                                cx.global_mut::<State>().strokes.clear();
                                                cx.global_mut::<State>().chord = false;
                                                let entity = cx.entity().downgrade();
                                                let interceptor = cx.intercept_keystrokes(
                                                    move |event, window, cx| {
                                                        let _ = entity.update(cx, |app, cx| {
                                                            app.record_shortcut(event, window, cx)
                                                        });
                                                    },
                                                );
                                                cx.global_mut::<State>().interceptor =
                                                    Some(interceptor);
                                                app.settings_shortcut_focus.focus(window, cx);
                                                cx.notify();
                                            })),
                                    )
                                    .child(
                                        Button::new(("shortcut-chord", index)).debug_selector(move || format!("shortcut-chord-{index}"))
                                            .label("Chord").ghost().small()
                                            .on_click(cx.listener(move |app, _, window, cx| {
                                                cx.global_mut::<State>().recording = Some(chord_name.clone());
                                                cx.global_mut::<State>().strokes.clear();
                                                cx.global_mut::<State>().chord = true;
                                                let entity = cx.entity().downgrade();
                                                let interceptor = cx.intercept_keystrokes(move |event, window, cx| {
                                                    let _ = entity.update(cx, |app, cx| app.record_shortcut(event, window, cx));
                                                });
                                                cx.global_mut::<State>().interceptor = Some(interceptor);
                                                app.settings_shortcut_focus.focus(window, cx);
                                                cx.notify();
                                            })),
                                    )
                                    .child(
                                        Button::new(("shortcut-reset", index))
                                            .debug_selector(move || {
                                                format!("shortcut-reset-{index}")
                                            })
                                            .label("Reset")
                                            .ghost()
                                            .small()
                                            .on_click(cx.listener(move |app, _, _, cx| {
                                                if let Err(error) =
                                                    app.save_shortcut(&reset, None, cx)
                                                {
                                                    app.report_error(error, cx);
                                                }
                                                cx.global_mut::<State>().recording = None;
                                                cx.global_mut::<State>().interceptor = None;
                                                cx.notify();
                                            })),
                                    ),
                            )
                    }),
            )
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn r14_f_chord_prefix_conflicts_and_validation() {
        assert!(chord_conflicts("ctrl-k", "ctrl-k ctrl-c"));
        assert!(chord_conflicts("ctrl-k ctrl-c", "ctrl-k"));
        assert!(!chord_conflicts("ctrl-k ctrl-c", "ctrl-k ctrl-u"));
        assert!(!chord_conflicts("ctrl-k", "ctrl-j"));
        assert!(validate_chord("ctrl-k ctrl-c").is_ok());
        assert!(validate_chord("").is_err());
        assert!(validate_chord("a b c d e").is_err());
    }
    #[test]
    fn settings_fixc_shortcuts_restart_without_environment_settings() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("keys.json");
        let overrides = Overrides(BTreeMap::from([(
            "native::TogglePalette".into(),
            "ctrl-alt-p".into(),
        )]));
        overrides.save(&path).unwrap();
        assert_eq!(Overrides::load(&path).0, overrides.0);
    }
}
