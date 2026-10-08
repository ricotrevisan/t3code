//! Restart-safe window size, layout and view preferences, stored beside the native drafts.
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

pub const SIDEBAR_WIDTH: (f32, f32, f32) = (208.0, 288.0, 480.0);
pub const RIGHT_PANEL_WIDTH: (f32, f32, f32) = (280.0, 340.0, 640.0);

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct WindowState {
    pub width: f32,
    pub height: f32,
    pub context_meter_hidden: bool,
    pub notifications: crate::notifications::Preferences,
    pub sidebar_open: bool,
    pub sidebar_width: f32,
    pub right_panel_open: bool,
    pub right_panel_width: f32,
    pub follow_up_behavior: crate::queue::FollowUpBehavior,
}

impl Default for WindowState {
    fn default() -> Self {
        Self {
            width: 0.0,
            height: 0.0,
            context_meter_hidden: false,
            notifications: crate::notifications::Preferences::default(),
            sidebar_open: true,
            sidebar_width: SIDEBAR_WIDTH.1,
            right_panel_open: true,
            right_panel_width: RIGHT_PANEL_WIDTH.1,
            follow_up_behavior: crate::queue::FollowUpBehavior::default(),
        }
    }
}

impl WindowState {
    /// The saved size, when it is large enough to restore.
    pub fn size(&self) -> Option<(f32, f32)> {
        (self.width >= 480.0 && self.height >= 360.0).then_some((self.width, self.height))
    }
}

/// `width` held to a `(min, default, max)` range; unusable values fall back to the default.
pub fn clamp_width(width: f32, (min, default, max): (f32, f32, f32)) -> f32 {
    if width.is_finite() && width > 0.0 {
        width.clamp(min, max)
    } else {
        default
    }
}

fn path() -> PathBuf {
    crate::drafts::default_path().with_file_name("window.json")
}

fn parse(text: &str) -> WindowState {
    let mut state: WindowState = serde_json::from_str(text).unwrap_or_default();
    state.sidebar_width = clamp_width(state.sidebar_width, SIDEBAR_WIDTH);
    state.right_panel_width = clamp_width(state.right_panel_width, RIGHT_PANEL_WIDTH);
    state
}

pub fn load() -> WindowState {
    std::fs::read_to_string(path())
        .map(|text| parse(&text))
        .unwrap_or_default()
}

fn save_to(path: &std::path::Path, state: WindowState) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let text = serde_json::to_string(&state)?;
    let pending = path.with_extension("json.pending");
    std::fs::write(&pending, text)?;
    std::fs::rename(pending, path)
}

pub fn save(state: WindowState) {
    let _ = save_to(&path(), state);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn follow_up_behavior_restart_and_legacy_default() {
        use crate::queue::FollowUpBehavior;
        assert_eq!(parse("{}").follow_up_behavior, FollowUpBehavior::Queue);
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("device/window.json");
        for behavior in [FollowUpBehavior::Queue, FollowUpBehavior::Steer] {
            save_to(
                &path,
                WindowState {
                    follow_up_behavior: behavior,
                    ..WindowState::default()
                },
            )
            .unwrap();
            assert_eq!(
                parse(&std::fs::read_to_string(&path).unwrap()).follow_up_behavior,
                behavior
            );
        }
    }

    #[test]
    fn notification_preferences_restart_and_legacy_defaults() {
        use crate::notifications::{NotificationMode, Preferences};
        assert_eq!(parse("{}").notifications, Preferences::default());
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("device/window.json");
        for mode in NotificationMode::ALL {
            for in_app in [false, true] {
                let state = WindowState {
                    notifications: Preferences { mode, in_app },
                    ..WindowState::default()
                };
                save_to(&path, state).unwrap();
                let restarted = parse(&std::fs::read_to_string(&path).unwrap());
                assert_eq!(restarted.notifications, state.notifications);
            }
        }
    }

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

    #[test]
    fn layout_round_trips_and_older_files_open_both_panels_at_default_widths() {
        let state = WindowState {
            sidebar_open: false,
            sidebar_width: 360.0,
            right_panel_open: false,
            right_panel_width: 420.0,
            ..WindowState::default()
        };
        assert_eq!(parse(&serde_json::to_string(&state).unwrap()), state);

        let old = parse(r#"{"width":1280.0,"height":900.0}"#);
        assert_eq!(old.size(), Some((1280.0, 900.0)));
        assert!(old.sidebar_open && old.right_panel_open);
        assert_eq!(old.sidebar_width, SIDEBAR_WIDTH.1);
        assert_eq!(old.right_panel_width, RIGHT_PANEL_WIDTH.1);

        let wild = parse(r#"{"sidebar_width":5000.0,"right_panel_width":-1.0}"#);
        assert_eq!(wild.sidebar_width, SIDEBAR_WIDTH.2);
        assert_eq!(wild.right_panel_width, RIGHT_PANEL_WIDTH.1);
        assert_eq!(parse(r#"{"sidebar_open":"yes"}"#), WindowState::default());
    }
}
