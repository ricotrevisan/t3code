//! Markdown file navigation reuses the inspector's guarded, checkout-bound reader.
use crate::NativeApp;
use gpui_kit::{Context, Window};

pub(crate) fn workspace_link(href: &str, cwd: &str) -> Option<(String, Option<u32>)> {
    let href = if let Some(path) = href.strip_prefix("file://") {
        let path = path.strip_prefix("localhost").unwrap_or(path);
        if !path.starts_with('/') {
            return None;
        }
        path
    } else {
        href
    };
    let href = decode_path(href)?;
    let href = href.replace('\\', "/");
    let cwd = cwd.replace('\\', "/");
    let href = href.as_str();
    let (href, line) = if let Some((path, line)) = href.rsplit_once("#L") {
        (path, line.split('-').next()?.parse::<u32>().ok())
    } else if let Some((path, line)) = href.rsplit_once(':') {
        if let Ok(line) = line.parse::<u32>() {
            (path, Some(line))
        } else {
            (href, None)
        }
    } else {
        (href, None)
    };
    let drive_path = href.as_bytes().get(1) == Some(&b':') && href.as_bytes().get(2) == Some(&b'/');
    let href = if href.starts_with('/') && href.as_bytes().get(2) == Some(&b':') {
        &href[1..]
    } else {
        href
    };
    let path = if href.starts_with('/') || drive_path || href.as_bytes().get(1) == Some(&b':') {
        href.strip_prefix(cwd.trim_end_matches('/'))?
            .strip_prefix('/')?
    } else {
        href.strip_prefix("./").unwrap_or(href)
    };
    if path.is_empty()
        || path.contains(['\0', '\n', '\r', ':', '?', '#', '\\'])
        || path.split('/').any(|part| part == ".." || part.is_empty())
    {
        return None;
    }
    Some((path.to_owned(), line.filter(|line| *line > 0)))
}

fn decode_path(path: &str) -> Option<String> {
    let mut bytes = Vec::with_capacity(path.len());
    let mut iter = path.bytes();
    while let Some(byte) = iter.next() {
        if byte == b'%' {
            let a = char::from(iter.next()?).to_digit(16)?;
            let b = char::from(iter.next()?).to_digit(16)?;
            bytes.push((a * 16 + b) as u8);
        } else {
            bytes.push(byte);
        }
    }
    String::from_utf8(bytes).ok()
}

/// Only workspace-qualified inline code becomes a link. Fenced examples and
/// authored link labels remain literal; basename lookup needs the file index.
pub(crate) fn link_inline_workspace_paths(markdown: &str, cwd: &str) -> String {
    let mut output = String::with_capacity(markdown.len());
    let mut fence: Option<(char, usize)> = None;
    for line in markdown.split_inclusive('\n') {
        let trimmed = line.trim_start();
        let marker = trimmed.chars().next().unwrap_or(' ');
        let count = trimmed.chars().take_while(|ch| *ch == marker).count();
        if matches!(marker, '`' | '~') && count >= 3 {
            if let Some((open, len)) = fence {
                if marker == open && count >= len {
                    fence = None;
                }
            } else {
                fence = Some((marker, count));
            }
            output.push_str(line);
            continue;
        }
        if fence.is_some() || line.starts_with("    ") {
            output.push_str(line);
            continue;
        }
        let mut rest = line;
        while let Some(start) = rest.find('`') {
            output.push_str(&rest[..start]);
            let run = rest[start..].bytes().take_while(|ch| *ch == b'`').count();
            let delimiter = &rest[start..start + run];
            let after = &rest[start + run..];
            let Some(end) = after.find(delimiter) else {
                output.push_str(&rest[start..]);
                rest = "";
                break;
            };
            let code = &after[..end];
            let suffix = &after[end + run..];
            let path = code.trim();
            let already_linked = rest[..start].ends_with('[') || suffix.starts_with("](");
            if !already_linked
                && path.contains(['/', '\\'])
                && !path.contains(['<', '>', '`', '[', ']', '(', ')'])
                && workspace_link(path, cwd).is_some()
            {
                let destination = path.replace('\\', "/").replace(' ', "%20");
                output.push_str(&format!("[{delimiter}{code}{delimiter}](<{destination}>)"));
            } else {
                output.push_str(&rest[start..start + run + end + run]);
            }
            rest = suffix;
        }
        output.push_str(rest);
    }
    output
}

impl NativeApp {
    pub(crate) fn open_message_link(
        &mut self,
        href: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Ok(url) = url::Url::parse(href)
            && matches!(url.scheme(), "https" | "http" | "mailto")
        {
            cx.open_url(href);
            return;
        }
        let Some(cwd) = self.selected_workspace() else {
            self.report_error("Select a workspace to open this file link", cx);
            return;
        };
        let Some((path, line)) = workspace_link(href, &cwd) else {
            self.report_error("This link is not a file in the selected workspace", cx);
            return;
        };
        if self
            .file_editor
            .dirty(self.file_contents.read(cx).value().as_str())
        {
            self.report_error(
                "Save or discard unsaved file edits before opening this link",
                cx,
            );
            return;
        }
        self.file_path
            .update(cx, |input, cx| input.set_value(path, window, cx));
        self.right_tab = crate::RightTab::Inspector;
        self.set_inspector_open(true, cx);
        self.file_operation_at(
            false,
            line.map(|line| gpui_kit::component::input::Position::new(line - 1, 0)),
            window,
            cx,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn r13_d_workspace_paths_decode_windows_and_spaces_without_allowing_traversal() {
        assert_eq!(
            workspace_link(r"C:\work\src\main.rs:12", r"C:\work"),
            Some(("src/main.rs".into(), Some(12)))
        );
        assert_eq!(
            workspace_link("file:///C:/work/src/main.rs#L4", "C:/work"),
            Some(("src/main.rs".into(), Some(4)))
        );
        assert_eq!(
            workspace_link("src/my%20file.rs", "/work"),
            Some(("src/my file.rs".into(), None))
        );
        for path in [
            "src/%2e%2e/secret",
            "src/%00name",
            "file://otherhost/work/file",
            r"D:\other\file",
            "src/%ff.rs",
        ] {
            assert!(workspace_link(path, "/work").is_none(), "{path}");
        }
    }
    #[test]
    fn r13_d_inline_paths_link_but_fences_and_authored_links_stay_literal() {
        let text = "Read `src/main.rs:4` and `symbol`.\n```rs\n`src/main.rs`\n```\n[`src/main.rs`](other)\n";
        let linked = link_inline_workspace_paths(text, "/work");
        assert!(linked.contains("[`src/main.rs:4`](<src/main.rs:4>)"));
        assert!(linked.contains("```rs\n`src/main.rs`\n```"));
        assert!(linked.contains("[`src/main.rs`](other)"));
        assert!(linked.contains("`symbol`"));
    }
    #[test]
    fn conversation_fixe_links_resolve_only_within_the_selected_workspace() {
        assert_eq!(
            workspace_link("./src/main.rs#L12", "/work"),
            Some(("src/main.rs".into(), Some(12)))
        );
        assert_eq!(
            workspace_link("file:///work/src/main.rs:4", "/work"),
            Some(("src/main.rs".into(), Some(4)))
        );
        for path in [
            "../secret",
            "/workspace/secret",
            "javascript:alert(1)",
            "foo?token=x",
        ] {
            assert!(workspace_link(path, "/work").is_none());
        }
    }
}
