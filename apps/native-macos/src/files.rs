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

#[derive(Default)]
pub struct SearchOptions {
    pub case_sensitive: bool,
    pub whole_word: bool,
    pub use_regex: bool,
}
impl SearchOptions {
    pub fn request(&self, cwd: &str, query: &str) -> Result<serde_json::Value, &'static str> {
        if query.is_empty() || query.chars().map(char::len_utf16).sum::<usize>() > 256 {
            return Err("Content query must contain 1–256 characters");
        }
        Ok(
            serde_json::json!({"cwd":cwd,"query":query,"limit":100,"caseSensitive":self.case_sensitive,"wholeWord":self.whole_word,"useRegex":self.use_regex}),
        )
    }
}
#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContentMatch {
    pub path: String,
    pub line_number: usize,
    pub line_content: String,
    pub match_ranges: Vec<MatchRange>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContentResults {
    pub matches: Vec<ContentMatch>,
    pub truncated: bool,
    pub regex_fallback_error: Option<String>,
}

#[derive(Clone, Deserialize)]
pub struct MatchRange {
    pub start: usize,
    pub end: usize,
}

// Server search ranges use JavaScript UTF-16 indices; GPUI highlights use UTF-8 bytes.
pub fn utf16_byte_offset(text: &str, offset: usize) -> Option<usize> {
    let mut units = 0;
    for (byte, character) in text.char_indices() {
        if units == offset {
            return Some(byte);
        }
        units += character.len_utf16();
        if units > offset {
            return None;
        }
    }
    (units == offset).then_some(text.len())
}
impl ContentMatch {
    pub fn byte_ranges(&self) -> Vec<std::ops::Range<usize>> {
        self.match_ranges
            .iter()
            .filter_map(|range| {
                let start = utf16_byte_offset(&self.line_content, range.start)?;
                let end = utf16_byte_offset(&self.line_content, range.end)?;
                (start < end).then_some(start..end)
            })
            .collect()
    }
    pub fn character_column(&self) -> usize {
        self.byte_ranges()
            .first()
            .map_or(0, |range| self.line_content[..range.start].chars().count())
    }
}
#[cfg(test)]
mod search_tests {
    use super::*;
    #[test]
    fn unicode_match_offsets_are_safe_for_highlights_and_native_cursor() {
        let hit = ContentMatch {
            path: "notes".into(),
            line_number: 3,
            line_content: "🎉 世界".into(),
            match_ranges: vec![MatchRange { start: 3, end: 5 }],
        };
        assert_eq!(hit.byte_ranges(), vec![5..11]);
        assert_eq!(hit.character_column(), 2);
        assert_eq!(utf16_byte_offset(&hit.line_content, 1), None);
        assert_eq!(utf16_byte_offset(&hit.line_content, 99), None);
    }
}
