//! Terminal paths resolve on the environment, not the client filesystem.
use crate::NativeApp;
use gpui_kit::{Context, Window, component::input::Position};

pub(super) fn workspace_path(
    raw: &str,
    cwd: &str,
    workspace: &str,
) -> Option<(String, Option<Position>)> {
    let mut raw = raw.trim().trim_end_matches(':').to_owned();
    if raw.starts_with("file://") {
        let url = url::Url::parse(&raw).ok()?;
        if url.host_str().is_some_and(|host| host != "localhost") {
            return None;
        }
        let mut decoded = Vec::new();
        let mut bytes = url.path().bytes();
        while let Some(byte) = bytes.next() {
            if byte == b'%' {
                let high = char::from(bytes.next()?).to_digit(16)?;
                let low = char::from(bytes.next()?).to_digit(16)?;
                decoded.push((high * 16 + low) as u8);
            } else {
                decoded.push(byte);
            }
        }
        raw = String::from_utf8(decoded).ok()?;
        if raw.starts_with('/') && raw.as_bytes().get(2) == Some(&b':') {
            raw.remove(0);
        }
    }
    let mut numbers = Vec::new();
    while numbers.len() < 2 {
        let Some((path, suffix)) = raw.rsplit_once(':') else {
            break;
        };
        let Ok(n) = suffix.parse::<u32>() else {
            break;
        };
        if n == 0 {
            return None;
        }
        numbers.push(n);
        raw = path.to_owned();
    }
    let position = match numbers.as_slice() {
        [line] => Some(Position::new(line - 1, 0)),
        [column, line] => Some(Position::new(line - 1, column - 1)),
        _ => None,
    };
    let raw = raw.replace('\\', "/");
    let cwd = cwd.replace('\\', "/");
    let workspace = workspace.replace('\\', "/");
    let raw = if let Some(tail) = raw.strip_prefix("~/") {
        let home = ["/Users/", "/home/"].into_iter().find_map(|prefix| {
            let user = cwd.strip_prefix(prefix)?.split('/').next()?;
            Some(format!("{prefix}{user}"))
        })?;
        format!("{home}/{tail}")
    } else if raw.starts_with('/') || raw.as_bytes().get(1) == Some(&b':') {
        raw
    } else {
        format!("{cwd}/{raw}")
    };
    fn normalize(path: &str) -> Option<String> {
        if path.contains(['\0', '\n', '\r', '?', '#']) {
            return None;
        }
        let mut parts = Vec::new();
        for part in path.split('/') {
            match part {
                "" | "." => {}
                ".." => {
                    parts.pop()?;
                }
                _ => parts.push(part),
            }
        }
        Some(format!(
            "{}{}",
            if path.starts_with('/') { "/" } else { "" },
            parts.join("/")
        ))
    }
    let absolute = normalize(&raw)?;
    let workspace = normalize(&workspace)?;
    let relative = absolute.strip_prefix(&format!("{}/", workspace.trim_end_matches('/')))?;
    if relative.is_empty() || relative.contains(':') {
        return None;
    }
    Some((relative.to_owned(), position))
}

impl NativeApp {
    pub(super) fn terminal_open_path(
        &mut self,
        raw: &str,
        cwd: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(workspace) = self.selected_workspace() else {
            return;
        };
        let Some((path, position)) = workspace_path(raw, cwd, &workspace) else {
            self.report_error(
                "Terminal link is outside the selected workspace or is not a supported file path",
                cx,
            );
            return;
        };
        if !self.can_change_checkout(cx) || self.file_busy {
            return;
        }
        self.file_path
            .update(cx, |input, cx| input.set_value(path, window, cx));
        self.browser_panel
            .update(cx, |panel, cx| panel.deactivate(cx));
        self.right_tab = crate::RightTab::Inspector;
        self.set_inspector_open(true, cx);
        self.file_operation_at(false, position, window, cx);
    }
    pub(super) fn terminal_external_editor(
        &mut self,
        raw: &str,
        cwd: &str,
        cx: &mut Context<Self>,
    ) {
        if !self.connection_ready() {
            return;
        }
        let (Some(workspace), Some(editor)) = (self.selected_workspace(), self.preferred_editor())
        else {
            return;
        };
        let Some((path, _)) = workspace_path(raw, cwd, &workspace) else {
            return;
        };
        let path = format!("{}/{path}", workspace.trim_end_matches(['/', '\\']));
        if self.is_remote_editor_checkout() {
            if let Some(host) = self.remote_editor_host()
                && let Some(link) = crate::app::git_editors::remote_editor_url(&editor, host, &path)
            {
                cx.open_url(&link);
            } else {
                self.report_error("Remote SSH editor route is unavailable", cx);
            }
        } else if self.editor_unavailable_reason().is_none() {
            // LaunchEditorInput has no line/column fields. The native reader
            // supports positions; this existing external route opens the file.
            self.perform(
                "shell.openInEditor",
                serde_json::json!({"cwd":path,"editor":editor}),
                None,
                cx,
            );
        }
    }
    pub(super) fn terminal_preview_link(
        &mut self,
        url: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let origin = self.server.read(cx).value().to_string();
        let scope = self.environment_scope.clone();
        let opened = self.browser_panel.update(cx, |panel, cx| {
            panel.sync_scope(&scope, cx);
            panel.open_preview(
                url,
                &origin,
                t3code_desktop_gpui::surfaces::browser::model::Profile::Default,
                window,
                cx,
            )
        });
        if opened.is_some() {
            self.right_tab = crate::RightTab::Inspector;
            self.set_inspector_open(true, cx);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn r16_t_paths_resolve_terminal_cwd_positions_and_reject_outside_workspace() {
        let (path, position) =
            workspace_path("../src/main.rs:12:8:", "/repo/build", "/repo").unwrap();
        assert_eq!(path, "src/main.rs");
        assert_eq!(position, Some(Position::new(11, 7)));
        assert_eq!(workspace_path("/repo-other/file", "/repo", "/repo"), None);
        assert_eq!(workspace_path("../../secret", "/repo/build", "/repo"), None);
        assert_eq!(
            workspace_path("javascript:alert(1)/evil", "/repo", "/repo"),
            None
        );
        assert_eq!(
            workspace_path("file://other/repo/file", "/repo", "/repo"),
            None
        );
        assert_eq!(workspace_path("src/a.rs:0", "/repo", "/repo"), None);
        assert_eq!(
            workspace_path(
                "~/repo/src/a.rs",
                "/home/rico/repo/build",
                "/home/rico/repo"
            )
            .unwrap()
            .0,
            "src/a.rs"
        );
        assert_eq!(
            workspace_path("file:///repo/a%20b.rs", "/repo", "/repo")
                .unwrap()
                .0,
            "a b.rs"
        );
        assert_eq!(
            workspace_path("C:\\repo\\src\\a.rs:4:2", "C:\\repo", "C:\\repo")
                .unwrap()
                .0,
            "src/a.rs"
        );
    }
}
