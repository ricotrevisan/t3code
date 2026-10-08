//! Host-directory browsing always runs on the selected environment, not this device.
use crate::{InspectorSection, NativeApp};
use gpui_kit::{Context, Task, Window};
use serde_json::{Value, json};
use t3code_desktop_gpui::{commands, connection};

#[derive(Default)]
pub(crate) struct DirectoryBrowser {
    pub adding: bool,
    pub parent: Option<String>,
    pub entries: Vec<Value>,
    pub loading: bool,
    pub error: Option<String>,
    pub request: u64,
    pub generation: u64,
    pub task: Option<Task<()>>,
}
impl NativeApp {
    pub(crate) fn browse_project_directory(
        &mut self,
        path: Option<String>,
        cx: &mut Context<Self>,
    ) {
        let Some(sender) = self.commands.clone() else {
            let error = "Connect to an environment to browse its directories";
            self.directory_browser.error = Some(error.into());
            self.report_error(error, cx);
            cx.notify();
            return;
        };
        let path = path.unwrap_or_else(|| {
            let typed = self.workspace.read(cx).value();
            if typed.trim().is_empty() {
                "~".into()
            } else {
                typed.to_string()
            }
        });
        let path = directory_query(&path);
        self.directory_browser.request += 1;
        let request = self.directory_browser.request;
        let generation = self.generation;
        self.directory_browser.generation = generation;
        let scope = self.environment_scope.clone();
        self.directory_browser.loading = true;
        self.directory_browser.parent = None;
        self.directory_browser.entries.clear();
        self.directory_browser.error = None;
        self.directory_browser.task = Some(cx.spawn(async move |entity, cx| {
            let result =
                connection::call(&sender, "filesystem.browse", json!({"partialPath":path})).await;
            let _ = entity.update(cx, |app, cx| {
                if app.generation != generation
                    || app.environment_scope != scope
                    || app.directory_browser.request != request
                {
                    return;
                }
                app.directory_browser.loading = false;
                match result {
                    Ok(value) => {
                        app.directory_browser.parent =
                            value["parentPath"].as_str().map(str::to_owned);
                        app.directory_browser.entries =
                            value["entries"].as_array().cloned().unwrap_or_default();
                        if app.directory_browser.parent.is_none() {
                            app.directory_browser.error =
                                Some("Invalid directory browser response".into());
                        }
                    }
                    Err(error) => app.directory_browser.error = Some(error),
                }
                if let Some(error) = app.directory_browser.error.clone() {
                    app.report_error(error, cx);
                }
                cx.notify();
            });
        }));
        cx.notify();
    }
    pub(crate) fn open_add_project(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.directory_browser.adding = true;
        self.title
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.workspace
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.reveal_inspector(InspectorSection::Project, window, cx);
        self.browse_project_directory(None, cx);
    }
    pub(crate) fn create_project_from_fields(&mut self, cx: &mut Context<Self>) {
        if self.pending_actions > 0 {
            self.set_status("Wait for the pending project receipt", cx);
            cx.notify();
            return;
        }
        match commands::create_project(
            &self.title.read(cx).value(),
            &self.workspace.read(cx).value(),
        ) {
            Ok(command) => self.dispatch(command, cx),
            Err(error) => {
                self.report_error(error, cx);
                cx.notify();
            }
        }
    }
}
pub(crate) fn parent_directory(path: &str) -> Option<String> {
    let windows = path.contains('\\');
    let separator = if windows { '\\' } else { '/' };
    let trimmed = path.trim_end_matches(separator);
    let (parent, _) = trimmed.rsplit_once(separator)?;
    if parent.is_empty() {
        Some(separator.to_string())
    } else if windows && parent.ends_with(':') {
        Some(format!("{parent}{separator}"))
    } else {
        Some(parent.into())
    }
}

fn directory_query(path: &str) -> String {
    if path == "~" || path.ends_with(['/', '\\']) {
        path.into()
    } else if path.contains('\\') {
        format!("{path}\\")
    } else {
        format!("{path}/")
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn directory_navigation_requests_children_not_sibling_prefix_matches() {
        assert_eq!(directory_query("/repo"), "/repo/");
        assert_eq!(directory_query("/"), "/");
        assert_eq!(directory_query("~"), "~");
        assert_eq!(parent_directory("/repo"), Some("/".into()));
        assert_eq!(parent_directory("/"), None);
        assert_eq!(parent_directory("C:\\work"), Some("C:\\".into()));
        assert_eq!(parent_directory("C:\\"), None);
        assert_eq!(directory_query("C:\\work"), "C:\\work\\");
    }
}
