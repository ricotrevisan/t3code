//! Per-thread wire projection. Unimplemented events fail closed rather than display stale history.
//! Preserve contract fields that this slice does not yet render at the adapter seam.
use serde_json::{Value, json};

// Re-selecting a thread replaces ThreadDetail; process-local revisions must
// not collide with that instance's cached timeline at the same wire sequence.
fn projection_revision() -> u64 {
    static REVISION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    REVISION.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

#[derive(Default)]
pub struct ThreadDetail {
    /// Local projection revision also changes for replacement snapshots at the same sequence.
    pub revision: u64,
    /// Contiguous latest-turn message updates since this cached revision.
    /// Snapshots, historical changes and other event kinds force full preparation.
    pub tail_change: Option<(u64, String)>,
    pub sequence: Option<u64>,
    pub thread: Option<std::sync::Arc<Value>>,
    pub synchronized: bool,
}

fn required<'a>(value: &'a Value, key: &str) -> Result<&'a Value, String> {
    value.get(key).ok_or_else(|| format!("Missing {key}"))
}

fn array_mut<'a>(value: &'a mut Value, key: &str) -> Result<&'a mut Vec<Value>, String> {
    value
        .get_mut(key)
        .and_then(Value::as_array_mut)
        .ok_or_else(|| format!("Invalid {key}"))
}

fn copy_fields(target: &mut Value, source: &Value, fields: &[&str]) {
    for key in fields {
        if let Some(value) = source.get(*key) {
            target[*key] = value.clone();
        }
    }
}

