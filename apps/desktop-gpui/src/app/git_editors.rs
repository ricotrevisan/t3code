use crate::NativeApp;
use gpui_kit::component::{
    ActiveTheme as _, Disableable, Sizable,
    button::{Button, ButtonVariants},
};
use gpui_kit::{Context, div, prelude::*};
/// Mirrors contracts/editor.ts buildRemoteOpenUrl; encode segments, not slashes.
pub(crate) fn remote_editor_url(editor: &str, host: &str, path: &str) -> Option<String> {
    let scheme = match editor {
        "zed" => "zed",
        "vscode" => "vscode",
        "cursor" => "cursor",
        "vscode-insiders" => "vscode-insiders",
        "vscodium" => "vscodium",
        _ => return None,
    };
    if host.trim().is_empty() || path.trim().is_empty() {
        return None;
    }
    let windows_c = path.as_bytes().get(1) == Some(&b':')
        && matches!(path.as_bytes().first(), Some(b'C' | b'c'))
        && matches!(path.as_bytes().get(2), Some(b'/' | b'\\'));
    let mut path = path.replace('\\', "/");
    if !path.starts_with('/') {
        path.insert(0, '/');
    }
    if editor == "zed" && windows_c {
        path = path[3..].to_owned();
    }
    let encoded_path = path
        .split('/')
        .map(encode_segment)
        .collect::<Vec<_>>()
        .join("/");
    let host = encode_segment(host);
    Some(if editor == "zed" {
        format!("{scheme}://ssh/{host}{encoded_path}")
    } else {
        format!("{scheme}://vscode-remote/ssh-remote+{host}{encoded_path}")
    })
}
fn encode_segment(value: &str) -> String {
    value
        .as_bytes()
        .iter()
        .map(|b| {
            if b.is_ascii_alphanumeric() || b"-_.!~*'()".contains(b) {
                char::from(*b).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect()
}
#[derive(Default)]
pub(crate) struct EditorChoice {
    pub open: bool,
    pub preferred: Option<String>,
}
impl EditorChoice {
    pub(crate) fn restore() -> Self {
        #[cfg(test)]
        let preferred = None;
        #[cfg(not(test))]
        let preferred = std::fs::read_to_string(
            t3code_desktop_gpui::drafts::default_path().with_file_name("preferred-editor.txt"),
        )
        .ok();
        Self {
            preferred,
            open: false,
        }
    }
}
impl NativeApp {
    pub(crate) fn open_diff_editor(&mut self, cx: &mut Context<Self>) {
        if !self.connection_ready() {
            return;
        }
        let Some(file) = self.diffs.files.get(self.diffs.selected_file) else {
            return;
        };
        let path = file.path.clone();
        self.open_checkout_file_editor(&path, cx);
    }
    pub(crate) fn open_git_form_file_editor(&mut self, path: &str, cx: &mut Context<Self>) {
        if self.git_header.context.as_ref() != Some(&self.checkout_context())
            || self
                .git_header
                .form
                .as_ref()
                .is_none_or(|form| !form.paths.iter().any(|p| p == path))
        {
            self.report_error(
                "Checkout changed; reopen the commit form before opening a file",
                cx,
            );
            return;
        }
        self.open_checkout_file_editor(path, cx);
    }
    pub(crate) fn open_checkout_file_editor(
        &mut self,
        relative_path: &str,
        cx: &mut Context<Self>,
    ) {
        if !self.connection_ready() {
            return;
        }
        let (Some(cwd), Some(editor)) = (self.selected_workspace(), self.preferred_editor()) else {
            return;
        };
        if let Some(reason) = self.editor_unavailable_reason() {
            self.report_error(reason, cx);
            return;
        }
        if relative_path.is_empty()
            || relative_path.starts_with(['/', '\\'])
            || relative_path.contains(':')
            || relative_path.split(['/', '\\']).any(|part| part == "..")
        {
            self.report_error("Diff path is outside the selected checkout", cx);
            return;
        }
        let path = format!("{}/{}", cwd.trim_end_matches(['/', '\\']), relative_path);
        if self.is_remote_editor_checkout() {
            if let Some(host) = self.remote_editor_host()
                && let Some(link) = remote_editor_url(&editor, host, &path)
            {
                cx.open_url(&link);
            } else {
                self.report_error("Remote SSH editor route is unavailable", cx);
            }
        } else if self.editor_unavailable_reason().is_none() {
            self.perform(
                "shell.openInEditor",
                serde_json::json!({"cwd":path,"editor":editor}),
                None,
                cx,
            );
        }
    }
    pub(crate) fn editor_unavailable_reason(&self) -> Option<&'static str> {
        let remote = self.is_remote_editor_checkout();
        if remote && self.remote_editor_host().is_none() {
            Some(
                "Remote environment has not advertised an SSH host. Server-side editor launch is disabled.",
            )
        } else if !remote && self.available_editors.is_empty() {
            Some("No available editors or file managers reported by the server")
        } else {
            None
        }
    }
    pub(crate) fn is_remote_editor_checkout(&self) -> bool {
        url::Url::parse(&self.environment_scope)
            .ok()
            .and_then(|url| url.host_str().map(str::to_owned))
            .is_some_and(|host| {
                host != "localhost"
                    && host
                        .parse::<std::net::IpAddr>()
                        .ok()
                        .is_none_or(|ip| !ip.is_loopback())
            })
    }
    pub(crate) fn remote_editor_host(&self) -> Option<&str> {
        self.server_config.config(self.generation)?["remoteOpenTargets"]
            .as_array()?
            .iter()
            .find_map(|target| {
                target["host"]
                    .as_str()
                    .filter(|host| !host.trim().is_empty())
            })
    }
    pub(crate) fn effective_editors(&self) -> Vec<String> {
        if self.is_remote_editor_checkout() {
            vec!["vscode".into(), "zed".into()]
        } else {
            self.available_editors.clone()
        }
    }
    pub(crate) fn preferred_editor(&self) -> Option<String> {
        self.editor_choice
            .preferred
            .as_ref()
            .filter(|editor| self.effective_editors().contains(editor))
            .cloned()
            .or_else(|| {
                self.effective_editors()
                    .iter()
                    .find(|editor| editor.as_str() == "zed")
                    .cloned()
            })
            .or_else(|| self.effective_editors().first().cloned())
    }
    pub(crate) fn select_editor(&mut self, editor: String, cx: &mut Context<Self>) {
        if !self.effective_editors().contains(&editor) || self.editor_unavailable_reason().is_some()
        {
            return;
        }
        self.editor_choice.preferred = Some(editor.clone());
        self.editor_choice.open = false;
        #[cfg(not(test))]
        {
            let path =
                t3code_desktop_gpui::drafts::default_path().with_file_name("preferred-editor.txt");
            if let Err(error) = super::checkout_operations::persist(&path, editor.as_bytes()) {
                self.report_error(format!("Could not save preferred editor: {error}"), cx);
            }
        }
        self.open_in_editor(cx);
    }
    pub(crate) fn render_editor_choices(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let reason = self.editor_unavailable_reason();
        div()
            .flex()
            .flex_col()
            .gap_1()
            .p_2()
            .bg(cx.theme().popover)
            .when_some(reason, |view, reason| {
                view.child(div().text_xs().child(reason))
            })
            .when(self.is_remote_editor_checkout() && reason.is_none(), |view| view.child(div().text_xs().child("Opens the editor on this Mac over SSH. Install its URL handler and configure SSH keys here.")))
            .children(self.effective_editors().iter().map(|editor| {
                let editor = editor.clone();
                Button::new(gpui_kit::SharedString::from(format!("open-in-{editor}")))
                    .ghost()
                    .small()
                    .label(editor.clone())
                    .disabled(reason.is_some())
                    .on_click(
                        cx.listener(move |app, _, _, cx| app.select_editor(editor.clone(), cx)),
                    )
            }))
    }
}
