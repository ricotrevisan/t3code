//! Project actions resolve through the authoritative server settings layers.
use serde_json::{Value, json};

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
