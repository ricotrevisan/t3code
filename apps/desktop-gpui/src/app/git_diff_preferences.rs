//! Display options are scoped to the environment page, never to a checkout receipt.
use crate::NativeApp;
use gpui_kit::Context;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

#[derive(Default, Serialize, Deserialize, Clone, Copy, PartialEq, Debug)]
struct Preferences {
    split: bool,
    wrap: bool,
    ignore_whitespace: bool,
}
fn read(path: &Path) -> BTreeMap<String, Preferences> {
    std::fs::read(path)
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}
fn save(path: &Path, environment: &str, preferences: Preferences) -> std::io::Result<()> {
    // Re-read at the synchronous write boundary: another retained page may have saved.
    let mut document = read(path);
    document.insert(environment.into(), preferences);
    super::checkout_operations::persist(path, &serde_json::to_vec(&document)?)
}
impl NativeApp {
    fn diff_preferences_path(&self) -> Option<PathBuf> {
        #[cfg(test)]
        {
            self.diffs.preferences_path.clone()
        }
        #[cfg(not(test))]
        {
            Some(
                t3code_desktop_gpui::drafts::default_path().with_file_name("diff-preferences.json"),
            )
        }
    }
    pub(crate) fn restore_diff_preferences(&mut self) {
        if self.diffs.preferences_loaded {
            return;
        }
        self.diffs.preferences_loaded = true;
        if let Some(preferences) = self
            .diff_preferences_path()
            .and_then(|path| read(&path).get(&self.environment_scope).copied())
        {
            self.diffs.split = preferences.split;
            self.diffs.wrap = preferences.wrap;
            self.diffs.ignore_whitespace = preferences.ignore_whitespace;
        }
    }
    pub(crate) fn save_diff_preferences(&mut self, cx: &mut Context<Self>) {
        self.diffs.preferences_loaded = true;
        if let Some(path) = self.diff_preferences_path() {
            let preferences = Preferences {
                split: self.diffs.split,
                wrap: self.diffs.wrap,
                ignore_whitespace: self.diffs.ignore_whitespace,
            };
            if let Err(error) = save(&path, &self.environment_scope, preferences) {
                self.report_error(format!("Could not save diff options: {error}"), cx);
            }
        }
        cx.notify();
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn r16_e_preferences_roundtrip_keeps_other_environment_and_reverse_options() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("diff.json");
        let enabled = Preferences {
            split: true,
            wrap: true,
            ignore_whitespace: true,
        };
        save(&path, "office", enabled).unwrap();
        save(&path, "lab", Preferences::default()).unwrap();
        assert_eq!(read(&path)["office"], enabled);
        assert_eq!(read(&path)["lab"], Preferences::default());
        save(&path, "office", Preferences::default()).unwrap();
        assert_eq!(read(&path)["office"], Preferences::default());
        assert_eq!(read(&path).len(), 2);
    }
}