pub fn apply_event(thread: &Value, event: &Value) -> Result<Option<Value>, String> {
    let mut next = thread.clone();
    let kind = required(event, "type")?
        .as_str()
        .ok_or("Invalid event type")?;
    let payload = required(event, "payload")?;
    let occurred = required(event, "occurredAt")?;
    if let Some(id) = payload.get("threadId")
        && id != &thread["id"]
    {
        return Err("Event belongs to a different thread".into());
    }
    match kind {
        "project.created"
        | "project.meta-updated"
        | "project.deleted"
        | "thread.approval-response-requested"
        | "thread.user-input-response-requested"
        | "thread.checkpoint-revert-requested" => return Ok(Some(next)),
        "thread.deleted" => return Ok(None),
        "thread.created" => {
            return Err("Unexpected thread creation in an existing detail stream".into());
        }
        "thread.archived" => {
            next["archivedAt"] = required(payload, "archivedAt")?.clone();
            next["titleRegeneration"] = Value::Null;
        }
        "thread.unarchived" => next["archivedAt"] = Value::Null,
        "thread.settled" => {
            next["settledOverride"] = json!("settled");
            next["settledAt"] = required(payload, "settledAt")?.clone();
            next["unsettledAt"] = Value::Null;
            next["activeOrderKey"] = Value::Null;
        }
        "thread.unsettled" => {
            next["settledOverride"] = if payload["reason"] == "user" {
                json!("active")
            } else {
                Value::Null
            };
            next["settledAt"] = Value::Null;
            next["unsettledAt"] = if thread["settledOverride"] == "active" {
                thread["unsettledAt"].clone()
            } else {
                required(payload, "updatedAt")?.clone()
            };
        }
        "thread.snoozed" => copy_fields(&mut next, payload, &["snoozedUntil", "snoozedAt"]),
        "thread.unsnoozed" => {
            next["snoozedUntil"] = Value::Null;
            next["snoozedAt"] = Value::Null;
        }
        "thread.pinned" => copy_fields(&mut next, payload, &["pinnedAt", "pinOrderKey"]),
        "thread.unpinned" => {
            next["pinnedAt"] = Value::Null;
            next["pinOrderKey"] = Value::Null;
        }
        "thread.pin-reordered" => next["pinOrderKey"] = required(payload, "orderKey")?.clone(),
        "thread.auto-settle-set" => {
            next["autoSettleDisabledAt"] = required(payload, "autoSettleDisabledAt")?.clone()
        }
        "thread.meta-updated" => copy_fields(
            &mut next,
            payload,
            &[
                "title",
                "titleState",
                "titleRegeneration",
                "modelSelection",
                "branch",
                "worktreePath",
                "linkedPullRequest",
                "branchPullRequest",
                "activeOrderKey",
            ],
        ),
        "thread.runtime-mode-set" => {
            next["runtimeMode"] = required(payload, "runtimeMode")?.clone()
        }
        "thread.interaction-mode-set" => {
            next["interactionMode"] = required(payload, "interactionMode")?.clone()
        }
        "thread.turn-start-requested" => {
            copy_fields(
                &mut next,
                payload,
                &["modelSelection", "runtimeMode", "interactionMode"],
            );
            next["updatedAt"] = occurred.clone();
            return Ok(Some(next));
        }
        "thread.turn-interrupt-requested" => {
            let Some(turn_id) = payload.get("turnId") else {
                return Ok(Some(next));
            };
            if next["latestTurn"].is_null() || &next["latestTurn"]["turnId"] != turn_id {
                return Ok(Some(next));
            }
            next["latestTurn"]["state"] = json!("interrupted");
            for key in ["startedAt", "completedAt"] {
                if next["latestTurn"][key].is_null() {
                    next["latestTurn"][key] = required(payload, "createdAt")?.clone();
                }
            }
            next["updatedAt"] = occurred.clone();
            return Ok(Some(next));
        }
        "thread.message-sent" => {
            let id = required(payload, "messageId")?;
            let streaming = required(payload, "streaming")?
                .as_bool()
                .ok_or("Invalid streaming flag")?;
            let text = required(payload, "text")?
                .as_str()
                .ok_or("Invalid message text")?;
            let messages = array_mut(&mut next, "messages")?;
            if let Some(existing) = messages.iter_mut().find(|m| &m["id"] == id) {
                if streaming {
                    let old = existing["text"]
                        .as_str()
                        .ok_or("Invalid stored message text")?;
                    existing["text"] = json!(format!("{old}{text}"));
                } else if !text.is_empty() {
                    existing["text"] = json!(text);
                }
                existing["streaming"] = json!(streaming);
                copy_fields(existing, payload, &["turnId", "attachments", "context"]);
                if !streaming {
                    existing["updatedAt"] = required(payload, "updatedAt")?.clone();
                }
            } else {
                let mut message =
                    json!({"id":id,"turnId":payload.get("turnId").unwrap_or(&Value::Null)});
                copy_fields(
                    &mut message,
                    payload,
                    &[
                        "role",
                        "text",
                        "streaming",
                        "createdAt",
                        "updatedAt",
                        "attachments",
                        "context",
                    ],
                );
                messages.push(message);
            }
            let turn = payload.get("turnId").unwrap_or(&Value::Null);
            let running = !turn.is_null()
                && thread["session"]["status"] == "running"
                && &thread["session"]["activeTurnId"] == turn;
            let settles = !streaming && !running;
            if payload["role"] == "assistant"
                && !turn.is_null()
                && (thread["latestTurn"].is_null() || &thread["latestTurn"]["turnId"] == turn)
            {
                let matching = &thread["latestTurn"]["turnId"] == turn;
                let state = if !settles {
                    "running"
                } else {
                    match thread["latestTurn"]["state"].as_str() {
                        Some("interrupted") => "interrupted",
                        Some("error") => "error",
                        _ => "completed",
                    }
                };
                next["latestTurn"] = json!({
                    "turnId":turn, "state":state,
                    "requestedAt":if matching { thread["latestTurn"]["requestedAt"].clone() } else { payload["createdAt"].clone() },
                    "startedAt":if matching && !thread["latestTurn"]["startedAt"].is_null() { thread["latestTurn"]["startedAt"].clone() } else { payload["createdAt"].clone() },
                    "completedAt":if settles { payload["updatedAt"].clone() } else if matching { thread["latestTurn"]["completedAt"].clone() } else { Value::Null },
                    "assistantMessageId":id,
                });
            }
            if payload["role"] == "assistant" && !turn.is_null() {
                for checkpoint in array_mut(&mut next, "checkpoints")? {
                    if &checkpoint["turnId"] == turn {
                        checkpoint["assistantMessageId"] = id.clone();
                    }
                }
            }
            next["updatedAt"] = occurred.clone();
            return Ok(Some(next));
        }
        "thread.session-set" => {
            let session = required(payload, "session")?;
            let latest = &thread["latestTurn"];
            if session["status"] == "running" && !session["activeTurnId"].is_null() {
                let matching = latest["turnId"] == session["activeTurnId"];
                next["latestTurn"] = json!({
                    "turnId":session["activeTurnId"], "state":"running",
                    "requestedAt":if matching { latest["requestedAt"].clone() } else { session["updatedAt"].clone() },
                    "startedAt":if matching && !latest["startedAt"].is_null() { latest["startedAt"].clone() } else { session["updatedAt"].clone() },
                    "completedAt":null,
                    "assistantMessageId":if matching { latest["assistantMessageId"].clone() } else { Value::Null },
                });
            } else if !latest.is_null() && latest["state"] == "running" {
                let state = match session["status"].as_str() {
                    Some("idle" | "ready") => Some("completed"),
                    Some("error") => Some("error"),
                    Some("interrupted" | "stopped") => Some("interrupted"),
                    _ => None,
                };
                if let Some(state) = state {
                    next["latestTurn"]["state"] = json!(state);
                    next["latestTurn"]["completedAt"] = session["updatedAt"].clone();
                }
            }
            next["session"] = session.clone();
            next["updatedAt"] = occurred.clone();
            return Ok(Some(next));
        }
        "thread.session-stop-requested" => {
            if thread["session"].is_null() {
                return Ok(Some(next));
            }
            next["session"]["status"] = json!("stopped");
            next["session"]["activeTurnId"] = Value::Null;
            next["session"]["updatedAt"] = required(payload, "createdAt")?.clone();
            next["updatedAt"] = occurred.clone();
            return Ok(Some(next));
        }
        "thread.proposed-plan-upserted" => {
            let plan = required(payload, "proposedPlan")?;
            let plans = array_mut(&mut next, "proposedPlans")?;
            plans.retain(|entry| entry["id"] != plan["id"]);
            plans.push(plan.clone());
            plans.sort_by(|a, b| {
                string_key(a, "createdAt")
                    .cmp(string_key(b, "createdAt"))
                    .then_with(|| string_key(a, "id").cmp(string_key(b, "id")))
            });
            next["updatedAt"] = occurred.clone();
            return Ok(Some(next));
        }
        "thread.turn-diff-completed" => {
            let turn = required(payload, "turnId")?;
            let checkpoints = array_mut(&mut next, "checkpoints")?;
            if checkpoints
                .iter()
                .any(|entry| &entry["turnId"] == turn && entry["status"] != "missing")
                && payload["status"] == "missing"
            {
                return Ok(Some(thread.clone()));
            }
            let mut checkpoint = json!({});
            copy_fields(
                &mut checkpoint,
                payload,
                &[
                    "turnId",
                    "checkpointTurnCount",
                    "checkpointRef",
                    "status",
                    "files",
                    "assistantMessageId",
                    "completedAt",
                ],
            );
            checkpoints.retain(|entry| &entry["turnId"] != turn);
            checkpoints.push(checkpoint);
            checkpoints
                .sort_by_key(|entry| entry["checkpointTurnCount"].as_u64().unwrap_or(u64::MAX));
            let running = thread["session"]["status"] == "running"
                && &thread["session"]["activeTurnId"] == turn;
            if !running
                && (thread["latestTurn"].is_null() || &thread["latestTurn"]["turnId"] == turn)
            {
                next["latestTurn"] = json!({"turnId":turn,
                    "state": if thread["latestTurn"]["state"] == "interrupted" { "interrupted" } else if payload["status"] == "error" { "error" } else { "completed" },
                    "requestedAt":thread["latestTurn"].get("requestedAt").unwrap_or(&payload["completedAt"]),
                    "startedAt":if thread["latestTurn"]["startedAt"].is_null() { payload["completedAt"].clone() } else { thread["latestTurn"]["startedAt"].clone() },
                    "completedAt":payload["completedAt"], "assistantMessageId":payload["assistantMessageId"]});
            }
            next["updatedAt"] = occurred.clone();
            return Ok(Some(next));
        }
        "thread.reverted" => {
            let count = required(payload, "turnCount")?
                .as_u64()
                .ok_or("Invalid turn count")?;
            let checkpoints = array_mut(&mut next, "checkpoints")?;
            checkpoints.retain(|entry| {
                entry["checkpointTurnCount"]
                    .as_u64()
                    .is_some_and(|n| n <= count)
            });
            checkpoints
                .sort_by_key(|entry| entry["checkpointTurnCount"].as_u64().unwrap_or(u64::MAX));
            let turns: std::collections::HashSet<Value> = checkpoints
                .iter()
                .map(|entry| entry["turnId"].clone())
                .collect();
            let latest = checkpoints.last().cloned();
            let messages = array_mut(&mut next, "messages")?;
            let imported = |message: &Value| string_key(message, "id").starts_with("import:");
            let mut retained: std::collections::HashSet<Value> = messages
                .iter()
                .filter(|message| {
                    message["role"] == "system"
                        || imported(message)
                        || (!message["turnId"].is_null() && turns.contains(&message["turnId"]))
                })
                .map(|message| message["id"].clone())
                .collect();
            for role in ["user", "assistant"] {
                let retained_count = messages
                    .iter()
                    .filter(|message| {
                        message["role"] == role
                            && !imported(message)
                            && retained.contains(&message["id"])
                    })
                    .count();
                let mut fallback: Vec<_> = messages
                    .iter()
                    .filter(|message| {
                        message["role"] == role
                            && !retained.contains(&message["id"])
                            && (message["turnId"].is_null() || turns.contains(&message["turnId"]))
                    })
                    .collect();
                fallback.sort_by(|a, b| {
                    compare_date_time(string_key(a, "createdAt"), string_key(b, "createdAt"))
                        .then_with(|| string_key(a, "id").cmp(string_key(b, "id")))
                });
                for message in fallback
                    .into_iter()
                    .take((count as usize).saturating_sub(retained_count))
                {
                    retained.insert(message["id"].clone());
                }
            }
            messages.retain(|message| retained.contains(&message["id"]));
            for key in ["proposedPlans", "activities"] {
                array_mut(&mut next, key)?
                    .retain(|entry| entry["turnId"].is_null() || turns.contains(&entry["turnId"]));
            }
            next["latestTurn"] = latest.map(|checkpoint| json!({"turnId":checkpoint["turnId"],"state":if checkpoint["status"] == "error" { "error" } else { "completed" },"requestedAt":checkpoint["completedAt"],"startedAt":checkpoint["completedAt"],"completedAt":checkpoint["completedAt"],"assistantMessageId":checkpoint["assistantMessageId"]})).unwrap_or(Value::Null);
            next["updatedAt"] = occurred.clone();
            return Ok(Some(next));
        }
        "thread.activity-appended" => {
            let activity = required(payload, "activity")?;
            let activities = array_mut(&mut next, "activities")?;
            let supersedes = resolvable_context(activity);
            activities.retain(|entry| {
                entry["id"] != activity["id"]
                    && !(supersedes
                        && entry["turnId"] == activity["turnId"]
                        && resolvable_context(entry))
            });
            activities.push(activity.clone());
            activities.sort_by(|a, b| {
                a["sequence"]
                    .as_u64()
                    .unwrap_or(u64::MAX)
                    .cmp(&b["sequence"].as_u64().unwrap_or(u64::MAX))
                    .then_with(|| string_key(a, "createdAt").cmp(string_key(b, "createdAt")))
                    .then_with(|| string_key(a, "id").cmp(string_key(b, "id")))
            });
            next["updatedAt"] = occurred.clone();
            return Ok(Some(next));
        }
        _ => return Err(format!("Unsupported thread event: {kind}")),
    }
    next["updatedAt"] = required(payload, "updatedAt")?.clone();
    Ok(Some(next))
}

