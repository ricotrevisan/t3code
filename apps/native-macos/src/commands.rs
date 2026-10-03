//! Construct only fork-owned client commands; the server remains the decider.
use chrono::{SecondsFormat, Utc};
use serde_json::{Value, json};
use uuid::Uuid;

pub fn id() -> String {
    Uuid::new_v4().to_string()
}
pub fn now() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)
}

pub fn command(kind: &str, thread_id: &str) -> Value {
    json!({"type":kind,"commandId":id(),"threadId":thread_id})
}

pub fn create_project(title: &str, workspace: &str) -> Result<Value, &'static str> {
    if title.trim().is_empty() || workspace.trim().is_empty() {
        return Err("Project title and workspace are required");
    }
    Ok(
        json!({"type":"project.create","commandId":id(),"projectId":id(),"title":title.trim(),"workspaceRoot":workspace.trim(),"createdAt":now()}),
    )
}

pub fn create_thread(
    project_id: &str,
    title: &str,
    selection: Value,
) -> Result<Value, &'static str> {
    if title.trim().is_empty()
        || selection["instanceId"]
            .as_str()
            .is_none_or(|s| s.trim().is_empty())
        || selection["model"]
            .as_str()
            .is_none_or(|s| s.trim().is_empty())
    {
        return Err("Thread title, provider instance and model are required");
    }
    Ok(
        json!({"type":"thread.create","commandId":id(),"threadId":id(),"projectId":project_id,"title":title.trim(),"modelSelection":selection,"runtimeMode":"full-access","interactionMode":"default","branch":null,"worktreePath":null,"createdAt":now()}),
    )
}

pub fn select_model(previous: &Value, instance: &str, model: &str) -> Result<Value, &'static str> {
    let instance = instance.trim();
    let model = model.trim();
    if instance.is_empty() || model.is_empty() {
        return Err("Provider instance and model are required");
    }
    if previous["instanceId"] == instance && previous["model"] == model {
        return Ok(previous.clone());
    }
    Ok(json!({"instanceId":instance,"model":model}))
}

pub fn start_turn(
    thread_id: &str,
    text: &str,
    selection: Value,
    runtime: &str,
    interaction: &str,
) -> Result<Value, &'static str> {
    if text.trim().is_empty() {
        return Err("A prompt is required");
    }
    let mut result = command("thread.turn.start", thread_id);
    result["message"] = json!({"messageId":id(),"role":"user","text":text,"attachments":[]});
    result["modelSelection"] = selection;
    result["runtimeMode"] = json!(runtime);
    result["interactionMode"] = json!(interaction);
    result["createdAt"] = json!(now());
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn empty_form_input_never_becomes_a_destructive_or_send_command() {
        assert!(create_project("", "/tmp").is_err());
        assert!(create_project("Title", "").is_err());
        assert!(create_thread("p", "Title", Value::Null).is_err());
        assert!(start_turn("t", " \n", Value::Null, "full-access", "default").is_err());
    }
    #[test]
    fn unchanged_model_retains_provider_options_but_switching_does_not_leak_them() {
        let previous = json!({"instanceId":"codex","model":"gpt-5.4","options":[{"id":"reasoningEffort","value":"high"}]});
        let unchanged = select_model(&previous, "codex", "gpt-5.4").unwrap();
        let command = start_turn("t", "Prompt", unchanged, "full-access", "default").unwrap();
        assert_eq!(command["modelSelection"], previous);
        assert_eq!(
            select_model(&previous, "claudeAgent", "sonnet").unwrap(),
            json!({"instanceId":"claudeAgent","model":"sonnet"})
        );
        assert!(select_model(&previous, "", "model").is_err());
    }
    #[test]
    fn command_ids_are_fresh_and_prompt_content_is_not_trimmed() {
        let selection = json!({"instanceId":"codex","model":"gpt-5.4"});
        let a = start_turn(
            "t",
            "  hello\n世界  ",
            selection.clone(),
            "approval-required",
            "plan",
        )
        .unwrap();
        let b = start_turn("t", "hello", selection, "approval-required", "plan").unwrap();
        assert_ne!(a["commandId"], b["commandId"]);
        assert_eq!(a["message"]["text"], "  hello\n世界  ");
        assert_eq!(a["runtimeMode"], "approval-required");
        assert_eq!(a["interactionMode"], "plan");
    }
}
