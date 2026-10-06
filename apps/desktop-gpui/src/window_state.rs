//! Restart-safe window size, stored beside the native drafts.
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct WindowSize {
    pub width: f32,
    pub height: f32,
}

fn path() -> PathBuf {
    crate::drafts::default_path().with_file_name("window.json")
}

fn parse(text: &str) -> Option<WindowSize> {
    let size: WindowSize = serde_json::from_str(text).ok()?;
    (size.width >= 480.0 && size.height >= 360.0).then_some(size)
}

pub fn load() -> Option<WindowSize> {
    parse(&std::fs::read_to_string(path()).ok()?)
}

pub fn save(size: WindowSize) {
    if let Ok(text) = serde_json::to_string(&size) {
        let _ = std::fs::write(path(), text);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_below_the_usable_minimum_are_rejected() {
        assert!(parse(r#"{"width":100.0,"height":100.0}"#).is_none());
        assert!(parse("not json").is_none());
        assert_eq!(
            parse(r#"{"width":1280.0,"height":900.0}"#),
            Some(WindowSize {
                width: 1280.0,
                height: 900.0
            })
        );
    }
}
