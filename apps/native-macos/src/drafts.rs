//! Native-only draft persistence, isolated from the shipping client's live state.
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs, io,
    path::{Path, PathBuf},
};

#[derive(Default, Clone, Serialize, Deserialize)]
struct EnvironmentDrafts {
    drafts: BTreeMap<String, String>,
    stash: Vec<String>,
}

#[derive(Clone, Serialize, Deserialize)]
struct Document {
    version: u8,
    environments: BTreeMap<String, EnvironmentDrafts>,
}

pub struct Drafts {
    path: PathBuf,
    document: Document,
    revision: u64,
    committed: std::sync::Arc<std::sync::Mutex<u64>>,
}

pub struct Write {
    path: PathBuf,
    data: Vec<u8>,
    revision: u64,
    committed: std::sync::Arc<std::sync::Mutex<u64>>,
}

impl Drafts {
    pub fn open(path: PathBuf) -> io::Result<Self> {
        let document = match fs::read(&path) {
            Ok(data) => {
                if data.len() > 16 * 1024 * 1024 {
                    return Err(io::Error::other("Draft file exceeds 16 MiB"));
                }
                let document: Document = serde_json::from_slice(&data).map_err(io::Error::other)?;
                if document.version != 1 {
                    return Err(io::Error::other("Unsupported draft version"));
                }
                document
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => Document {
                version: 1,
                environments: Default::default(),
            },
            Err(error) => return Err(error),
        };
        Ok(Self {
            path,
            document,
            revision: 0,
            committed: Default::default(),
        })
    }

    pub fn get(&self, environment: &str, thread: &str) -> &str {
        self.document
            .environments
            .get(environment)
            .and_then(|e| e.drafts.get(thread))
            .map(String::as_str)
            .unwrap_or("")
    }
    pub fn set(&mut self, environment: &str, thread: &str, text: String) {
        self.revision += 1;
        let state = self
            .document
            .environments
            .entry(environment.into())
            .or_default();
        if text.is_empty() {
            state.drafts.remove(thread);
        } else {
            state.drafts.insert(thread.into(), text);
        }
    }
    pub fn stash(&mut self, environment: &str, text: String) {
        if !text.trim().is_empty() {
            self.revision += 1;
            self.document
                .environments
                .entry(environment.into())
                .or_default()
                .stash
                .push(text);
        }
    }
    pub fn unstash(&mut self, environment: &str) -> Option<String> {
        let result = self
            .document
            .environments
            .get_mut(environment)
            .and_then(|e| e.stash.pop());
        if result.is_some() {
            self.revision += 1;
        }
        result
    }
    pub fn write_snapshot(&self) -> io::Result<Write> {
        Ok(Write {
            path: self.path.clone(),
            data: serde_json::to_vec(&self.document).map_err(io::Error::other)?,
            revision: self.revision,
            committed: self.committed.clone(),
        })
    }
}

impl Write {
    /// Execute on the background executor after debouncing; never fsync on the UI thread.
    pub fn persist(self) -> io::Result<()> {
        let mut committed = self
            .committed
            .lock()
            .map_err(|_| io::Error::other("Draft writer lock poisoned"))?;
        if self.revision < *committed {
            return Ok(());
        }
        let directory = self
            .path
            .parent()
            .ok_or_else(|| io::Error::other("Draft path has no parent"))?;
        fs::create_dir_all(directory)?;
        let temporary = directory.join(format!(".drafts-{}.tmp", uuid::Uuid::new_v4()));
        let result = (|| {
            use std::io::Write;
            let mut options = fs::OpenOptions::new();
            options.create_new(true).write(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            let mut file = options.open(&temporary)?;
            file.write_all(&self.data)?;
            file.sync_all()?;
            fs::rename(&temporary, &self.path)?;
            fs::File::open(directory)?.sync_all()?;
            Ok(())
        })();
        if result.is_err() {
            let _ = fs::remove_file(temporary);
        } else {
            *committed = self.revision;
        }
        result
    }
}

pub fn default_path() -> PathBuf {
    if let Some(directory) = std::env::var_os("T3_NATIVE_HOME") {
        return Path::new(&directory).join("drafts.json");
    }
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    home.join("Library/Application Support/T3 Code Native/drafts.json")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn drafts_and_stash_survive_restart_without_cross_environment_leaks() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("drafts.json");
        let mut store = Drafts::open(path.clone()).unwrap();
        store.set("local", "t", "Hello 🌍\n世界".into());
        store.set("remote", "t", "Different draft".into());
        store.stash("local", "Saved prompt".into());
        store.write_snapshot().unwrap().persist().unwrap();
        let mut reopened = Drafts::open(path).unwrap();
        assert_eq!(reopened.get("local", "t"), "Hello 🌍\n世界");
        assert_eq!(reopened.get("remote", "t"), "Different draft");
        assert_eq!(reopened.unstash("local"), Some("Saved prompt".into()));
        assert!(reopened.unstash("remote").is_none());
    }
    #[test]
    fn corrupted_or_future_storage_is_never_silently_overwritten() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("drafts.json");
        fs::write(&path, "broken").unwrap();
        assert!(Drafts::open(path.clone()).is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), "broken");
        fs::write(&path, r#"{"version":2,"environments":{}}"#).unwrap();
        assert!(Drafts::open(path).is_err());
    }
    #[test]
    fn late_background_write_cannot_overwrite_a_newer_draft() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("drafts.json");
        let mut store = Drafts::open(path.clone()).unwrap();
        store.set("local", "t", "Old".into());
        let old = store.write_snapshot().unwrap();
        store.set("local", "t", "New".into());
        store.write_snapshot().unwrap().persist().unwrap();
        old.persist().unwrap();
        assert_eq!(Drafts::open(path).unwrap().get("local", "t"), "New");
    }
    #[test]
    fn clearing_one_draft_preserves_other_threads() {
        let directory = tempfile::tempdir().unwrap();
        let mut store = Drafts::open(directory.path().join("drafts.json")).unwrap();
        store.set("local", "a", "First".into());
        store.set("local", "b", "Second".into());
        store.set("local", "a", String::new());
        assert_eq!(store.get("local", "a"), "");
        assert_eq!(store.get("local", "b"), "Second");
    }
}
