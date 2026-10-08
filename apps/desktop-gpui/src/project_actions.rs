//! Project actions resolve through the authoritative server settings layers.
use serde_json::{Value, json};

/// Explicit composer choices belong to one project on one environment page.
/// Passing this record while retargeting a draft cannot leak its permissions.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct DraftDefaultsOverride {
    pub environment: String,
    pub project: String,
    pub runtime_mode: Option<String>,
    pub workspace_mode: Option<String>,
    #[serde(default)]
    pub interaction_mode: Option<String>,
    #[serde(default)]
    pub run_setup_script: Option<bool>,
    #[serde(default)]
    pub start_from_origin: Option<bool>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ThreadDefaults {
    pub runtime_mode: String,
    pub workspace_mode: String,
    pub interaction_mode: String,
    pub run_setup_script: bool,
    pub start_from_origin: bool,
}

/// Same precedence as shared/projectSettings.ts. Missing settings are an error,
/// not permission to silently bootstrap a full-access thread. A caller without
/// t3.json must load it before using the workspace result when the server tier
/// is unset (the built-in local tier is returned for a confirmed absent file).
pub fn thread_defaults(
    settings: Option<&Value>,
    environment: &str,
    project: &str,
    legacy_workspace: Option<&str>,
    project_file: Option<&Value>,
    explicit: Option<&DraftDefaultsOverride>,
) -> Result<ThreadDefaults, String> {
    let settings = settings
        .filter(|v| v.is_object())
        .ok_or("Load server settings before creating a thread")?;
    let entry = &settings["projectSettingsOverrides"][project];
    let explicit =
        explicit.filter(|choice| choice.environment == environment && choice.project == project);
    let runtime = explicit
        .and_then(|choice| choice.runtime_mode.as_deref())
        .or_else(|| entry["defaultRuntimeMode"].as_str())
        .or_else(|| settings["defaultRuntimeMode"].as_str())
        .unwrap_or("full-access");
    if ![
        "approval-required",
        "auto-accept-edits",
        "auto",
        "full-access",
    ]
    .contains(&runtime)
    {
        return Err(format!("Unsupported runtime default: {runtime}"));
    }
    // A present null project tier defers to the file, not the environment.
    let workspace_tier = if let Some(value) = entry.get("defaultThreadEnvMode") {
        value.as_str()
    } else if settings["projectSettingsFolded"] != true && legacy_workspace.is_some() {
        legacy_workspace
    } else {
        settings["defaultThreadEnvMode"].as_str()
    };
    let workspace = explicit
        .and_then(|choice| choice.workspace_mode.as_deref())
        .or(workspace_tier)
        .or_else(|| project_file.and_then(|file| file["defaultThreadEnvMode"].as_str()))
        .unwrap_or("local");
    if !["local", "worktree"].contains(&workspace) {
        return Err(format!("Unsupported workspace default: {workspace}"));
    }
    let interaction = explicit.and_then(|c| c.interaction_mode.as_deref()).unwrap_or("default");
    if !["default", "plan"].contains(&interaction) { return Err(format!("Unsupported interaction default: {interaction}")); }
    Ok(ThreadDefaults {
        runtime_mode: runtime.into(),
        workspace_mode: workspace.into(),
        interaction_mode: interaction.into(),
        run_setup_script: explicit.and_then(|c| c.run_setup_script).unwrap_or(true),
        start_from_origin: explicit.and_then(|c| c.start_from_origin).unwrap_or_else(|| workspace == "worktree" && entry["newWorktreesStartFromOrigin"].as_bool().or_else(|| settings["newWorktreesStartFromOrigin"].as_bool()).unwrap_or(true)),
    })
}

pub fn resolve(settings: &Value, project: &str, legacy_scripts: &[Value]) -> Vec<Value> {
    if let Some(scripts) =
        settings["projectSettingsOverrides"][project]["defaultProjectScripts"].as_array()
    {
        return scripts.clone();
    }
    let defaults = || {
        settings["defaultProjectScripts"]
            .as_array()
            .cloned()
            .unwrap_or_default()
    };
    if settings["projectSettingsFolded"] == true {
        return defaults();
    }
    match settings["projectScriptOverrides"].get(project) {
        Some(Value::Null) => defaults(),
        Some(Value::Array(scripts)) => scripts.clone(),
        _ if !legacy_scripts.is_empty() => legacy_scripts.to_vec(),
        _ => defaults(),
    }
}

pub fn model_default(settings: Option<&Value>, project: &str, legacy: Option<&Value>) -> Value {
    let Some(settings) = settings else {
        return legacy.cloned().unwrap_or(Value::Null);
    };
    if let Some(value) = settings["projectSettingsOverrides"][project].get("defaultModelSelection")
    {
        return value.clone();
    }
    if settings["projectSettingsFolded"] != true
        && let Some(value) = legacy
    {
        return value.clone();
    }
    settings["defaultModelSelection"].clone()
}

