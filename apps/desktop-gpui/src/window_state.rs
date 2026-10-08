//! Restart-safe window size, layout and view preferences, stored beside the native drafts.
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

pub const SIDEBAR_WIDTH: (f32, f32, f32) = (208.0, 288.0, 480.0);
pub const RIGHT_PANEL_WIDTH: (f32, f32, f32) = (280.0, 340.0, 640.0);
pub const TERMINAL_HEIGHT: (f32, f32, f32) = (180.0, 260.0, 900.0);

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct WindowState {
    pub width: f32,
    pub height: f32,
    pub x: Option<f32>,
    pub y: Option<f32>,
    pub maximized: bool,
    pub context_meter_hidden: bool,
    pub notifications: crate::notifications::Preferences,
    pub sidebar_open: bool,
    pub sidebar_width: f32,
    pub right_panel_open: bool,
    pub right_panel_width: f32,
    pub terminal_height: f32,
    pub follow_up_behavior: crate::queue::FollowUpBehavior,
}

impl Default for WindowState {
    fn default() -> Self {
        Self {
            width: 0.0,
            height: 0.0,
            x: None,
            y: None,
            maximized: false,
            context_meter_hidden: false,
            notifications: crate::notifications::Preferences::default(),
            sidebar_open: true,
            sidebar_width: SIDEBAR_WIDTH.1,
            right_panel_open: true,
            right_panel_width: RIGHT_PANEL_WIDTH.1,
            terminal_height: TERMINAL_HEIGHT.1,
            follow_up_behavior: crate::queue::FollowUpBehavior::default(),
        }
    }
}

impl WindowState {
    /// Recover a fully visible normal rectangle against current displays. A
    /// disconnected display or corrupt position falls back to the primary;
    /// legacy size-only state is centred there. Never restore off-screen chrome.
    pub fn restore_rect(&self, displays: &[[f32; 4]]) -> Option<[f32; 4]> {
        let [dx, dy, dw, dh] = *displays.first()?;
        let (width, height) = self.size().unwrap_or((1280.0, 900.0));
        let rect = match (self.x, self.y) {
            (Some(x), Some(y)) if x.is_finite() && y.is_finite() => [x, y, width, height],
            _ => [
                dx + (dw - width) / 2.0,
                dy + (dh - height) / 2.0,
                width,
                height,
            ],
        };
        clamp_rect(rect, displays)
    }

    /// The saved size, when it is large enough to restore.
    pub fn size(&self) -> Option<(f32, f32)> {
        (self.width.is_finite()
            && self.height.is_finite()
            && self.width >= 480.0
            && self.height >= 360.0)
            .then_some((self.width, self.height))
    }
}

