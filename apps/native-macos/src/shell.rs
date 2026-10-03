//! Ordered shell projection. The global sequence may skip unrelated thread events.
use serde::Deserialize;

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Project {
    pub id: String,
    pub title: String,
    pub workspace_root: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Thread {
    pub id: String,
    pub project_id: String,
    pub title: String,
    #[serde(default)]
    pub archived_at: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub snapshot_sequence: u64,
    pub projects: Vec<Project>,
    pub threads: Vec<Thread>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
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
    pub sequence: Option<u64>,
    pub projects: Vec<Project>,
    pub threads: Vec<Thread>,
    pub synchronized: bool,
}

impl Shell {
    pub fn apply(&mut self, item: ShellItem) -> Result<(), &'static str> {
        match item {
            ShellItem::Snapshot { snapshot } => {
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
