//! Construct only fork-owned client commands; the server remains the decider.
use std::collections::HashMap;

use chrono::{SecondsFormat, Utc};
use serde_json::{Value, json};

use crate::conversation::UserInputQuestion;
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

pub fn rename_project(project_id: &str, title: &str) -> Result<Value, &'static str> {
    if project_id.trim().is_empty() || title.trim().is_empty() {
        return Err("Select a project and enter its title");
    }
    Ok(
        json!({"type":"project.meta.update","commandId":id(),"projectId":project_id,"title":title.trim()}),
    )
}

pub fn remove_project(project_id: &str) -> Result<Value, &'static str> {
    if project_id.trim().is_empty() {
        return Err("Select a project first");
    }
    // No force flag: active sessions must be stopped explicitly before removal.
    Ok(json!({"type":"project.delete","commandId":id(),"projectId":project_id}))
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
    attachments: &[Value],
    selection: Value,
    runtime: &str,
    interaction: &str,
) -> Result<Value, &'static str> {
    if text.trim().is_empty() {
        return Err("A prompt is required");
    }
    let mut result = command("thread.turn.start", thread_id);
    result["message"] =
        json!({"messageId":id(),"role":"user","text":text,"attachments":attachments});
    result["modelSelection"] = selection;
    result["runtimeMode"] = json!(runtime);
    result["interactionMode"] = json!(interaction);
    result["createdAt"] = json!(now());
    Ok(result)
}

/// A `thread.runtime-mode.set` / `thread.interaction-mode.set` command. The
/// server validates the value; the client only names the field.
pub fn set_thread_mode(kind: &str, thread_id: &str, field: &str, value: &str) -> Value {
    let mut command = command(kind, thread_id);
    command[field] = json!(value);
    command["createdAt"] = json!(now());
    command
}

/// Restore the working tree to a completed turn's checkpoint.
pub fn revert_command(thread_id: &str, turn_count: u64) -> Value {
    let mut command = command("thread.checkpoint.revert", thread_id);
    command["turnCount"] = json!(turn_count);
    command["createdAt"] = json!(now());
    command
}

/// Answer a pending provider approval.
pub fn approval_response(thread_id: &str, request_id: &str, decision: &str) -> Value {
    let mut command = command("thread.approval.respond", thread_id);
    command["requestId"] = json!(request_id);
    command["decision"] = json!(decision);
    command["createdAt"] = json!(now());
    command
}

/// Answer a pending user-input request, keyed by question id as the web
/// does: custom text outranks selected options, a multi-select question
/// answers with a list, and every question needs an answer.
pub fn user_input_response(
    thread_id: &str,
    request_id: &str,
    questions: &[UserInputQuestion],
    selected: &HashMap<String, Vec<String>>,
    custom: &HashMap<String, String>,
) -> Result<Value, &'static str> {
    let mut answers = serde_json::Map::new();
    for question in questions {
        let text = custom
            .get(&question.id)
            .map(|text| text.trim())
            .filter(|text| question.allow_custom_answer && !text.is_empty());
        let chosen: Vec<&String> = selected
            .get(&question.id)
            .into_iter()
            .flatten()
            .filter(|value| {
                question
                    .options
                    .iter()
                    .any(|option| &option.value == *value)
            })
            .collect();
        let answer = match (text, question.multi_select) {
            (Some(text), _) => json!(text),
            (None, true) if !chosen.is_empty() => json!(chosen),
            (None, false) if !chosen.is_empty() => json!(chosen[0]),
            _ => return Err("Answer every question first"),
        };
        answers.insert(question.id.clone(), answer);
    }
    let mut command = command("thread.user-input.respond", thread_id);
    command["requestId"] = json!(request_id);
    command["answers"] = Value::Object(answers);
    command["createdAt"] = json!(now());
    Ok(command)
}