/// Fit a rectangle to the work area with the greatest intersection. Displays
/// are in one coordinate space, primary first. Disconnected windows recenter on
/// the primary; partially visible ones retain their target and move minimally.
/// No minimum size may force chrome under a Dock on a small display.
pub fn clamp_rect(rect: [f32; 4], work_areas: &[[f32; 4]]) -> Option<[f32; 4]> {
    let valid = |r: &&[f32; 4]| r.iter().all(|v| v.is_finite()) && r[2] > 0. && r[3] > 0.;
    let primary = *work_areas.iter().find(valid)?;
    let [x, y, w, h] = rect;
    if !rect.iter().all(|v| v.is_finite()) || w <= 0. || h <= 0. {
        return None;
    }
    let mut target = primary;
    let mut overlap = 0.;
    for area in work_areas.iter().filter(valid) {
        let [dx, dy, dw, dh] = *area;
        let intersection =
            ((x + w).min(dx + dw) - x.max(dx)).max(0.) * ((y + h).min(dy + dh) - y.max(dy)).max(0.);
        if intersection > overlap {
            overlap = intersection;
            target = *area;
        }
    }
    let [dx, dy, dw, dh] = target;
    let width = w.min(dw);
    let height = h.min(dh);
    Some(if overlap == 0. {
        [
            dx + (dw - width) / 2.,
            dy + (dh - height) / 2.,
            width,
            height,
        ]
    } else {
        [
            x.clamp(dx, dx + dw - width),
            y.clamp(dy, dy + dh - height),
            width,
            height,
        ]
    })
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
    state.terminal_height = clamp_width(state.terminal_height, TERMINAL_HEIGHT);
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
    fn terminal_height_restart_legacy_and_clamped_restore() {
        assert_eq!(parse("{}").terminal_height, TERMINAL_HEIGHT.1);
        assert_eq!(
            parse(r#"{"terminal_height":1200}"#).terminal_height,
            TERMINAL_HEIGHT.2
        );
        assert_eq!(
            parse(r#"{"terminal_height":10}"#).terminal_height,
            TERMINAL_HEIGHT.0
        );
        assert_eq!(
            parse(r#"{"terminal_height":-1}"#).terminal_height,
            TERMINAL_HEIGHT.1
        );
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("window.json");
        let state = WindowState {
            terminal_height: 420.0,
            ..WindowState::default()
        };
        save_to(&path, state).unwrap();
        assert_eq!(
            parse(&std::fs::read_to_string(path).unwrap()).terminal_height,
            420.0
        );
    }

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
    fn r13_a_window_position_recovers_disconnected_display_and_legacy_size() {
        let displays = [[0., 0., 1440., 900.], [-1920., 0., 1920., 1080.]];
        let state = WindowState {
            x: Some(-1800.),
            y: Some(40.),
            width: 1280.,
            height: 800.,
            maximized: true,
            ..WindowState::default()
        };
        assert_eq!(
            state.restore_rect(&displays),
            Some([-1800., 40., 1280., 800.])
        );
        assert_eq!(
            state.restore_rect(&displays[..1]),
            Some([80., 50., 1280., 800.])
        );
        assert_eq!(parse(&serde_json::to_string(&state).unwrap()), state);
        assert_eq!(
            parse(r#"{"width":1280,"height":800}"#).restore_rect(&displays),
            Some([80., 50., 1280., 800.])
        );
        let corrupt = WindowState {
            x: Some(f32::NAN),
            width: f32::INFINITY,
            ..state
        };
        assert_eq!(
            corrupt.restore_rect(&displays),
            Some([80., 0., 1280., 900.])
        );
        assert_eq!(
            state.restore_rect(&[[0., 0., 800., 600.]]),
            Some([0., 0., 800., 600.])
        );
        assert_eq!(state.restore_rect(&[]), None);
    }

    #[test]
    fn r16_win_dock_bottom_clamps_diagnosed_outer_frame() {
        let work = [[0., 25., 1920., 975.]];
        assert_eq!(
            clamp_rect([320., 49., 1280., 983.], &work),
            Some([320., 25., 1280., 975.])
        );
        assert_eq!(
            clamp_rect([320., 49., 1280., 900.], &work),
            Some([320., 49., 1280., 900.])
        );
    }

    #[test]
    fn r16_win_left_right_and_auto_hidden_docks() {
        let rect = [0., 0., 1920., 1080.];
        for area in [
            [80., 25., 1840., 1055.],
            [0., 25., 1840., 1055.],
            [0., 25., 1920., 1054.],
        ] {
            assert_eq!(clamp_rect(rect, &[area]), Some(area));
            assert_eq!(clamp_rect(area, &[area]), Some(area));
        }
    }

    #[test]
    fn r16_win_secondary_display_retained_when_dock_overlaps_saved_rect() {
        let work = [[0., 25., 1920., 975.], [-1600., -175., 1520., 875.]];
        assert_eq!(
            clamp_rect([-1500., -150., 1500., 900.], &work),
            Some([-1580., -175., 1500., 875.])
        );
        assert_eq!(
            clamp_rect([-1500., -150., 1500., 900.], &work[..1]),
            Some([210., 62.5, 1500., 900.])
        );
        // Displays above/right of the primary use the same global coordinates.
        assert_eq!(
            clamp_rect(
                [2050., -1000., 1000., 800.],
                &[[0., 25., 1920., 975.], [1920., -1055., 1920., 975.]]
            ),
            Some([2050., -1000., 1000., 800.])
        );
    }

    #[test]
    fn r16_win_straddling_window_uses_greatest_intersection() {
        let areas = [[0., 25., 1000., 800.], [1000., 25., 1000., 800.]];
        assert_eq!(
            clamp_rect([900., 100., 800., 600.], &areas),
            Some([1000., 100., 800., 600.])
        );
        assert_eq!(
            clamp_rect([400., 100., 800., 600.], &areas),
            Some([200., 100., 800., 600.])
        );
    }

    #[test]
    fn r16_win_visible_frame_changes_preserve_in_bounds_user_resizes() {
        let large = [[0., 25., 1920., 1055.]];
        let dock = [[0., 25., 1920., 900.]];
        let user = [320., 100., 900., 700.];
        for areas in [large, dock, large] {
            assert_eq!(clamp_rect(user, &areas), Some(user));
        }
        let full = clamp_rect([0., 25., 1920., 1055.], &dock).unwrap();
        assert_eq!(full, dock[0]);
        assert_eq!(clamp_rect(full, &large), Some(full));
    }

    #[test]
    fn r16_win_invalid_displays_and_tiny_work_area() {
        assert_eq!(
            clamp_rect(
                [0., 0., 1280., 900.],
                &[
                    [0., 0., 0., 900.],
                    [f32::NAN, 0., 1920., 1080.],
                    [10., 20., 300., 200.]
                ]
            ),
            Some([10., 20., 300., 200.])
        );
        assert_eq!(clamp_rect([0., 0., 1280., 900.], &[]), None);
        assert_eq!(
            clamp_rect([f32::NAN, 0., 1280., 900.], &[[0., 0., 1920., 1080.]]),
            None
        );
    }

    #[test]
    fn r16_win_normal_and_maximized_work_area_restore_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("window.json");
        for maximized in [false, true] {
            let state = WindowState {
                x: Some(-1500.),
                y: Some(49.),
                width: 1280.,
                height: 983.,
                maximized,
                sidebar_open: false,
                ..WindowState::default()
            };
            save_to(&path, state).unwrap();
            let restarted = parse(&std::fs::read_to_string(&path).unwrap());
            assert_eq!(restarted, state);
            let areas = [[0., 25., 1920., 975.], [-1920., 25., 1920., 875.]];
            assert_eq!(
                restarted.restore_rect(&areas),
                Some([-1500., 25., 1280., 875.])
            );
            assert_eq!(
                restarted.restore_rect(&areas[..1]),
                Some([320., 25., 1280., 975.])
            );
        }
        assert_eq!(
            WindowState::default().restore_rect(&[[80., 25., 1840., 975.]]),
            Some([360., 62.5, 1280., 900.])
        );
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