fn compare_date_time(left: &str, right: &str) -> std::cmp::Ordering {
    let parse =
        |value| chrono::DateTime::parse_from_rfc3339(value).map(|date| date.timestamp_millis());
    match (parse(left), parse(right)) {
        (Ok(left), Ok(right)) => left.cmp(&right),
        (Ok(_), Err(_)) => std::cmp::Ordering::Greater,
        (Err(_), Ok(_)) => std::cmp::Ordering::Less,
        (Err(_), Err(_)) => left.cmp(right),
    }
}

fn string_key<'a>(value: &'a Value, key: &str) -> &'a str {
    value[key].as_str().unwrap_or("")
}
fn resolvable_context(value: &Value) -> bool {
    value["kind"] == "context-window.updated"
        && value["payload"]["usedTokens"]
            .as_f64()
            .is_some_and(|n| n.is_finite() && n >= 0.)
}

impl ThreadDetail {
    pub fn apply(&mut self, selected_id: &str, item: Value) -> Result<(), String> {
        match item["kind"].as_str() {
            Some("snapshot") => {
                let snapshot = required(&item, "snapshot")?;
                let thread = required(snapshot, "thread")?;
                if thread["id"].as_str() != Some(selected_id) || !thread["messages"].is_array() {
                    return Err("Invalid thread snapshot".into());
                }
                let sequence = required(snapshot, "snapshotSequence")?
                    .as_u64()
                    .ok_or("Invalid snapshot sequence")?;
                self.thread = Some(std::sync::Arc::new(thread.clone()));
                self.tail_change = None;
                self.revision = projection_revision();
                self.sequence = Some(sequence);
                self.synchronized = false;
            }
            Some("synchronized") => {
                if self.sequence.is_none() {
                    return Err("Synchronization before snapshot".into());
                }
                self.synchronized = true;
            }
            Some("event") => {
                let event = required(&item, "event")?;
                let sequence = required(event, "sequence")?
                    .as_u64()
                    .ok_or("Invalid event sequence")?;
                let previous = self.sequence.ok_or("Event before snapshot")?;
                if sequence <= previous {
                    return Ok(());
                }
                if let Some(thread) = &self.thread {
                    let kind = event["type"].as_str().unwrap_or("");
                    let item = match kind {
                        "thread.activity-appended" => &event["payload"]["activity"],
                        "thread.proposed-plan-upserted" => &event["payload"]["proposedPlan"],
                        _ => &event["payload"],
                    };
                    let tail = item["turnId"].as_str().filter(|turn| {
                        matches!(
                            kind,
                            "thread.message-sent"
                                | "thread.activity-appended"
                                | "thread.proposed-plan-upserted"
                        ) && thread["latestTurn"]["turnId"].as_str() == Some(*turn)
                            && item["createdAt"]
                                .as_str()
                                .zip(thread["latestTurn"]["startedAt"].as_str())
                                .is_some_and(|(created, started)| {
                                    compare_date_time(created, started) != std::cmp::Ordering::Less
                                })
                    });
                    let change = tail.map(|turn| {
                        let base = self
                            .tail_change
                            .as_ref()
                            .filter(|(_, previous)| previous == turn)
                            .map_or(self.revision, |(base, _)| *base);
                        (base, turn.to_owned())
                    });
                    self.thread = apply_event(thread, event)?.map(std::sync::Arc::new);
                    self.tail_change = change;
                }
                self.sequence = Some(sequence);
                self.revision = projection_revision();
            }
            _ => return Err("Unsupported thread stream item".into()),
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "thread_tests.rs"]
mod tests;
