//! Repository operations always execute in the connected environment through server RPCs.
use crate::commands;
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CloneProgress {
    pub project_id: String,
    pub destination_path: String,
    pub phase: String,
    pub stage: String,
    pub percent: Option<u64>,
    pub detail: Option<String>,
    pub error: Option<String>,
    pub sequence: u64,
}
pub fn clone_request(title: &str, remote: &str, destination: &str) -> Result<Value, &'static str> {
    if title.trim().is_empty() || remote.trim().is_empty() || destination.trim().is_empty() {
        return Err("Clone requires a title, repository URL and destination directory");
    }
    Ok(
        json!({"projectId":commands::id(),"title":title.trim(),"createdAt":commands::now(),"remoteUrl":remote.trim(),"destinationPath":destination.trim()}),
    )
}
pub fn apply_clones(previous: &mut Vec<CloneProgress>, value: Value) -> Result<(), String> {
    let mut incoming: Vec<CloneProgress> =
        serde_json::from_value(value).map_err(|_| "Invalid repository clone progress")?;
    for item in &mut incoming {
        if let Some(old) = previous
            .iter()
            .find(|old| old.project_id == item.project_id && old.sequence > item.sequence)
        {
            *item = old.clone();
        }
    }
    *previous = incoming;
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    fn progress(sequence: u64, phase: &str) -> Value {
        json!([{"projectId":"p","destinationPath":"/tmp/owned","phase":phase,"stage":"checkout","percent":null,"detail":null,"error":null,"sequence":sequence}])
    }
    #[test]
    fn reversed_progress_cannot_revive_completed_clone_and_snapshot_removal_is_authoritative() {
        let mut rows = Vec::new();
        apply_clones(&mut rows, progress(3, "done")).unwrap();
        apply_clones(&mut rows, progress(2, "running")).unwrap();
        assert_eq!(rows[0].phase, "done");
        apply_clones(&mut rows, json!([])).unwrap();
        assert!(rows.is_empty());
    }
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Ref {
    pub name: String,
    pub current: bool,
    pub worktree_path: Option<String>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Refs {
    pub refs: Vec<Ref>,
    pub is_repo: bool,
    pub next_cursor: Option<u64>,
}
pub fn create_worktree(
    cwd: &str,
    base: &str,
    branch: &str,
    path: &str,
) -> Result<Value, &'static str> {
    if base.trim().is_empty() {
        return Err("Select a base branch first");
    }
    let mut request = json!({"cwd":cwd,"refName":base.trim(),"path":if path.trim().is_empty(){Value::Null}else{json!(path.trim())}});
    if !branch.trim().is_empty() {
        request["newRefName"] = json!(branch.trim());
        request["baseRefName"] = json!(base.trim());
    }
    Ok(request)
}
