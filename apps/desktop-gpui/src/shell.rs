//! Ordered shell projection. The global sequence may skip unrelated thread events.
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Project {
    pub id: String,
    pub title: String,
    pub workspace_root: String,
    #[serde(default)]
    pub default_model_selection: Option<serde_json::Value>,
    #[serde(default)]
    pub scripts: Vec<serde_json::Value>,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Session {
    pub status: String,
    #[serde(default)]
    pub last_error: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LatestTurn {
    pub turn_id: String,
    pub state: String,
    #[serde(default)]
    pub requested_at: Option<String>,
    #[serde(default)]
    pub started_at: Option<String>,
    #[serde(default)]
    pub completed_at: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Thread {
    pub id: String,
    pub project_id: String,
    pub title: String,
    #[serde(default)]
    pub archived_at: Option<String>,
    #[serde(default)]
    pub model_selection: serde_json::Value,
    #[serde(default)]
    pub runtime_mode: Option<String>,
    #[serde(default)]
    pub interaction_mode: Option<String>,
    #[serde(default)]
    pub snoozed_until: Option<String>,
    #[serde(default)]
    pub snoozed_at: Option<String>,
    #[serde(default)]
    pub settled_override: Option<String>,
    #[serde(default)]
    pub auto_settle_disabled_at: Option<String>,
    #[serde(default)]
    pub pinned_at: Option<String>,
    #[serde(default)]
    pub pin_order_key: Option<String>,
    #[serde(default)]
    pub active_order_key: Option<String>,
    #[serde(default)]
    pub unsettled_at: Option<String>,
    #[serde(default)]
    pub title_regeneration: Option<serde_json::Value>,
    #[serde(default)]
    pub branch: Option<String>,
    #[serde(default)]
    pub worktree_path: Option<String>,
    // Shell-level live state, unset on older servers.
    #[serde(default)]
    pub session: Option<Session>,
    /// Captured at the shell wire boundary, including status-edge timestamps.
    /// Kept separate from Session so existing client-created session fixtures
    /// cannot fabricate a fresh error merely by setting its status.
    #[serde(default)]
    pub session_updated_at: Option<String>,
    #[serde(default)]
    pub latest_turn: Option<LatestTurn>,
    #[serde(default)]
    pub background_liveness: Option<String>,
    #[serde(default)]
    pub has_pending_approvals: bool,
    #[serde(default)]
    pub has_pending_user_input: bool,
    #[serde(default)]
    pub has_actionable_proposed_plan: bool,
    #[serde(default)]
    pub linked_pull_request: Option<serde_json::Value>,
    #[serde(default)]
    pub latest_user_message_at: Option<String>,
    #[serde(default)]
    pub updated_at: Option<String>,
    #[serde(default)]
    pub created_at: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub snapshot_sequence: u64,
    pub projects: Vec<Project>,
    #[serde(deserialize_with = "decode_threads")]
    pub threads: Vec<Thread>,
}

fn decode_thread_value(value: serde_json::Value) -> Result<Thread, serde_json::Error> {
    let updated_at = value["session"]["updatedAt"].as_str().map(str::to_owned);
    let mut thread: Thread = serde_json::from_value(value)?;
    thread.session_updated_at = updated_at;
    Ok(thread)
}

fn decode_thread<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<Thread, D::Error> {
    decode_thread_value(serde_json::Value::deserialize(deserializer)?)
        .map_err(serde::de::Error::custom)
}

fn decode_threads<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Vec<Thread>, D::Error> {
    Vec::<serde_json::Value>::deserialize(deserializer)?
        .into_iter()
        .map(|value| decode_thread_value(value).map_err(serde::de::Error::custom))
        .collect()
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
// Wire events carry the full thread/project projection; the variant-size lint
// does not apply to a decoded network message.
#[allow(clippy::large_enum_variant)]
pub enum ShellItem {
    Snapshot {
        snapshot: Snapshot,
    },
    Synchronized,
    ProjectUpserted {
        sequence: u64,
        project: Project,
    },
    ProjectRemoved {
        sequence: u64,
        #[serde(rename = "projectId")]
        project_id: String,
    },
    ThreadUpserted {
        sequence: u64,
        #[serde(deserialize_with = "decode_thread")]
        thread: Thread,
    },
    ThreadRemoved {
        sequence: u64,
        #[serde(rename = "threadId")]
        thread_id: String,
    },
}

#[derive(Default)]
pub struct Shell {
    /// Local identity also advances for replacement snapshots at the same wire sequence.
    pub revision: u64,
    pub sequence: Option<u64>,
    pub projects: Vec<Project>,
    pub threads: Vec<Thread>,
    pub synchronized: bool,
}

impl Shell {
    pub fn apply(&mut self, item: ShellItem) -> Result<(), &'static str> {
        match item {
            ShellItem::Snapshot { snapshot } => {
                self.revision += 1;
                self.sequence = Some(snapshot.snapshot_sequence);
                self.projects = snapshot.projects;
                self.threads = snapshot.threads;
                self.synchronized = false;
                return Ok(());
            }
            ShellItem::Synchronized => {
                if self.sequence.is_none() {
                    return Err("synchronized before initial snapshot");
                }
                self.synchronized = true;
                return Ok(());
            }
            _ => {}
        }
        let sequence = match &item {
            ShellItem::ProjectUpserted { sequence, .. }
            | ShellItem::ProjectRemoved { sequence, .. }
            | ShellItem::ThreadUpserted { sequence, .. }
            | ShellItem::ThreadRemoved { sequence, .. } => *sequence,
            _ => unreachable!(),
        };
        let previous = self.sequence.ok_or("event before initial snapshot")?;
        if sequence <= previous {
            return Ok(());
        }
        match item {
            ShellItem::ProjectUpserted { project, .. } => {
                if let Some(existing) = self.projects.iter_mut().find(|p| p.id == project.id) {
                    *existing = project;
                } else {
                    self.projects.push(project);
                }
            }
            ShellItem::ThreadUpserted { thread, .. } => {
                if let Some(existing) = self.threads.iter_mut().find(|t| t.id == thread.id) {
                    *existing = thread;
                } else {
                    self.threads.push(thread);
                }
            }
            ShellItem::ProjectRemoved { project_id, .. } => {
                self.projects.retain(|p| p.id != project_id);
            }
            ShellItem::ThreadRemoved { thread_id, .. } => {
                self.threads.retain(|t| t.id != thread_id);
            }
            _ => unreachable!(),
        }
        self.sequence = Some(sequence);
        self.revision += 1;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn apply(shell: &mut Shell, value: serde_json::Value) {
        shell.apply(serde_json::from_value(value).unwrap()).unwrap();
    }

    #[test]
    fn snapshot_replay_deduplication_and_reverse_actions() {
        let mut shell = Shell::default();
        apply(
            &mut shell,
            json!({"kind":"snapshot","snapshot":{"snapshotSequence":4,"projects":[{"id":"p","title":"Project","workspaceRoot":"/tmp/p"}],"threads":[]}}),
        );
        apply(
            &mut shell,
            json!({"kind":"thread-upserted","sequence":8,"thread":{"id":"t","projectId":"p","title":"First","archivedAt":null}}),
        );
        apply(
            &mut shell,
            json!({"kind":"thread-upserted","sequence":7,"thread":{"id":"t","projectId":"p","title":"Stale"}}),
        );
        assert_eq!(shell.threads[0].title, "First");
        apply(
            &mut shell,
            json!({"kind":"thread-upserted","sequence":9,"thread":{"id":"t","projectId":"p","title":"Renamed","archivedAt":"2026-01-01"}}),
        );
        assert!(shell.threads[0].archived_at.is_some());
        apply(
            &mut shell,
            json!({"kind":"thread-upserted","sequence":10,"thread":{"id":"t","projectId":"p","title":"Renamed","archivedAt":null}}),
        );
        assert!(shell.threads[0].archived_at.is_none());
        apply(&mut shell, json!({"kind":"synchronized"}));
        assert!(shell.synchronized);
        apply(
            &mut shell,
            json!({"kind":"thread-removed","sequence":11,"threadId":"t"}),
        );
        assert!(shell.threads.is_empty());
        apply(
            &mut shell,
            json!({"kind":"project-removed","sequence":12,"projectId":"p"}),
        );
        assert!(shell.projects.is_empty());
    }

    #[test]
    fn rejects_events_without_snapshot_and_resets_on_reconnect_snapshot() {
        let mut shell = Shell::default();
        assert!(shell.apply(ShellItem::Synchronized).is_err());
        assert!(
            shell
                .apply(ShellItem::ThreadRemoved {
                    sequence: 1,
                    thread_id: "t".into()
                })
                .is_err()
        );
        apply(
            &mut shell,
            json!({"kind":"snapshot","snapshot":{"snapshotSequence":20,"projects":[],"threads":[]}}),
        );
        apply(&mut shell, json!({"kind":"synchronized"}));
        apply(
            &mut shell,
            json!({"kind":"snapshot","snapshot":{"snapshotSequence":30,"projects":[],"threads":[]}}),
        );
        assert!(!shell.synchronized);
        assert_eq!(shell.sequence, Some(30));
    }
}