/// Close an async user-input request without answering it.
pub fn user_input_dismiss(thread_id: &str, request_id: &str) -> Value {
    let mut command = command("thread.user-input.dismiss", thread_id);
    command["requestId"] = json!(request_id);
    command["createdAt"] = json!(now());
    command
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::conversation::UserInputOption;

    fn question(id: &str, multi_select: bool, allow_custom_answer: bool) -> UserInputQuestion {
        let option = |value: &str| UserInputOption {
            label: value.to_uppercase(),
            description: String::new(),
            value: value.to_owned(),
        };
        UserInputQuestion {
            id: id.to_owned(),
            header: "Header".to_owned(),
            question: "Question?".to_owned(),
            options: vec![option("a"), option("b")],
            multi_select,
            allow_custom_answer,
        }
    }

    #[test]
    fn user_input_answers_are_keyed_by_question_id() {
        let questions = [question("one", false, false), question("many", true, true)];
        let mut selected = HashMap::from([
            ("one".to_owned(), vec!["b".to_owned()]),
            (
                "many".to_owned(),
                vec!["a".to_owned(), "gone".to_owned(), "b".to_owned()],
            ),
        ]);
        let mut custom = HashMap::from([("one".to_owned(), "ignored".to_owned())]);
        let response = user_input_response("t", "req", &questions, &selected, &custom).unwrap();
        assert_eq!(response["type"], "thread.user-input.respond");
        assert_eq!(response["threadId"], "t");
        assert_eq!(response["requestId"], "req");
        assert_eq!(response["answers"], json!({"one": "b", "many": ["a", "b"]}));
        assert!(response["createdAt"].is_string());

        custom.insert("many".to_owned(), "  my own  ".to_owned());
        let response = user_input_response("t", "req", &questions, &selected, &custom).unwrap();
        assert_eq!(
            response["answers"]["many"], "my own",
            "custom text outranks options"
        );

        selected.remove("one");
        assert!(user_input_response("t", "req", &questions, &selected, &custom).is_err());
    }

    #[test]
    fn user_input_dismiss_names_the_request() {
        let dismiss = user_input_dismiss("t", "req");
        assert_eq!(dismiss["type"], "thread.user-input.dismiss");
        assert_eq!(dismiss["threadId"], "t");
        assert_eq!(dismiss["requestId"], "req");
        assert!(dismiss["createdAt"].is_string());
        assert!(dismiss.get("answers").is_none());
    }
    #[test]
    fn empty_form_input_never_becomes_a_destructive_or_send_command() {
        assert!(create_project("", "/tmp").is_err());
        assert!(create_project("Title", "").is_err());
        assert!(create_thread("p", "Title", Value::Null).is_err());
        assert!(start_turn("t", " \n", &[], Value::Null, "full-access", "default").is_err());
    }
    #[test]
    fn project_updates_are_metadata_only_and_removal_never_forces_running_sessions() {
        let renamed = rename_project("p", " New name ").unwrap();
        assert_eq!(renamed["title"], "New name");
        assert!(renamed.get("workspaceRoot").is_none());
        let removed = remove_project("p").unwrap();
        assert_eq!(removed["type"], "project.delete");
        assert!(removed.get("force").is_none());
        assert!(rename_project("p", " ").is_err());
        assert!(remove_project("").is_err());
    }
    #[test]
    fn unchanged_model_retains_provider_options_but_switching_does_not_leak_them() {
        let previous = json!({"instanceId":"codex","model":"gpt-5.4","options":[{"id":"reasoningEffort","value":"high"}]});
        let unchanged = select_model(&previous, "codex", "gpt-5.4").unwrap();
        let command = start_turn("t", "Prompt", &[], unchanged, "full-access", "default").unwrap();
        assert_eq!(command["modelSelection"], previous);
        assert_eq!(
            select_model(&previous, "claudeAgent", "sonnet").unwrap(),
            json!({"instanceId":"claudeAgent","model":"sonnet"})
        );
        assert!(select_model(&previous, "", "model").is_err());
    }
    #[test]
    fn thread_mode_changes_carry_the_mode_and_a_timestamp() {
        let runtime = set_thread_mode(
            "thread.runtime-mode.set",
            "t",
            "runtimeMode",
            "approval-required",
        );
        assert_eq!(runtime["type"], "thread.runtime-mode.set");
        assert_eq!(runtime["threadId"], "t");
        assert_eq!(runtime["runtimeMode"], "approval-required");
        assert!(runtime["createdAt"].is_string());
        let interaction = set_thread_mode(
            "thread.interaction-mode.set",
            "t",
            "interactionMode",
            "plan",
        );
        assert_eq!(interaction["interactionMode"], "plan");
        assert!(interaction.get("runtimeMode").is_none());
    }

    #[test]
    fn revert_command_targets_a_checkpoint_turn() {
        let revert = revert_command("t", 3);
        assert_eq!(revert["type"], "thread.checkpoint.revert");
        assert_eq!(revert["threadId"], "t");
        assert_eq!(revert["turnCount"], 3);
        assert!(revert["createdAt"].is_string());
    }

    #[test]
    fn approval_response_carries_the_request_and_decision() {
        let response = approval_response("t", "req-1", "acceptAlways");
        assert_eq!(response["type"], "thread.approval.respond");
        assert_eq!(response["threadId"], "t");
        assert_eq!(response["requestId"], "req-1");
        assert_eq!(response["decision"], "acceptAlways");
        assert!(response["createdAt"].is_string());
    }

    #[test]
    fn command_ids_are_fresh_and_prompt_content_is_not_trimmed() {
        let selection = json!({"instanceId":"codex","model":"gpt-5.4"});
        let a = start_turn(
            "t",
            "  hello\n世界  ",
            &[],
            selection.clone(),
            "approval-required",
            "plan",
        )
        .unwrap();
        let b = start_turn("t", "hello", &[], selection, "approval-required", "plan").unwrap();
        assert_ne!(a["commandId"], b["commandId"]);
        assert_eq!(a["message"]["text"], "  hello\n世界  ");
        assert_eq!(a["runtimeMode"], "approval-required");
        assert_eq!(a["interactionMode"], "plan");
    }
}
