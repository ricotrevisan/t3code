//! Native-only draft persistence, isolated from the shipping client's live state.
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs, io,
    path::{Path, PathBuf},
};

#[derive(Default, Clone, Serialize, Deserialize)]
struct EnvironmentDrafts {
    #[serde(default)]
    pending_threads: BTreeMap<String, PendingThread>,
    drafts: BTreeMap<String, String>,
    #[serde(default)]
    resources: BTreeMap<String, Vec<serde_json::Value>>,
    stash: Vec<StashEntry>,
}

/// A stash owns its uploaded references; these never remain on the cleared draft.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(from = "StoredStash")]
pub struct StashEntry {
    pub id: String,
    pub created_at: String,
    pub prompt: String,
    pub attachments: Vec<serde_json::Value>,
    pub context: Option<serde_json::Value>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum StoredStash {
    Legacy(String),
    Record {
        id: String,
        created_at: String,
        prompt: String,
        #[serde(default)]
        attachments: Vec<serde_json::Value>,
        #[serde(default)]
        context: Option<serde_json::Value>,
    },
}

impl From<StoredStash> for StashEntry {
    fn from(stored: StoredStash) -> Self {
        match stored {
            StoredStash::Legacy(prompt) => Self::new(prompt, Vec::new()),
            StoredStash::Record {
                id,
                created_at,
                prompt,
                attachments,
                context,
            } => Self {
                id,
                created_at,
                prompt,
                attachments,
                context,
            },
        }
    }
}

impl StashEntry {
    pub fn new(prompt: String, attachments: Vec<serde_json::Value>) -> Self {
        let context = crate::mentions::message_context(&prompt).1;
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            created_at: crate::commands::now(),
            prompt,
            attachments,
            context,
        }
    }
    pub fn snippet(&self) -> String {
        let text = self.prompt.split_whitespace().collect::<Vec<_>>().join(" ");
        if text.is_empty() {
            return format!("({} attachments)", self.attachments.len());
        }
        let mut snippet: String = text.chars().take(90).collect();
        if text.chars().count() > 90 {
            snippet.push('…');
        }
        snippet
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct PendingThread {
    pub thread: crate::shell::Thread,
    pub created_at: String,
    pub prepare_worktree: Option<serde_json::Value>,
    pub run_setup_script: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SendShortcut {
    #[default]
    Enter,
    ModEnter,
    ModEnterMultiline,
}

impl SendShortcut {
    pub fn requires_modifier(self, prompt: &str) -> bool {
        self == Self::ModEnter || (self == Self::ModEnterMultiline && prompt.contains(['\n', '\r']))
    }
}

#[derive(Clone, Serialize, Deserialize)]
struct Document {
    version: u8,
    #[serde(default)]
    send_shortcut: SendShortcut,
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
                if !matches!(document.version, 1 | 2) {
                    return Err(io::Error::other("Unsupported draft version"));
                }
                Document {
                    version: 2,
                    ..document
                }
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => Document {
                version: 2,
                send_shortcut: Default::default(),
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

    pub fn send_shortcut(&self) -> SendShortcut {
        self.document.send_shortcut
    }
    pub fn set_send_shortcut(&mut self, shortcut: SendShortcut) {
        self.document.send_shortcut = shortcut;
        self.revision += 1;
    }

    pub fn pending_threads(&self, environment: &str) -> Vec<PendingThread> {
        self.document
            .environments
            .get(environment)
            .map(|state| state.pending_threads.values().cloned().collect())
            .unwrap_or_default()
    }
    pub fn put_pending(&mut self, environment: &str, draft: PendingThread) {
        self.revision += 1;
        self.document
            .environments
            .entry(environment.into())
            .or_default()
            .pending_threads
            .insert(draft.thread.id.clone(), draft);
    }
    pub fn remove_pending(&mut self, environment: &str, id: &str) {
        if let Some(state) = self.document.environments.get_mut(environment)
            && state.pending_threads.remove(id).is_some()
        {
            self.revision += 1;
        }
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
        self.stash_record(environment, StashEntry::new(text, Vec::new()));
    }
    pub fn stash_record(&mut self, environment: &str, record: StashEntry) {
        if record.prompt.trim().is_empty()
            && record.attachments.is_empty()
            && record.context.is_none()
        {
            return;
        }
        self.revision += 1;
        self.document
            .environments
            .entry(environment.into())
            .or_default()
            .stash
            .push(record);
    }
    pub fn stashes(&self, environment: &str) -> &[StashEntry] {
        self.document
            .environments
            .get(environment)
            .map(|e| e.stash.as_slice())
            .unwrap_or_default()
    }
    pub fn resources(&self, environment: &str, thread: &str) -> Vec<serde_json::Value> {
        self.document
            .environments
            .get(environment)
            .and_then(|state| state.resources.get(thread))
            .cloned()
            .unwrap_or_default()
    }
    pub fn consume_resources(
        &mut self,
        environment: &str,
        thread: &str,
        sent: &[serde_json::Value],
    ) {
        if let Some(state) = self.document.environments.get_mut(environment)
            && let Some(refs) = state.resources.get_mut(thread)
        {
            refs.retain(|item| !sent.iter().any(|sent| sent["id"] == item["id"]));
            if refs.is_empty() {
                state.resources.remove(thread);
            }
            self.revision += 1;
        }
    }
    /// One document revision transfers the stash into durable draft ownership.
    pub fn restore_record(
        &mut self,
        environment: &str,
        thread: &str,
        id: &str,
    ) -> Option<StashEntry> {
        let record = self.take_stash(environment, id)?;
        self.set(environment, thread, record.prompt.clone());
        self.document
            .environments
            .get_mut(environment)?
            .resources
            .insert(thread.into(), record.attachments.clone());
        Some(record)
    }
    pub fn take_stash(&mut self, environment: &str, id: &str) -> Option<StashEntry> {
        let state = self.document.environments.get_mut(environment)?;
        let index = state.stash.iter().position(|entry| entry.id == id)?;
        self.revision += 1;
        Some(state.stash.remove(index))
    }
    pub fn unstash(&mut self, environment: &str) -> Option<String> {
        let id = self.stashes(environment).last()?.id.clone();
        self.take_stash(environment, &id).map(|entry| entry.prompt)
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
    fn pending_thread_checkout_recipe_survives_restart_and_is_environment_scoped() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("drafts.json");
        let mut store = Drafts::open(path.clone()).unwrap();
        let draft=PendingThread{thread:serde_json::from_value(serde_json::json!({"id":"draft","projectId":"p","title":"Owned draft","modelSelection":{"instanceId":"codex","model":"gpt-6-astra"}})).unwrap(),created_at:crate::commands::now(),prepare_worktree:Some(serde_json::json!({"projectCwd":"/tmp/owned","baseBranch":"main","startFromOrigin":true})),run_setup_script:true};
        store.put_pending("a", draft);
        store.set("a", "draft", "Unsent 世界".into());
        store.write_snapshot().unwrap().persist().unwrap();
        let mut restored = Drafts::open(path.clone()).unwrap();
        assert!(restored.pending_threads("b").is_empty());
        assert_eq!(
            restored.pending_threads("a")[0]
                .prepare_worktree
                .as_ref()
                .unwrap()["baseBranch"],
            "main"
        );
        assert_eq!(restored.get("a", "draft"), "Unsent 世界");
        restored.remove_pending("a", "draft");
        restored.write_snapshot().unwrap().persist().unwrap();
        assert!(Drafts::open(path).unwrap().pending_threads("a").is_empty());
    }

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
        fs::write(&path, r#"{"version":3,"environments":{}}"#).unwrap();
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
    fn version_one_string_stashes_migrate_without_losing_drafts() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("drafts.json");
        fs::write(&path, r#"{"version":1,"environments":{"local":{"drafts":{"t":"keep"},"stash":["old one","世界"]}}}"#).unwrap();
        let mut store = Drafts::open(path.clone()).unwrap();
        assert_eq!(store.stashes("local").len(), 2);
        let record = StashEntry::new(
            "[main.rs](src/main.rs)".into(),
            vec![serde_json::json!({"id":"owned"})],
        );
        let id = record.id.clone();
        store.stash_record("local", record);
        store.write_snapshot().unwrap().persist().unwrap();
        let mut restored = Drafts::open(path).unwrap();
        assert_eq!(restored.get("local", "t"), "keep");
        assert!(restored.stashes("remote").is_empty());
        let entry = restored.take_stash("local", &id).unwrap();
        assert_eq!(entry.attachments[0]["id"], "owned");
        assert_eq!(entry.context.unwrap()["records"][0]["path"], "src/main.rs");
        assert_eq!(restored.unstash("local").unwrap(), "世界");
        assert_eq!(restored.unstash("local").unwrap(), "old one");
        assert_eq!(restored.document.version, 2);
    }

    #[test]
    fn send_shortcut_is_device_local_and_survives_restart() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("drafts.json");
        let mut store = Drafts::open(path.clone()).unwrap();
        store.set_send_shortcut(SendShortcut::ModEnterMultiline);
        store.write_snapshot().unwrap().persist().unwrap();
        let restored = Drafts::open(path).unwrap();
        assert_eq!(restored.send_shortcut(), SendShortcut::ModEnterMultiline);
        assert!(!restored.send_shortcut().requires_modifier("one line"));
        assert!(restored.send_shortcut().requires_modifier("two\nlines"));
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
