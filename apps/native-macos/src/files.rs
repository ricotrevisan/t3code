//! File results are bound to the read request, never to whatever path is currently typed.
use serde::Deserialize;
#[derive(Clone, Debug, Deserialize)]
pub struct Entry {
    pub path: String,
    pub kind: String,
}
#[derive(Deserialize)]
pub struct Listing {
    pub entries: Vec<Entry>,
    pub truncated: bool,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Read {
    pub relative_path: String,
    pub contents: String,
    pub truncated: bool,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Target {
    pub cwd: String,
    pub path: String,
}
#[derive(Default)]
pub struct Editor {
    pub target: Option<Target>,
    pub saved: String,
    pub truncated: bool,
}
impl Editor {
    pub fn writable(&self) -> bool {
        self.target.as_ref().is_some_and(|target| {
            !target.path.starts_with('/') && !target.path.split('/').any(|part| part == "..")
        }) && !self.truncated
    }
    pub fn dirty(&self, text: &str) -> bool {
        self.target.is_some() && self.saved != text
    }
    pub fn loaded(&mut self, cwd: String, result: Read) {
        self.target = Some(Target {
            cwd,
            path: result.relative_path,
        });
        self.saved = result.contents;
        self.truncated = result.truncated;
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn truncated_external_and_traversal_reads_cannot_be_written() {
        let mut editor = Editor::default();
        for path in ["/tmp/outside", "../outside", "dir/../outside"] {
            editor.loaded(
                "/tmp/workspace".into(),
                Read {
                    relative_path: path.into(),
                    contents: "text".into(),
                    truncated: false,
                },
            );
            assert!(!editor.writable());
        }
        editor.loaded(
            "/tmp/workspace".into(),
            Read {
                relative_path: "file.txt".into(),
                contents: "text".into(),
                truncated: true,
            },
        );
        assert!(!editor.writable());
        editor.truncated = false;
        assert!(editor.writable());
        assert!(editor.dirty("edited"));
        assert!(!editor.dirty("text"));
    }
}
