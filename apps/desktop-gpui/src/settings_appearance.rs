//! Device-local appearance and scale, independent of the selected environment.
use gpui_kit::{App, Global};
use serde::{Deserialize, Serialize};
use std::{fs, io, path::Path};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Appearance {
    Light,
    #[default]
    Dark,
    System,
}
impl Appearance {
    pub const ALL: [Self; 3] = [Self::Light, Self::Dark, Self::System];
    pub fn label(self) -> &'static str {
        match self {
            Self::Light => "Light",
            Self::Dark => "Dark",
            Self::System => "System",
        }
    }
    pub fn dark(self, system_dark: bool) -> bool {
        match self {
            Self::Light => false,
            Self::Dark => true,
            Self::System => system_dark,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Variant {
    #[default]
    ZedOne,
    HighContrast,
}
impl Variant {
    pub const ALL: [Self; 2] = [Self::ZedOne, Self::HighContrast];
    pub fn label(self) -> &'static str {
        match self {
            Self::ZedOne => "Zed One",
            Self::HighContrast => "High contrast",
        }
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum TimestampFormat {
    #[default]
    #[serde(rename = "locale")]
    Locale,
    #[serde(rename = "12-hour")]
    TwelveHour,
    #[serde(rename = "24-hour")]
    TwentyFourHour,
}
impl TimestampFormat {
    pub const ALL: [Self; 3] = [Self::Locale, Self::TwelveHour, Self::TwentyFourHour];
    pub fn label(self) -> &'static str {
        match self {
            Self::Locale => "System locale",
            Self::TwelveHour => "12-hour",
            Self::TwentyFourHour => "24-hour",
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Preferences {
    pub appearance: Appearance,
    pub font_size: f32,
    pub zoom: i8,
    pub variant: Variant,
    pub prose_size: f32,
    pub code_size: f32,
    pub confirm_archive: bool,
    pub confirm_delete: bool,
    pub confirm_unpin: bool,
    pub timestamp_format: TimestampFormat,
}
impl Default for Preferences {
    fn default() -> Self {
        Self {
            appearance: Appearance::Dark,
            font_size: 14.,
            zoom: 0,
            variant: Variant::ZedOne,
            prose_size: 14.,
            code_size: 13.,
            confirm_archive: false,
            confirm_delete: true,
            confirm_unpin: false,
            timestamp_format: TimestampFormat::Locale,
        }
    }
}
impl Global for Preferences {}
impl Preferences {
    pub fn normalized(mut self) -> Self {
        self.font_size = if self.font_size.is_finite() {
            self.font_size.clamp(10., 24.)
        } else {
            14.
        };
        self.zoom = self.zoom.clamp(-4, 8);
        self.prose_size = bounded_font(self.prose_size, 14.);
        self.code_size = bounded_font(self.code_size, 13.);
        self
    }
    pub fn effective_size(self) -> f32 {
        self.font_size * 1.1_f32.powi(self.zoom.into())
    }
    pub fn load(path: &Path) -> Self {
        fs::read(path)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Self>(&bytes).ok())
            .unwrap_or_default()
            .normalized()
    }
    pub fn save(self, path: &Path) -> io::Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let temporary = path.with_extension("json.tmp");
        fs::write(&temporary, serde_json::to_vec_pretty(&self.normalized())?)?;
        fs::rename(temporary, path)
    }
}
fn bounded_font(value: f32, default: f32) -> f32 {
    if value.is_finite() {
        value.clamp(10., 32.)
    } else {
        default
    }
}
pub fn current(cx: &App) -> Preferences {
    cx.try_global::<Preferences>().copied().unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn appearance_and_zoom_restart_without_environment_state() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("appearance.json");
        let preferences = Preferences {
            appearance: Appearance::System,
            font_size: 18.,
            zoom: 2,
            ..Preferences::default()
        };
        preferences.save(&path).unwrap();
        let restored = Preferences::load(&path);
        assert_eq!(restored.appearance, Appearance::System);
        assert_eq!(restored.effective_size(), preferences.effective_size());
        assert!(!restored.appearance.dark(false));
        assert!(restored.appearance.dark(true));
        assert!(Appearance::Dark.dark(false));
        assert!(!Appearance::Light.dark(true));
    }
    #[test]
    fn malformed_preferences_and_scale_are_bounded() {
        let preferences = Preferences {
            font_size: f32::NAN,
            zoom: 127,
            ..Preferences::default()
        }
        .normalized();
        assert_eq!(preferences.font_size, 14.);
        assert_eq!(preferences.zoom, 8);
        assert_eq!(
            Preferences {
                font_size: 0.,
                zoom: -127,
                ..preferences
            }
            .normalized()
            .zoom,
            -4
        );
    }
}
