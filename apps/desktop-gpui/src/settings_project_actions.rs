//! Checked-in actions are read on the host; imports are explicit settings writes.
use crate::NativeApp;
use gpui_kit::{Context, Task};
use serde_json::{Value, json};
use t3code_desktop_gpui::connection;

pub const ICONS: [&str; 6] = ["play", "test", "lint", "configure", "build", "debug"];
pub(crate) struct ActionExtras {
    pub icon: String,
    pub scripts: Vec<Value>,
    pub error: Option<String>,
    pub target: Option<(u64, String)>,
    pub task: Option<Task<()>>,
    pub loading: bool,
}
impl Default for ActionExtras {
    fn default() -> Self {
        Self {
            icon: "play".into(),
            scripts: vec![],
            error: None,
            target: None,
            task: None,
            loading: false,
        }
    }
}
impl NativeApp {
    pub(crate) fn load_checked_in_actions(&mut self, cx: &mut Context<Self>) {
        let Some(project) = self
            .selected_project
            .as_ref()
            .and_then(|id| self.shell.projects.iter().find(|project| &project.id == id))
        else {
            self.report_error("Select a project first", cx);
            return;
        };
        let Some(sender) = self.commands.clone() else {
            self.report_error("Not connected; cannot read t3.json", cx);
            return;
        };
        let cwd = project.workspace_root.clone();
        let target = (self.generation, project.id.clone());
        self.action_extras.target = Some(target.clone());
        self.action_extras.scripts.clear();
        self.action_extras.error = None;
        self.action_extras.loading = true;
        self.action_extras.task = Some(cx.spawn(async move |entity, cx| {
            let result = connection::call(
                &sender,
                "projects.readFile",
                json!({"cwd":cwd,"relativePath":"t3.json"}),
            )
            .await;
            let _ = entity.update(cx, |app, cx| {
                if app.generation != target.0
                    || app.selected_project.as_ref() != Some(&target.1)
                    || app.action_extras.target.as_ref() != Some(&target)
                {
                    return;
                }
                app.action_extras.loading = false;
                match result {
                    Ok(read) if read["truncated"].as_bool() == Some(true) => {
                        app.action_extras.error =
                            Some("t3.json is too large to import safely".into())
                    }
                    Ok(read) => {
                        match parse_scripts(read["contents"].as_str().unwrap_or_default()) {
                            Ok(scripts) => app.action_extras.scripts = scripts,
                            Err(error) => app.action_extras.error = Some(error),
                        }
                    }
                    Err(error) => {
                        app.action_extras.error = Some(format!("Could not read t3.json: {error}"))
                    }
                }
                if let Some(error) = app.action_extras.error.clone() {
                    app.report_error(error, cx);
                }
                cx.notify();
            });
        }));
        cx.notify();
    }
}

pub fn parse_scripts(contents: &str) -> Result<Vec<Value>, String> {
    let file: Value =
        serde_json::from_str(contents).map_err(|error| format!("Invalid t3.json: {error}"))?;
    if !file.is_object() {
        return Err("t3.json must contain an object".into());
    }
    for key in ["$schema", "iconPath"] {
        if let Some(value) = file.get(key)
            && (!value.is_string()
                || (key == "iconPath"
                    && (value.as_str().unwrap_or_default().trim().is_empty()
                        || value.as_str().unwrap_or_default().encode_utf16().count() > 512)))
        {
            return Err(format!("Invalid t3.json {key}"));
        }
    }
    for (key, choices) in [
        ("defaultThreadEnvMode", &["local", "worktree"][..]),
        (
            "worktreeSubmodules",
            &["recursive", "top-level", "none"][..],
        ),
    ] {
        if let Some(value) = file.get(key)
            && !value.as_str().is_some_and(|value| choices.contains(&value))
        {
            return Err(format!("Invalid t3.json {key}"));
        }
    }
    let Some(scripts) = file.get("scripts") else {
        return Ok(vec![]);
    };
    let scripts = scripts
        .as_array()
        .filter(|scripts| scripts.len() <= 50)
        .ok_or("t3.json scripts must be an array of at most 50 actions")?;
    let mut result = Vec::new();
    for (index, script) in scripts.iter().enumerate() {
        let invalid = || {
            format!(
                "Invalid t3.json action {}: check name, command, icon and options",
                index + 1
            )
        };
        if !script.is_object() {
            return Err(invalid());
        }
        let mut script = script.clone();
        for key in ["name", "command", "previewUrl"] {
            if let Some(value) = script.get(key) {
                let value = value
                    .as_str()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .ok_or_else(invalid)?;
                script[key] = value.into();
            } else if key != "previewUrl" {
                return Err(invalid());
            }
        }
        if let Some(icon) = script.get("icon")
            && !icon.as_str().is_some_and(|icon| ICONS.contains(&icon))
        {
            return Err(invalid());
        }
        for key in ["runOnWorktreeCreate", "async", "autoOpenPreview"] {
            if script.get(key).is_some_and(|value| !value.is_boolean()) {
                return Err(invalid());
            }
        }
        result.push(script);
    }
    Ok(result)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn checked_in_actions_validate_the_whole_file_before_import() {
        let scripts = parse_scripts(r#"{"scripts":[{"name":" Build ","command":" vp build ","icon":"build","previewUrl":"http://localhost:3000","autoOpenPreview":true}]}"#).unwrap();
        assert_eq!(scripts[0]["name"], "Build");
        assert_eq!(scripts[0]["command"], "vp build");
        assert_eq!(scripts[0]["autoOpenPreview"], true);
        assert!(parse_scripts(r#"{"scripts":[{"name":"Good","command":"x"},{"name":"Bad","command":"x","icon":"unknown"}]}"#).is_err());
        assert!(parse_scripts(r#"{"worktreeSubmodules":"bad","scripts":[]}"#).is_err());
        assert!(
            parse_scripts(r#"{"scripts":[{"name":"x","command":"x","async":"false"}]}"#).is_err()
        );
    }
}
