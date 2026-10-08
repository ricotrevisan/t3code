//! Markdown source editing matches web composer-list-continuation.ts.
use std::ops::Range;

pub fn list_continuation(text: &str, cursor: usize) -> Option<(Range<usize>, String)> {
    let prefix = text.get(..cursor)?;
    let start = prefix.rfind('\n').map_or(0, |i| i + 1);
    let end = text[cursor..].find('\n').map_or(text.len(), |i| cursor + i);
    let line = &text[start..end];
    let indent_len = line.len() - line.trim_start_matches([' ', '\t']).len();
    let indent = &line[..indent_len];
    let rest = &line[indent_len..];
    let (marker_len, next) =
        if rest.starts_with("- [ ]") || rest.starts_with("- [x]") || rest.starts_with("- [X]") {
            (5, format!("{indent}- [ ] "))
        } else if rest.starts_with(['-', '*', '+']) {
            (1, format!("{indent}{} ", &rest[..1]))
        } else {
            let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
            if digits == 0 || !matches!(rest.as_bytes().get(digits), Some(b'.' | b')')) {
                return None;
            }
            let next = rest[..digits]
                .parse::<u64>()
                .ok()
                .filter(|n| *n <= 9_007_199_254_740_991)
                .and_then(|n| n.checked_add(1))
                .map_or_else(|| rest[..digits].to_owned(), |n| format!("{n:0digits$}"));
            (
                digits + 1,
                format!("{indent}{next}{} ", &rest[digits..digits + 1]),
            )
        };
    if rest.len() > marker_len && !rest[marker_len..].starts_with([' ', '\t']) {
        return None;
    }
    let spaces =
        rest[marker_len..].len() - rest[marker_len..].trim_start_matches([' ', '\t']).len();
    let marker_end = start + indent_len + marker_len + spaces;
    if cursor < marker_end {
        return None;
    }
    if text[marker_end..end].trim().is_empty() {
        Some((start..cursor, String::new()))
    } else {
        Some((cursor..cursor, format!("\n{next}")))
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn r13_c_markdown_lists_continue_reset_tasks_and_exit_empty_items() {
        for (text, expected) in [
            ("- hello", "\n- "),
            ("  009) hello", "\n  010) "),
            ("- [x] done", "\n- [ ] "),
        ] {
            assert_eq!(list_continuation(text, text.len()).unwrap().1, expected);
        }
        assert_eq!(list_continuation("- ", 2), Some((0..2, String::new())));
        assert!(list_continuation("hello", 5).is_none());
        assert!(list_continuation("- item", 1).is_none());
        assert!(list_continuation("1.not a list", 12).is_none());
    }
}
