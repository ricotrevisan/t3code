//! Checkpoint patches use the same single-turn range as the web DiffPanel.
use crate::{NativeApp, RightTab};
use gpui_kit::{Context, Task};
use serde::Deserialize;
use serde_json::json;
use t3code_desktop_gpui::connection;

#[derive(Default)]
pub(crate) struct DiffState {
    pub thread_id: Option<String>,
    pub generation: u64,
    pub turn_count: Option<u64>,
    pub loading: bool,
    pub error: Option<String>,
    pub files: Vec<PatchFile>,
    pub selected_file: usize,
    pub git_scope: Option<String>,
    pub wrap: bool,
    pub expand_all: bool,
    pub collapsed: std::collections::HashSet<usize>,
    pub truncated: bool,
    epoch: u64,
    task: Option<Task<()>>,
}

impl DiffState {
    /// The thread-selection owner can call this to also invalidate switch-away/back replies.
    pub(crate) fn invalidate(&mut self) {
        let epoch = self.epoch.wrapping_add(1);
        *self = Self {
            epoch,
            ..Self::default()
        };
    }
}

pub(crate) struct PatchFile {
    pub path: String,
    pub patch: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct TurnDiff {
    thread_id: String,
    from_turn_count: u64,
    to_turn_count: u64,
    diff: String,
}

// Git quotes paths with C escapes (including octal UTF-8 bytes), not JSON escapes.
fn git_path(path: &str) -> String {
    let path = path.trim_end();
    if !path.starts_with('"') || !path.ends_with('"') {
        return path.to_owned();
    }
    let mut bytes = path.as_bytes()[1..path.len() - 1]
        .iter()
        .copied()
        .peekable();
    let mut decoded = Vec::new();
    while let Some(byte) = bytes.next() {
        if byte != b'\\' {
            decoded.push(byte);
            continue;
        }
        let Some(escaped) = bytes.next() else { break };
        if (b'0'..=b'7').contains(&escaped) {
            let mut value = u16::from(escaped - b'0');
            for _ in 0..2 {
                if let Some(next) = bytes.peek().copied().filter(|b| (b'0'..=b'7').contains(b)) {
                    bytes.next();
                    value = value * 8 + u16::from(next - b'0');
                } else {
                    break;
                }
            }
            decoded.push(value as u8);
        } else {
            decoded.push(match escaped {
                b't' => b'\t',
                b'n' => b'\n',
                b'r' => b'\r',
                b'a' => 7,
                b'b' => 8,
                b'f' => 12,
                b'v' => 11,
                other => other,
            });
        }
    }
    String::from_utf8_lossy(&decoded).into_owned()
}

fn header_path(header: &str) -> String {
    let paths = header.trim_end().trim_start_matches("diff --git ");
    if paths.starts_with('"') {
        if let Some((_, destination)) = paths.rsplit_once(" \"b/") {
            return git_path(&format!("\"b/{destination}"))
                .trim_start_matches("b/")
                .to_owned();
        }
    } else {
        // Unquoted names may themselves contain " b/". A non-rename header repeats
        // its path; authoritative +++/rename-to lines below cover changed names.
        for (index, _) in paths.match_indices(" b/") {
            let (source, destination) = paths.split_at(index);
            if source.strip_prefix("a/") == destination.strip_prefix(" b/") {
                return destination[3..].to_owned();
            }
        }
        if let Some((_, destination)) = paths.rsplit_once(" b/") {
            return destination.to_owned();
        }
    }
    "Patch".into()
}

// Preserve the server's patch verbatim, including binary/rename-only sections.
fn split_patch(patch: String) -> Vec<PatchFile> {
    if patch.trim().is_empty() {
        return Vec::new();
    }
    let mut files: Vec<PatchFile> = Vec::new();
    for line in patch.split_inclusive('\n') {
        if line.starts_with("diff --git ") || files.is_empty() {
            let path = header_path(line);
            files.push(PatchFile {
                path,
                patch: String::new(),
            });
        }
        let file = files.last_mut().unwrap();
        if let Some(path) = line.strip_prefix("+++ ") {
            let path = git_path(path);
            if path != "/dev/null" {
                file.path = path.strip_prefix("b/").unwrap_or(&path).to_owned();
            }
        } else if let Some(path) = line.strip_prefix("rename to ") {
            file.path = git_path(path);
        }
        file.patch.push_str(line);
    }
    files
}

impl NativeApp {
    pub(crate) fn view_git_diff(&mut self, scope: &str, cx: &mut Context<Self>) {
        self.inspector_open = true;
        self.right_tab = RightTab::Changes;
        let (Some(thread_id), Some(cwd), Some(sender)) = (
            self.selected_thread.clone(),
            self.selected_workspace(),
            self.commands.clone(),
        ) else {
            return;
        };
        self.diffs.epoch += 1;
        let epoch = self.diffs.epoch;
        let generation = self.generation;
        let context = self.checkout_context();
        let scope = scope.to_owned();
        self.diffs.generation = generation;
        self.diffs.thread_id = Some(thread_id);
        self.diffs.turn_count = None;
        self.diffs.git_scope = Some(scope.clone());
        self.diffs.files.clear();
        self.diffs.selected_file = 0;
        self.diffs.collapsed.clear();
        self.diffs.error = None;
        self.diffs.loading = true;
        self.diffs.task = Some(cx.spawn(async move |entity, cx| {
            let result =
                connection::call(&sender, "review.getDiffPreview", json!({"cwd":cwd})).await;
            let _ = entity.update(cx, |app, cx| {
                if app.generation != generation
                    || app.checkout_context() != context
                    || app.diffs.epoch != epoch
                {
                    return;
                }
                app.diffs.loading = false;
                match result {
                    Ok(value) if value["cwd"].as_str() == Some(cwd.as_str()) => {
                        if let Some(source) = value["sources"].as_array().and_then(|sources| {
                            sources.iter().find(|source| source["kind"] == scope)
                        }) {
                            app.diffs.files =
                                split_patch(source["diff"].as_str().unwrap_or_default().to_owned());
                            app.diffs.truncated = source["truncated"] == true;
                        } else {
                            app.diffs.error = Some("Requested diff scope is unavailable".into());
                        }
                    }
                    Ok(_) => app.diffs.error = Some("Diff response has the wrong checkout".into()),
                    Err(error) => app.diffs.error = Some(error),
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    pub(crate) fn view_checkpoint_diff(&mut self, turn_count: u64, cx: &mut Context<Self>) {
        self.inspector_open = true;
        self.right_tab = RightTab::Changes;
        let Some(thread_id) = self.selected_thread.clone() else {
            return;
        };
        self.diffs.generation = self.generation;
        self.diffs.epoch += 1;
        let epoch = self.diffs.epoch;
        self.diffs.task = None;
        self.diffs.thread_id = Some(thread_id.clone());
        self.diffs.turn_count = Some(turn_count);
        self.diffs.git_scope = None;
        self.diffs.truncated = false;
        self.diffs.collapsed.clear();
        self.diffs.files.clear();
        self.diffs.selected_file = 0;
        self.diffs.error = None;
        self.diffs.loading = true;
        let Some(sender) = self.commands.clone() else {
            self.diffs.loading = false;
            self.diffs.error = Some("Not connected".into());
            cx.notify();
            return;
        };
        let generation = self.generation;
        let from = turn_count.saturating_sub(1);
        self.diffs.task = Some(cx.spawn(async move |entity, cx| {
            let result = connection::call(
                &sender,
                "orchestration.getTurnDiff",
                json!({
                    "threadId":thread_id, "fromTurnCount":from, "toTurnCount":turn_count
                }),
            )
            .await;
            let expected_thread = thread_id.clone();
            let result = cx
                .background_executor()
                .spawn(async move {
                    let value = result?;
                    let reply: TurnDiff = serde_json::from_value(value)
                        .map_err(|_| "Invalid checkpoint diff response".to_owned())?;
                    if reply.thread_id != expected_thread
                        || reply.from_turn_count != from
                        || reply.to_turn_count != turn_count
                    {
                        return Err("Checkpoint diff response has the wrong range".to_owned());
                    }
                    Ok(split_patch(reply.diff))
                })
                .await;
            let _ = entity.update(cx, |app, cx| {
                if app.generation != generation
                    || app.selected_thread.as_ref() != Some(&thread_id)
                    || app.diffs.epoch != epoch
                    || app.diffs.turn_count != Some(turn_count)
                {
                    return;
                }
                app.diffs.loading = false;
                match result {
                    Ok(files) => app.diffs.files = files,
                    Err(error) => app.diffs.error = Some(error),
                }
                cx.notify();
            });
        }));
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn patch_file_paths_decode_git_names_without_losing_patch_bytes() {
        let cases = [
            (
                "diff --git a/dir b/name b/dir b/name\nBinary files a/dir b/name and b/dir b/name differ\n",
                "dir b/name",
            ),
            (
                "diff --git a/old name b/new name\nsimilarity index 100%\nrename from old name\nrename to new name\n",
                "new name",
            ),
            (
                "diff --git \"a/caf\\303\\251.txt\" \"b/caf\\303\\251.txt\"\n--- \"a/caf\\303\\251.txt\"\n+++ \"b/caf\\303\\251.txt\"\n@@ -1 +1 @@\n-old\n+new\n",
                "café.txt",
            ),
            (
                "diff --git a/gone b/gone\n--- a/gone\n+++ /dev/null\n",
                "gone",
            ),
            ("@@ -1 +1 @@\n-old\n+new\n", "Patch"),
        ];
        for (patch, expected) in cases {
            let files = split_patch(patch.into());
            assert_eq!(files.len(), 1);
            assert_eq!(files[0].path, expected);
            assert_eq!(files[0].patch, patch);
        }
        assert!(split_patch("\n ".into()).is_empty());
    }
}