pub fn model_patch(settings: &Value, project: &str, selection: Option<Value>) -> Value {
    let mut entry = settings["projectSettingsOverrides"][project]
        .as_object()
        .cloned()
        .unwrap_or_default();
    if let Some(selection) = selection {
        entry.insert("defaultModelSelection".into(), selection);
    } else {
        entry.remove("defaultModelSelection");
    }
    json!({"patch":{"projectSettingsOverrides":{project:entry}}})
}

pub fn patch(settings: &Value, project: &str, scripts: Option<Vec<Value>>) -> Value {
    let mut entry = settings["projectSettingsOverrides"][project]
        .as_object()
        .cloned()
        .unwrap_or_default();
    match scripts {
        Some(scripts) => {
            entry.insert("defaultProjectScripts".into(), Value::Array(scripts));
        }
        None => {
            entry.remove("defaultProjectScripts");
        }
    }
    json!({"patch":{"projectSettingsOverrides":{project:entry}}})
}

pub fn save(
    current: &[Value],
    previous_id: Option<&str>,
    id: &str,
    name: &str,
    command: &str,
    setup: bool,
    wait: bool,
) -> Result<Vec<Value>, &'static str> {
    if name.trim().is_empty() || command.trim().is_empty() {
        return Err("Action name and command are required");
    }
    if previous_id.is_some_and(|id| !current.iter().any(|s| s["id"] == id)) {
        return Err("Action was removed; refresh before editing");
    }
    let mut script = previous_id
        .and_then(|id| current.iter().find(|s| s["id"] == id))
        .cloned()
        .unwrap_or_else(|| json!({"id":id,"icon":"play"}));
    script["name"] = name.trim().into();
    script["command"] = command.trim().into();
    script["runOnWorktreeCreate"] = setup.into();
    script["async"] = (!wait).into();
    let mut scripts = current.to_vec();
    if setup {
        for s in &mut scripts {
            s["runOnWorktreeCreate"] = false.into();
        }
    }
    if let Some(index) = previous_id.and_then(|id| scripts.iter().position(|s| s["id"] == id)) {
        scripts[index] = script;
    } else {
        scripts.push(script);
    }
    Ok(scripts)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn r16_f_defaults_project_environment_explicit_and_retarget() {
        let settings = json!({"defaultRuntimeMode":"approval-required","defaultThreadEnvMode":"worktree","projectSettingsFolded":true,"projectSettingsOverrides":{"p":{"defaultRuntimeMode":"auto-accept-edits","defaultThreadEnvMode":"local"}}});
        let resolve = |environment, project, explicit| {
            thread_defaults(
                Some(&settings),
                environment,
                project,
                Some("worktree"),
                None,
                explicit,
            )
            .unwrap()
        };
        assert_eq!(
            resolve("office", "other", None),
            ThreadDefaults {
                runtime_mode: "approval-required".into(),
                workspace_mode: "worktree".into(),
                interaction_mode: "default".into(),
                run_setup_script: true,
                start_from_origin: true,
            }
        );
        assert_eq!(
            resolve("office", "p", None),
            ThreadDefaults {
                runtime_mode: "auto-accept-edits".into(),
                workspace_mode: "local".into(),
                interaction_mode: "default".into(),
                run_setup_script: true,
                start_from_origin: false,
            }
        );
        let choice = DraftDefaultsOverride {
            environment: "office".into(),
            project: "p".into(),
            runtime_mode: Some("full-access".into()),
            workspace_mode: Some("worktree".into()),
            ..Default::default()
        };
        assert_eq!(
            resolve("office", "p", Some(&choice)).runtime_mode,
            "full-access"
        );
        assert_eq!(
            resolve("office", "other", Some(&choice)).runtime_mode,
            "approval-required"
        );
        assert_eq!(
            resolve("lab", "p", Some(&choice)).runtime_mode,
            "auto-accept-edits"
        );
        assert!(thread_defaults(None, "office", "p", None, None, None).is_err());
    }

    #[test]
    fn r16_f_defaults_folded_legacy_file_null_and_invalid_modes() {
        let mut settings = json!({"defaultRuntimeMode":"approval-required","defaultThreadEnvMode":"worktree","projectSettingsOverrides":{}});
        let file = json!({"defaultThreadEnvMode":"worktree"});
        assert_eq!(
            thread_defaults(Some(&settings), "e", "p", Some("local"), Some(&file), None)
                .unwrap()
                .workspace_mode,
            "local"
        );
        settings["projectSettingsFolded"] = json!(true);
        settings["projectSettingsOverrides"]["p"] = json!({"defaultThreadEnvMode":null});
        assert_eq!(
            thread_defaults(Some(&settings), "e", "p", Some("local"), Some(&file), None)
                .unwrap()
                .workspace_mode,
            "worktree"
        );
        assert_eq!(
            thread_defaults(Some(&settings), "e", "p", None, None, None)
                .unwrap()
                .workspace_mode,
            "local"
        );
        settings["defaultRuntimeMode"] = json!("invalid");
        assert!(thread_defaults(Some(&settings), "e", "p", None, None, None).is_err());
        settings["defaultRuntimeMode"] = json!("approval-required");
        settings["projectSettingsOverrides"]["p"]["defaultThreadEnvMode"] = json!("remote");
        assert!(thread_defaults(Some(&settings), "e", "p", None, None, None).is_err());
    }

    #[test]
    fn model_defaults_resolve_null_inheritance_legacy_and_preserve_other_overrides() {
        let settings = json!({"defaultModelSelection":{"instanceId":"codex","model":"global"},"projectSettingsFolded":true,"projectSettingsOverrides":{"p":{"defaultProjectScripts":[{"id":"owned"}],"defaultModelSelection":null}}});
        let legacy = json!({"instanceId":"codex","model":"legacy"});
        assert_eq!(
            model_default(Some(&settings), "p", Some(&legacy)),
            Value::Null
        );
        assert_eq!(
            model_default(Some(&settings), "other", Some(&legacy))["model"],
            "global"
        );
        let mut unfolded = settings.clone();
        unfolded["projectSettingsFolded"] = false.into();
        assert_eq!(
            model_default(Some(&unfolded), "other", Some(&legacy)),
            legacy
        );
        let inherited = model_patch(&settings, "p", None);
        assert!(
            inherited["patch"]["projectSettingsOverrides"]["p"]
                .get("defaultModelSelection")
                .is_none()
        );
        assert_eq!(
            inherited["patch"]["projectSettingsOverrides"]["p"]["defaultProjectScripts"],
            settings["projectSettingsOverrides"]["p"]["defaultProjectScripts"]
        );
        let patch = model_patch(&settings, "p", Some(legacy.clone()));
        assert_eq!(
            patch["patch"]["projectSettingsOverrides"]["p"]["defaultProjectScripts"],
            settings["projectSettingsOverrides"]["p"]["defaultProjectScripts"]
        );
        assert_eq!(
            patch["patch"]["projectSettingsOverrides"]["p"]["defaultModelSelection"],
            legacy
        );
    }
    #[test]
    fn action_edit_preserves_preview_and_other_settings_and_replaces_setup() {
        let settings = json!({"projectSettingsFolded":true,"defaultProjectScripts":[{"id":"global"}],"projectSettingsOverrides":{"p":{"defaultRuntimeMode":"approval-required","defaultProjectScripts":[{"id":"a","name":"Old","command":"old","icon":"build","runOnWorktreeCreate":true},{"id":"b","name":"Preview","command":"serve","icon":"play","runOnWorktreeCreate":false,"previewUrl":"http://localhost:1234","autoOpenPreview":true}]}}});
        let scripts = save(
            &resolve(&settings, "p", &[]),
            Some("b"),
            "ignored",
            "New",
            "node setup.cjs",
            true,
            true,
        )
        .unwrap();
        assert_eq!(scripts[0]["runOnWorktreeCreate"], false);
        assert_eq!(scripts[1]["async"], false);
        assert_eq!(scripts[1]["previewUrl"], "http://localhost:1234");
        assert_eq!(scripts[1]["icon"], "play");
        let result = patch(&settings, "p", Some(scripts));
        assert_eq!(
            result["patch"]["projectSettingsOverrides"]["p"]["defaultRuntimeMode"],
            "approval-required"
        );
        let inherited = patch(&settings, "p", None);
        assert!(
            inherited["patch"]["projectSettingsOverrides"]["p"]
                .get("defaultProjectScripts")
                .is_none()
        );
        assert_eq!(
            inherited["patch"]["projectSettingsOverrides"]["p"]["defaultRuntimeMode"],
            "approval-required"
        );
    }
    #[test]
    fn empty_override_and_legacy_null_have_distinct_inheritance() {
        let settings = json!({"defaultProjectScripts":[{"id":"default"}],"projectSettingsOverrides":{"empty":{"defaultProjectScripts":[]}},"projectScriptOverrides":{"inherit":null,"legacy":[{"id":"old"}]}});
        assert!(resolve(&settings, "empty", &[json!({"id":"aggregate"})]).is_empty());
        assert_eq!(
            resolve(&settings, "inherit", &[json!({"id":"aggregate"})])[0]["id"],
            "default"
        );
        assert_eq!(resolve(&settings, "legacy", &[])[0]["id"], "old");
        assert!(save(&[], Some("removed"), "id", "name", "command", false, false).is_err());
    }
}
