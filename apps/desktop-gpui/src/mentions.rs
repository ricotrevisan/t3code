//! "@" file mentions: the composer inserts a file link (`[name](path)`, as the web
//! composer does) and a turn sends each link as a `mention` context record.
use std::ops::Range;

use serde_json::{Value, json};

const MAX_LINK_LABEL_CHARS: usize = 512;
const MAX_LABEL_CHARS: usize = 200;
const MAX_PATH_CHARS: usize = 2_048;
const MAX_RECORDS: usize = 200;

/// The "@query" being typed just before `cursor`: its byte range and the query.
pub fn path_trigger(text: &str, cursor: usize) -> Option<(Range<usize>, &str)> {
    let prefix = text.get(..cursor)?;
    let start = prefix
        .char_indices()
        .rfind(|(_, char)| char.is_whitespace())
        .map_or(0, |(index, char)| index + char.len_utf8());
    let query = prefix[start..].strip_prefix('@')?;
    (!query.is_empty()).then_some((start..cursor, query))
}

/// The composer token for a workspace path, byte for byte what the web's
/// `serializeComposerFileLink` writes.
pub fn file_link(path: &str) -> String {
    let label = basename(path)
        .replace('\\', "\\\\")
        .replace('[', "\\[")
        .replace(']', "\\]");
    let mut destination = String::new();
    for byte in path.bytes() {
        if byte.is_ascii_alphanumeric() || b"-_.!~*';,/:@&=+$".contains(&byte) {
            destination.push(byte as char);
        } else {
            destination.push_str(&format!("%{byte:02X}"));
        }
    }
    format!("[{label}]({destination})")
}

/// The text and `message.context` a turn sends for `text`. Every file link the
/// web composer would chip becomes a canonical `t3-context` reference to a
/// `mention` record; anything else, such as a link whose label no longer names
/// its file, stays plain text.
pub fn message_context(text: &str) -> (String, Option<Value>) {
    let mut out = String::with_capacity(text.len());
    let mut paths: Vec<String> = Vec::new();
    let mut records = Vec::new();
    let mut cursor = 0;
    let mut index = 0;
    while let Some(offset) = text[index..].find('[') {
        let start = index + offset;
        let boundary = text[..start]
            .chars()
            .next_back()
            .is_none_or(char::is_whitespace);
        let link = boundary.then(|| file_link_at(text, start)).flatten();
        let Some((end, path)) = link else {
            index = start + 1;
            continue;
        };
        index = end;
        let label = sanitize_label(basename(&path));
        let number = match paths.iter().position(|known| known == &path) {
            Some(position) => position + 1,
            None if paths.len() < MAX_RECORDS => {
                paths.push(path.clone());
                records.push(json!({
                    "version": 1,
                    "kind": "mention",
                    "contextId": format!("mention_{}", paths.len()),
                    "label": label,
                    "path": path,
                }));
                paths.len()
            }
            None => continue,
        };
        out.push_str(&text[cursor..start]);
        out.push_str(&format!(
            "[{label}](t3-context://v1/mention/mention_{number})"
        ));
        cursor = end;
    }
    out.push_str(&text[cursor..]);
    let context = (!records.is_empty()).then(|| json!({"version": 1, "records": records}));
    (out, context)
}

/// The file link starting at `start` (a `[`), as its end and decoded path, when
/// its label is the path's basename as the composer wrote it.
fn file_link_at(text: &str, start: usize) -> Option<(usize, String)> {
    let mut chars = text[start + 1..].char_indices();
    let mut label = String::new();
    let mut units = 0;
    let label_end = loop {
        let (offset, char) = chars.next()?;
        units += 1;
        if units > MAX_LINK_LABEL_CHARS + 1 {
            return None;
        }
        match char {
            ']' => break start + 1 + offset + 1,
            '\\' => match chars.next()? {
                (_, '\n' | '\r') => return None,
                (_, escaped) => label.push(escaped),
            },
            _ => label.push(char),
        }
    };
    let rest = text[label_end..].strip_prefix('(')?;
    let close = rest.find(|char: char| char == ')' || char.is_whitespace())?;
    if close == 0 || !rest[close..].starts_with(')') {
        return None;
    }
    let end = label_end + 1 + close + 1;
    if text[end..]
        .chars()
        .next()
        .is_some_and(|char| !char.is_whitespace())
    {
        return None;
    }
    let encoded = &rest[..close];
    let path = percent_decode(encoded).unwrap_or_else(|| encoded.to_owned());
    let path_ok = !path.is_empty()
        && path.trim() == path
        && path.chars().count() <= MAX_PATH_CHARS
        && !has_external_scheme(&path);
    (path_ok && label == basename(&path)).then_some((end, path))
}

