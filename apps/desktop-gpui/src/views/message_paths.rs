//! Markdown file navigation reuses the inspector's guarded, checkout-bound reader.
use crate::NativeApp;
use gpui_kit::{Context, Window};

pub(crate) fn workspace_link(href: &str, cwd: &str) -> Option<(String, Option<u32>)> {
    let href = href.strip_prefix("file://").unwrap_or(href);
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
    let path = if href.starts_with('/') {
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
