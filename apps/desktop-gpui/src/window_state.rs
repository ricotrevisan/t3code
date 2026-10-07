//! Restart-safe window size and view preferences, stored beside the native drafts.
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct WindowState {
    pub width: f32,
    pub height: f32,
    pub context_meter_hidden: bool,
}

impl WindowState {
    /// The saved size, when it is large enough to restore.
    pub fn size(&self) -> Option<(f32, f32)> {
        (self.width >= 480.0 && self.height >= 360.0).then_some((self.width, self.height))
    }
}

fn path() -> PathBuf {
    crate::drafts::default_path().with_file_name("window.json")
}

fn parse(text: &str) -> WindowState {
    serde_json::from_str(text).unwrap_or_default()
}

pub fn load() -> WindowState {
    std::fs::read_to_string(path())
        .map(|text| parse(&text))
        .unwrap_or_default()
}

pub fn save(state: WindowState) {
    if let Ok(text) = serde_json::to_string(&state) {
        let _ = std::fs::write(path(), text);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_below_the_usable_minimum_are_rejected() {
        assert!(parse(r#"{"width":100.0,"height":100.0}"#).size().is_none());
        assert_eq!(parse("not json"), WindowState::default());
        assert_eq!(
            parse(r#"{"width":1280.0,"height":900.0}"#).size(),
            Some((1280.0, 900.0))
        );
    }

    #[test]
    fn view_preferences_survive_without_a_saved_size() {
        let state = parse(r#"{"context_meter_hidden":true}"#);
        assert!(state.context_meter_hidden);
        assert!(state.size().is_none());
        assert!(!parse(r#"{"width":1280.0,"height":900.0}"#).context_meter_hidden);
    }
}