fn basename(path: &str) -> &str {
    path.rfind(['/', '\\'])
        .map_or(path, |index| &path[index + 1..])
}

fn has_external_scheme(path: &str) -> bool {
    let bytes = path.as_bytes();
    let drive = bytes.len() > 2
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && matches!(bytes[2], b'/' | b'\\');
    let scheme = path.split_once(':').is_some_and(|(scheme, _)| {
        scheme.starts_with(|char: char| char.is_ascii_alphabetic())
            && scheme
                .chars()
                .all(|char| char.is_ascii_alphanumeric() || matches!(char, '+' | '.' | '-'))
    });
    scheme && !drive
}

/// `decodeURIComponent`; None when an escape is malformed.
fn percent_decode(encoded: &str) -> Option<String> {
    let bytes = encoded.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            let hex = encoded.get(index + 1..index + 3)?;
            decoded.push(u8::from_str_radix(hex, 16).ok()?);
            index += 3;
        } else {
            decoded.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8(decoded).ok()
}

/// The contract's label rules: no brackets, backslashes or line breaks, bounded, never empty.
fn sanitize_label(label: &str) -> String {
    let cleaned = label
        .replace(['[', ']', '\\', '\r', '\n'], " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let cleaned: String = cleaned.chars().take(MAX_LABEL_CHARS).collect();
    if cleaned.is_empty() {
        "mention".to_owned()
    } else {
        cleaned
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(number: usize, label: &str, path: &str) -> Value {
        json!({"version":1,"kind":"mention","contextId":format!("mention_{number}"),"label":label,"path":path})
    }

    #[test]
    fn one_mention_becomes_a_reference_and_a_record() {
        let text = format!("look at {} please", file_link("src/main.rs"));
        assert_eq!(text, "look at [main.rs](src/main.rs) please");
        let (sent, context) = message_context(&text);
        assert_eq!(
            sent,
            "look at [main.rs](t3-context://v1/mention/mention_1) please"
        );
        assert_eq!(
            context,
            Some(json!({"version":1,"records":[record(1, "main.rs", "src/main.rs")]}))
        );
    }

    #[test]
    fn two_mentions_get_one_record_each_and_a_repeat_shares_its_record() {
        let a = file_link("src/a b.ts");
        let b = file_link("docs/[draft].md");
        assert_eq!(a, "[a b.ts](src/a%20b.ts)");
        assert_eq!(b, "[\\[draft\\].md](docs/%5Bdraft%5D.md)");
        let (sent, context) = message_context(&format!("{a} and {b}\n{a} "));
        assert_eq!(
            sent,
            "[a b.ts](t3-context://v1/mention/mention_1) and \
             [draft .md](t3-context://v1/mention/mention_2)\n\
             [a b.ts](t3-context://v1/mention/mention_1) "
        );
        assert_eq!(
            context.unwrap()["records"],
            json!([
                record(1, "a b.ts", "src/a b.ts"),
                record(2, "draft .md", "docs/[draft].md")
            ])
        );
    }

    #[test]
    fn an_edited_token_stays_plain_text() {
        for text in [
            "see [main.rs.bak](src/main.rs) now",
            "see x[main.rs](src/main.rs) now",
            "see [main.rs](src/main.rs)x now",
            "see [site](https://example.com/site) now",
        ] {
            assert_eq!(message_context(text), (text.to_owned(), None), "{text}");
        }
    }

    #[test]
    fn a_prompt_without_mentions_is_unchanged() {
        for text in ["", "plain [brackets] and (parens) @src", "a [b](c d)"] {
            assert_eq!(message_context(text), (text.to_owned(), None));
        }
    }

    #[test]
    fn the_trigger_is_the_at_token_before_the_cursor() {
        assert_eq!(path_trigger("open @src", 9), Some((5..9, "src")));
        assert_eq!(path_trigger("@sr later", 3), Some((0..3, "sr")));
        assert_eq!(path_trigger("é\n@ü", 6), Some((3..6, "ü")));
        assert_eq!(path_trigger("open @", 6), None);
        assert_eq!(path_trigger("open @src ", 10), None);
        assert_eq!(path_trigger("mail a@b", 8), None);
    }
}
