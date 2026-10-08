//! Host-owned PR identity and review coordinates. Never infer a successful mutation locally.
use serde_json::{Value, json};

pub fn text<'a>(v: &'a Value, key: &str) -> &'a str {
    v[key].as_str().unwrap_or("")
}
pub fn contains(v: &Value, key: &str, item: &str) -> bool {
    v[key]
        .as_array()
        .is_some_and(|xs| xs.iter().any(|x| x == item))
}
pub fn reference(row: &Value) -> Option<Value> {
    let mut r = json!({"projectId":row["projectId"].as_str()?,"repository":row["repository"].as_str()?,"number":row["number"].as_u64().filter(|n| *n > 0)?});
    if let Some(host) = row["host"].as_str().map(str::to_owned).or_else(|| {
        url::Url::parse(text(row, "url")).ok().and_then(|u| {
            u.host_str().map(|host| {
                if row["provider"] == "forgejo"
                    && let Some(port) = u.port()
                {
                    format!("{host}:{port}")
                } else {
                    host.to_owned()
                }
            })
        })
    }) {
        r["host"] = json!(host);
    }
    Some(r)
}
pub fn same_resource(expected: &Value, detail: &Value) -> bool {
    let Some(actual) = reference(detail) else {
        return false;
    };
    expected["projectId"] == actual["projectId"]
        && expected["number"] == actual["number"]
        && text(expected, "repository").eq_ignore_ascii_case(text(&actual, "repository"))
        && expected["host"]
            .as_str()
            .is_none_or(|host| host.eq_ignore_ascii_case(text(&actual, "host")))
}
/// Server capabilities and viewer permissions independently narrow each offer.
pub fn merge_reason(detail: &Value, method: &str) -> Option<&'static str> {
    if !contains(&detail["capabilities"], "actions", "merge") {
        return Some("This host does not support merging");
    }
    if !contains(&detail["viewerPermissions"], "actions", "merge") {
        return Some("You do not have permission to merge");
    }
    if detail["state"] != "open" {
        return Some("Only open pull requests can be merged");
    }
    if detail["isDraft"] == true {
        return Some("Mark this draft ready before merging");
    }
    if detail["mergeability"] == "conflicting" {
        return Some("Resolve branch conflicts before merging");
    }
    if !contains(&detail["capabilities"], "mergeMethods", method)
        || detail["mergeCapabilities"][method] != true
    {
        return Some("This repository does not allow this merge strategy");
    }
    None
}
#[derive(Clone, Debug)]
pub enum Position {
    Added(u64),
    Deleted(u64),
    Context { old: u64, new: u64 },
}
impl Position {
    pub fn value(&self) -> Value {
        match self {
            Self::Added(line) => json!({"kind":"added","newLine":line}),
            Self::Deleted(line) => json!({"kind":"deleted","oldLine":line}),
            Self::Context { old, new } => {
                json!({"kind":"context","oldLine":old,"newLine":new,"side":"right"})
            }
        }
    }
}
#[derive(Clone, Debug)]
pub struct Line {
    pub text: String,
    pub position: Option<Position>,
}
#[derive(Clone, Debug, Default)]
pub struct File {
    pub path: String,
    pub old_path: String,
    pub lines: std::sync::Arc<Vec<Line>>,
}
fn path(raw: &str) -> String {
    let raw = raw.trim_end();
    let decoded = if raw.starts_with('"') && raw.ends_with('"') {
        let mut out = Vec::new();
        let mut bytes = raw.as_bytes()[1..raw.len() - 1].iter().copied().peekable();
        while let Some(b) = bytes.next() {
            if b != b'\\' {
                out.push(b);
                continue;
            }
            let Some(b) = bytes.next() else { break };
            if (b'0'..=b'7').contains(&b) {
                let mut n = u16::from(b - b'0');
                for _ in 0..2 {
                    if let Some(b) = bytes.peek().copied().filter(|b| (b'0'..=b'7').contains(b)) {
                        bytes.next();
                        n = n * 8 + u16::from(b - b'0');
                    } else {
                        break;
                    }
                }
                out.push(n as u8);
            } else {
                out.push(match b {
                    b't' => b'\t',
                    b'n' => b'\n',
                    b'r' => b'\r',
                    other => other,
                });
            }
        }
        String::from_utf8_lossy(&out).into_owned()
    } else {
        raw.to_owned()
    };
    decoded
        .strip_prefix("a/")
        .or_else(|| decoded.strip_prefix("b/"))
        .unwrap_or(&decoded)
        .to_owned()
}
/// Parse once on receipt. Only real hunk coordinates can become review positions.
pub fn parse_patch(patch: &str) -> Vec<File> {
    let mut files: Vec<File> = Vec::new();
    let (mut old, mut new) = (0, 0);
    let mut hunk = false;
    for raw in patch.lines() {
        if raw.starts_with("diff --git ") {
            files.push(File {
                path: raw
                    .rsplit_once(" b/")
                    .map(|(_, p)| p.to_owned())
                    .unwrap_or_else(|| "Changed file".into()),
                ..Default::default()
            });
            hunk = false;
        }
        let Some(file) = files.last_mut() else {
            continue;
        };
        if !hunk {
            if let Some(p) = raw.strip_prefix("--- ").filter(|p| *p != "/dev/null") {
                file.old_path = path(p);
            }
            if let Some(p) = raw.strip_prefix("+++ ").filter(|p| *p != "/dev/null") {
                file.path = path(p);
            }
            if let Some(p) = raw.strip_prefix("rename to ") {
                file.path = path(p);
            }
            if let Some(p) = raw.strip_prefix("rename from ") {
                file.old_path = path(p);
            }
        }
        let position = if raw.starts_with("@@ ") {
            let parts: Vec<_> = raw.split_whitespace().collect();
            old = parts
                .get(1)
                .and_then(|p| {
                    p.trim_start_matches('-')
                        .split(',')
                        .next()?
                        .parse::<u64>()
                        .ok()
                })
                .unwrap_or(0);
            new = parts
                .get(2)
                .and_then(|p| {
                    p.trim_start_matches('+')
                        .split(',')
                        .next()?
                        .parse::<u64>()
                        .ok()
                })
                .unwrap_or(0);
            hunk = true;
            None
        } else if hunk && raw.starts_with('+') && new > 0 {
            let p = Position::Added(new);
            new += 1;
            Some(p)
        } else if hunk && raw.starts_with('-') && old > 0 {
            let p = Position::Deleted(old);
            old += 1;
            Some(p)
        } else if hunk && raw.starts_with(' ') && old > 0 && new > 0 {
            let p = Position::Context { old, new };
            old += 1;
            new += 1;
            Some(p)
        } else {
            None
        };
        std::sync::Arc::make_mut(&mut file.lines).push(Line {
            text: raw.to_owned(),
            position,
        });
    }
    files
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn r14_p_positions_preserve_rename_and_deleted_side() {
        let files = parse_patch(
            "diff --git a/old.rs b/new.rs\nrename from old.rs\nrename to new.rs\n--- a/old.rs\n+++ b/new.rs\n@@ -7,2 +9,2 @@\n context\n-old\n+new\n\\ No newline at end of file\n",
        );
        assert_eq!(files[0].old_path, "old.rs");
        assert_eq!(files[0].path, "new.rs");
        assert_eq!(
            files[0].lines[7].position.as_ref().map(Position::value),
            Some(json!({"kind":"deleted","oldLine":8}))
        );
        assert_eq!(
            files[0].lines[8].position.as_ref().map(Position::value),
            Some(json!({"kind":"added","newLine":10}))
        );
        assert!(files[0].lines[9].position.is_none());
    }
    #[test]
    fn r14_p_merge_intersects_host_repository_and_viewer() {
        let mut d = json!({"state":"open","isDraft":false,"mergeability":"mergeable","capabilities":{"actions":["merge"],"mergeMethods":["squash"]},"viewerPermissions":{"actions":["merge"]},"mergeCapabilities":{"squash":true}});
        assert_eq!(merge_reason(&d, "squash"), None);
        assert!(merge_reason(&d, "rebase").is_some());
        d["viewerPermissions"]["actions"] = json!([]);
        assert_eq!(
            merge_reason(&d, "squash"),
            Some("You do not have permission to merge")
        );
    }
    #[test]
    fn r14_p_reference_keeps_forgejo_authority_and_rejects_wrong_detail() {
        let detail = json!({"projectId":"p","provider":"forgejo","repository":"a/b","number":7,"url":"https://forge.example:8443/a/b/pulls/7"});
        let r = reference(&detail).unwrap();
        assert_eq!(r["host"], "forge.example:8443");
        assert!(same_resource(&r, &detail));
        let mut wrong = detail.clone();
        wrong["number"] = json!(8);
        assert!(!same_resource(&r, &wrong));
        wrong = detail.clone();
        wrong["url"] = json!("https://forge.example:9443/a/b/pulls/7");
        assert!(!same_resource(&r, &wrong));
    }
}
