//! Pure formatting for the thread conversation surface.
//!
//! Kept free of GPUI so the timeline rules are testable without a window.

use chrono::{DateTime, Utc};
use serde_json::Value;

/// The state of the thread's latest turn, as the timeline presents it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TurnState {
    Running,
    Completed,
    Interrupted,
    Error,
}

impl TurnState {
    pub fn parse(value: &str) -> Self {
        match value {
            "running" => Self::Running,
            "interrupted" => Self::Interrupted,
            "error" => Self::Error,
            _ => Self::Completed,
        }
    }

    pub fn is_danger(self) -> bool {
        matches!(self, Self::Error | Self::Interrupted)
    }
}

fn parse(value: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|date| date.with_timezone(&Utc))
}

/// A compact elapsed duration in the reference app's `5m 42s` style.
pub fn format_seconds(seconds: i64) -> String {
    let seconds = seconds.max(0);
    if seconds < 60 {
        format!("{seconds}s")
    } else if seconds < 3600 {
        format!("{}m {}s", seconds / 60, seconds % 60)
    } else {
        format!("{}h {}m", seconds / 3600, (seconds % 3600) / 60)
    }
}

/// Elapsed time between two ISO timestamps. `end` falls back to now.
pub fn format_elapsed(start: Option<&str>, end: Option<&str>) -> Option<String> {
    let start = parse(start?)?;
    let end = end.and_then(parse).unwrap_or_else(Utc::now);
    Some(format_seconds((end - start).num_seconds()))
}

/// The clock time a row was created, e.g. `14:03`.
pub fn clock_time(value: &str) -> Option<String> {
    parse(value).map(|date| date.format("%H:%M").to_string())
}

/// The label shown for a message role.
pub fn role_label(role: &str) -> &'static str {
    match role {
        "user" => "You",
        "assistant" => "T3 Code",
        "reasoning" => "Thinking",
        "system" => "System",
        _ => "Message",
    }
}

/// A one-line summary of the latest turn: its state and, when finished, how
/// long it took. Returns `None` when there is no turn to describe.
pub fn turn_summary(turn: &Value) -> Option<(String, TurnState)> {
    if turn.is_null() {
        return None;
    }
    let state = TurnState::parse(turn["state"].as_str().unwrap_or("completed"));
    let label = match state {
        TurnState::Running => "Working…".to_owned(),
        TurnState::Interrupted => "Interrupted".to_owned(),
        TurnState::Error => "Failed".to_owned(),
        TurnState::Completed => {
            let started = turn["startedAt"].as_str().or(turn["requestedAt"].as_str());
            match format_elapsed(started, turn["completedAt"].as_str()) {
                Some(elapsed) => format!("Worked for {elapsed}"),
                None => "Finished".to_owned(),
            }
        }
    };
    Some((label, state))
}

/// Added/deleted line totals for a checkpoint's changed files.
pub fn diff_totals(files: &[Value]) -> (u64, u64) {
    files.iter().fold((0, 0), |(additions, deletions), file| {
        (
            additions + file["additions"].as_u64().unwrap_or(0),
            deletions + file["deletions"].as_u64().unwrap_or(0),
        )
    })
}

/// An approval the agent is still waiting on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PendingApproval {
    pub request_id: String,
    pub summary: String,
    pub detail: Option<String>,
    /// Provider-supplied `(decision, label)` choices.
    pub options: Vec<(String, String)>,
}

/// The latest `approval.requested` activity that no `approval.resolved` has
/// closed, in activity order.
pub fn pending_approval(activities: &[Value]) -> Option<PendingApproval> {
    let mut pending: Option<Value> = None;
    let mut pending_id: Option<String> = None;
    for activity in activities {
        let kind = activity["kind"].as_str().unwrap_or("");
        let request_id = activity["payload"]["requestId"].as_str().map(str::to_owned);
        match kind {
            "approval.requested" => {
                if request_id.is_some() {
                    pending_id = request_id;
                    pending = Some(activity.clone());
                }
            }
            "approval.resolved"
                if request_id.is_some() && request_id == pending_id =>
            {
                pending = None;
                pending_id = None;
            }
            _ => {}
        }
    }
    let activity = pending?;
    let options = activity["payload"]["options"]
        .as_array()
        .map(|options| {
            options
                .iter()
                .filter_map(|option| {
                    Some((
                        option["decision"].as_str()?.to_owned(),
                        option["label"].as_str()?.to_owned(),
                    ))
                })
                .collect()
        })
        .unwrap_or_default();
    Some(PendingApproval {
        request_id: pending_id?,
        summary: activity["summary"]
            .as_str()
            .unwrap_or("Approval requested")
            .to_owned(),
        detail: activity["payload"]["detail"].as_str().map(str::to_owned),
        options,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn durations_use_human_units() {
        assert_eq!(format_seconds(9), "9s");
        assert_eq!(format_seconds(72), "1m 12s");
        assert_eq!(format_seconds(3_900), "1h 5m");
    }

    #[test]
    fn elapsed_reads_iso_timestamps() {
        let elapsed = format_elapsed(Some("2026-01-01T00:00:00Z"), Some("2026-01-01T00:05:42Z"));
        assert_eq!(elapsed.as_deref(), Some("5m 42s"));
        assert_eq!(format_elapsed(None, Some("2026-01-01T00:05:42Z")), None);
        assert_eq!(format_elapsed(Some("not-a-date"), None), None);
    }

    #[test]
    fn turn_summary_describes_each_state() {
        let completed = json!({
            "state": "completed",
            "requestedAt": "2026-01-01T00:00:00Z",
            "startedAt": "2026-01-01T00:00:00Z",
            "completedAt": "2026-01-01T00:00:42Z",
        });
        assert_eq!(
            turn_summary(&completed),
            Some(("Worked for 42s".to_owned(), TurnState::Completed))
        );
        assert!(turn_summary(&Value::Null).is_none());
        let error = json!({"state": "error"});
        assert_eq!(
            turn_summary(&error),
            Some(("Failed".to_owned(), TurnState::Error))
        );
        assert!(TurnState::parse("running") == TurnState::Running);
        assert!(TurnState::Interrupted.is_danger());
    }

    #[test]
    fn diff_totals_sum_files() {
        let files = vec![
            json!({"path": "a.rs", "additions": 10, "deletions": 2}),
            json!({"path": "b.rs", "additions": 3, "deletions": 5}),
        ];
        assert_eq!(diff_totals(&files), (13, 7));
        assert_eq!(diff_totals(&[]), (0, 0));
    }

    #[test]
    fn pending_approval_tracks_the_latest_unresolved_request() {
        let activities = vec![
            json!({"kind": "approval.requested", "summary": "Command approval requested", "payload": {"requestId": "a", "options": [{"decision": "accept", "label": "Allow once"}, {"decision": "decline", "label": "Deny"}]}}),
            json!({"kind": "approval.resolved", "payload": {"requestId": "a"}}),
            json!({"kind": "approval.requested", "summary": "File-change approval requested", "payload": {"requestId": "b", "detail": "write src/lib.rs"}}),
        ];
        let pending = pending_approval(&activities).expect("one approval is open");
        assert_eq!(pending.request_id, "b");
        assert_eq!(pending.summary, "File-change approval requested");
        assert_eq!(pending.detail.as_deref(), Some("write src/lib.rs"));
        assert!(pending.options.is_empty());

        let mut resolved = activities.clone();
        resolved.push(json!({"kind": "approval.resolved", "payload": {"requestId": "b"}}));
        assert!(pending_approval(&resolved).is_none());
    }
}
