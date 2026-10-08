//! Device-local terminal labels, scoped to environment + thread + terminal identity.
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, io, path::PathBuf};

#[derive(Default, Serialize, Deserialize)]
pub(super) struct Names {
    #[serde(skip)]
    pub path: Option<PathBuf>,
    labels: BTreeMap<String, String>,
    #[serde(skip)]
    blocked: bool,
}
impl Names {
    pub fn unavailable() -> Self {
        Self {
            blocked: true,
            ..Self::default()
        }
    }
    fn key(scope: &str, thread: &str, id: &str) -> String {
        serde_json::to_string(&(scope, thread, id)).unwrap()
    }
    pub fn load(path: PathBuf) -> io::Result<Self> {
        let mut names: Self = match std::fs::read(&path) {
            Ok(bytes) => serde_json::from_slice(&bytes).map_err(io::Error::other)?,
            Err(error) if error.kind() == io::ErrorKind::NotFound => Self::default(),
            Err(error) => return Err(error),
        };
        names.path = Some(path);
        Ok(names)
    }
    pub fn get(&self, scope: &str, thread: &str, id: &str) -> Option<&str> {
        self.labels
            .get(&Self::key(scope, thread, id))
            .map(String::as_str)
    }
    pub fn set(&mut self, scope: &str, thread: &str, id: &str, title: &str) -> io::Result<()> {
        if self.blocked {
            return Err(io::Error::other(
                "Name storage could not be read; existing file was preserved",
            ));
        }
        let key = Self::key(scope, thread, id);
        let old = self.labels.insert(key.clone(), title.to_owned());
        let result = self.save();
        if result.is_err() {
            if let Some(old) = old {
                self.labels.insert(key, old);
            } else {
                self.labels.remove(&key);
            }
        }
        result
    }
    fn save(&self) -> io::Result<()> {
        let Some(path) = &self.path else {
            return Ok(());
        };
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let temp = path.with_extension("tmp");
        std::fs::write(&temp, serde_json::to_vec(self).map_err(io::Error::other)?)?;
        std::fs::rename(temp, path)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn terminal_names_survive_reload_and_never_cross_scopes() {
        let root = std::env::temp_dir().join(format!("r13-h-{}", uuid::Uuid::new_v4()));
        let path = root.join("names.json");
        let mut names = Names::load(path.clone()).unwrap();
        names.set("env-a", "thread", "term-1", "Build").unwrap();
        let names = Names::load(path).unwrap();
        assert_eq!(names.get("env-a", "thread", "term-1"), Some("Build"));
        assert_eq!(names.get("env-b", "thread", "term-1"), None);
        assert_eq!(names.get("env-a", "other", "term-1"), None);
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn terminal_names_failed_save_preserves_previous_label() {
        let mut names = Names::default();
        names.set("e", "t", "i", "old").unwrap();
        names.path = Some(PathBuf::from("/dev/null/impossible.json"));
        assert!(names.set("e", "t", "i", "new").is_err());
        assert_eq!(names.get("e", "t", "i"), Some("old"));
    }
}
