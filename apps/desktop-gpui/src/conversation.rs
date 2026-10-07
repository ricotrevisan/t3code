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
            "approval.resolved" if request_id.is_some() && request_id == pending_id => {
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

/// One choice of a [`UserInputQuestion`]. `value` is the answer it submits:
/// the provider's option value, or its label when none is given.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UserInputOption {
    pub label: String,
    pub description: String,
    pub value: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UserInputQuestion {
    pub id: String,
    pub header: String,
    pub question: String,
    pub options: Vec<UserInputOption>,
    pub multi_select: bool,
    /// Custom text is allowed unless the provider sets `allowCustomAnswer: false`.
    pub allow_custom_answer: bool,
}

/// A question the agent is still waiting on the user to answer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PendingUserInput {
    pub request_id: String,
    pub questions: Vec<UserInputQuestion>,
    /// Async (`responseMode: "message"`) questions can be dismissed unanswered;
    /// native callbacks block the provider until they get a reply.
    pub dismissible: bool,
}

fn user_input_questions(value: &Value) -> Vec<UserInputQuestion> {
    let questions = value.as_array().map_or(&[][..], Vec::as_slice);
    questions
        .iter()
        .filter_map(|question| {
            let options: Vec<UserInputOption> = question["options"]
                .as_array()?
                .iter()
                .filter_map(|option| {
                    let label = option["label"].as_str()?.to_owned();
                    Some(UserInputOption {
                        value: option["value"].as_str().unwrap_or(&label).to_owned(),
                        description: option["description"].as_str().unwrap_or("").to_owned(),
                        label,
                    })
                })
                .collect();
            let allow_custom_answer = question["allowCustomAnswer"] != false;
            if options.is_empty() && !allow_custom_answer {
                return None;
            }
            Some(UserInputQuestion {
                id: question["id"].as_str()?.to_owned(),
                header: question["header"].as_str()?.to_owned(),
                question: question["question"].as_str()?.to_owned(),
                options,
                multi_select: question["multiSelect"] == true,
                allow_custom_answer,
            })
        })
        .collect()
}

/// The oldest `user-input.requested` activity that no `user-input.resolved`
/// (or stale-request reply failure) has closed, in activity order.
pub fn pending_user_input(activities: &[Value]) -> Option<PendingUserInput> {
    let mut open: Vec<PendingUserInput> = Vec::new();
    let mut closed = std::collections::HashSet::new();
    for activity in activities {
        let payload = &activity["payload"];
        let Some(request_id) = payload["requestId"].as_str() else {
            continue;
        };
        let stale_failure = activity["kind"] == "provider.user-input.respond.failed"
            && payload["detail"].as_str().is_some_and(|detail| {
                let detail = detail.to_lowercase();
                ["stale pending", "unknown pending"]
                    .iter()
                    .any(|fragment| detail.contains(fragment))
            });
        if activity["kind"] == "user-input.resolved" || stale_failure {
            closed.insert(request_id.to_owned());
            open.retain(|request| request.request_id != request_id);
        } else if activity["kind"] == "user-input.requested" && !closed.contains(request_id) {
            let questions = user_input_questions(&payload["questions"]);
            if questions.is_empty() {
                continue;
            }
            open.retain(|request| request.request_id != request_id);
            open.push(PendingUserInput {
                request_id: request_id.to_owned(),
                questions,
                dismissible: payload["responseMode"] == "message",
            });
        }
    }
    open.into_iter().next()
}

/// One turn of the conversation and the work it produced, in display order.
///
/// User messages carry no `turnId`, so a turn adopts the user message that
/// precedes its first item. Items without a turn form turnless groups.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TurnGroup {
    /// Stable key for per-turn view state: the turn id, or the first item's id.
    pub key: String,
    pub turn_id: Option<String>,
    /// User, assistant, reasoning, and system messages in timestamp order.
    pub messages: Vec<Value>,
    pub activities: Vec<Value>,
    pub plans: Vec<Value>,
    pub checkpoint: Option<Value>,
    /// `Working…`, `Worked for …`, `Interrupted`, or `Failed` when known.
    pub summary: Option<(String, TurnState)>,
}

impl TurnGroup {
    fn new(turn_id: Option<String>, key: String) -> Self {
        Self {
            key,
            turn_id,
            ..Self::default()
        }
    }

    fn push(&mut self, kind: ItemKind, item: Value) {
        match kind {
            ItemKind::Message => self.messages.push(item),
            ItemKind::Plan => self.plans.push(item),
            ItemKind::Activity => self.activities.push(item),
            ItemKind::Checkpoint => self.checkpoint = Some(item),
        }
    }

    /// Elapsed time from the turn's first item to its last recorded moment.
    fn worked_for(&self) -> Option<String> {
        let start = self
            .messages
            .iter()
            .chain(&self.activities)
            .chain(&self.plans)
            .filter_map(|item| item["createdAt"].as_str().and_then(parse))
            .min()?;
        let end = self
            .messages
            .iter()
            .filter_map(|message| message["updatedAt"].as_str())
            .chain(
                self.activities
                    .iter()
                    .filter_map(|activity| activity["createdAt"].as_str()),
            )
            .chain(
                self.checkpoint
                    .iter()
                    .filter_map(|checkpoint| checkpoint["completedAt"].as_str()),
            )
            .filter_map(parse)
            .max()?;
        Some(format_seconds((end - start).num_seconds()))
    }
}

#[derive(Clone, Copy)]
enum ItemKind {
    Message,
    Plan,
    Activity,
    Checkpoint,
}

fn turn_of(item: &Value) -> Option<String> {
    item["turnId"].as_str().map(str::to_owned)
}

/// Groups a thread's messages, proposed plans, activities, and checkpoints
/// into turns, in chronological order. Nothing is dropped: items without a
/// turn render in a turnless group at their timestamp.
pub fn group_turns(thread: &Value) -> Vec<TurnGroup> {
    let sources = [
        ("messages", "createdAt", ItemKind::Message),
        ("proposedPlans", "createdAt", ItemKind::Plan),
        ("activities", "createdAt", ItemKind::Activity),
        ("checkpoints", "completedAt", ItemKind::Checkpoint),
    ];
    let mut items: Vec<(Option<DateTime<Utc>>, ItemKind, &Value)> = sources
        .iter()
        .flat_map(|(key, time, kind)| {
            thread[*key]
                .as_array()
                .into_iter()
                .flatten()
                .map(move |item| (item[*time].as_str().and_then(parse), *kind, item))
        })
        .collect();
    items.sort_by_key(|(time, _, _)| *time);

    let mut groups: Vec<TurnGroup> = Vec::new();
    let mut by_turn = std::collections::HashMap::new();
    // The latest user message's group, until a turn claims it.
    let mut awaiting: Option<usize> = None;
    for (index, (_, kind, item)) in items.into_iter().enumerate() {
        let turn = turn_of(item);
        let fallback_key = || {
            item["id"]
                .as_str()
                .map_or_else(|| format!("item-{index}"), str::to_owned)
        };
        let is_user = matches!(kind, ItemKind::Message) && item["role"] == "user";
        let slot = match turn {
            Some(turn) if by_turn.contains_key(&turn) => by_turn[&turn],
            Some(turn) if !is_user && awaiting.is_some() => {
                let slot = awaiting.take().unwrap_or_default();
                groups[slot].key = turn.clone();
                groups[slot].turn_id = Some(turn.clone());
                by_turn.insert(turn, slot);
                slot
            }
            Some(turn) => {
                by_turn.insert(turn.clone(), groups.len());
                groups.push(TurnGroup::new(Some(turn.clone()), turn));
                if is_user {
                    awaiting = None;
                }
                groups.len() - 1
            }
            None if is_user => {
                groups.push(TurnGroup::new(None, fallback_key()));
                awaiting = Some(groups.len() - 1);
                groups.len() - 1
            }
            None => match groups.last() {
                Some(last) if last.turn_id.is_none() => groups.len() - 1,
                _ => {
                    groups.push(TurnGroup::new(None, fallback_key()));
                    groups.len() - 1
                }
            },
        };
        groups[slot].push(kind, item.clone());
    }

    let latest = &thread["latestTurn"];
    let latest_summary = turn_summary(latest);
    if let (Some(turn), Some(summary)) = (turn_of(latest), latest_summary) {
        if !by_turn.contains_key(&turn) {
            if let Some(slot) = awaiting {
                groups[slot].key = turn.clone();
                groups[slot].turn_id = Some(turn.clone());
                by_turn.insert(turn.clone(), slot);
            } else if summary.1 == TurnState::Running {
                by_turn.insert(turn.clone(), groups.len());
                groups.push(TurnGroup::new(Some(turn.clone()), turn.clone()));
            }
        }
        if let Some(&slot) = by_turn.get(&turn) {
            groups[slot].summary = Some(summary);
        }
    }
    for group in &mut groups {
        if group.summary.is_none() && group.turn_id.is_some() {
            group.summary = group
                .worked_for()
                .map(|elapsed| (format!("Worked for {elapsed}"), TurnState::Completed));
        }
    }
    groups
}

/// One renderable entry of the conversation timeline.
#[derive(Clone, Debug, PartialEq)]
pub enum TimelineEntry {
    Message(Value),
    /// A settled turn's `Worked for …`, `Interrupted`, or `Failed` line.
    TurnStatus {
        label: String,
        danger: bool,
    },
    /// The turn's activity disclosure. `latest` marks the newest one, whose
    /// toggle keeps the bare selector id.
    Activity {
        turn: String,
        label: String,
        activities: Vec<Value>,
        expanded: bool,
        latest: bool,
    },
    Plan(Value),
    Checkpoint {
        turn: String,
        checkpoint: Value,
        latest: bool,
    },
    /// The running turn's `Working…` indicator.
    Working(String),
    Approval(PendingApproval),
    UserInput(PendingUserInput),
    Error(String),
}

/// A timeline row with a key that stays the same while the entry it renders
/// exists, so a virtual list can splice around it.
#[derive(Clone, Debug, PartialEq)]
pub struct TimelineRow {
    pub key: String,
    pub entry: TimelineEntry,
}

/// Flattens a thread into timeline rows in display order: each turn's
/// messages, its status, activity disclosure, plans, checkpoint, and working
/// indicator, then the pending approval, the open user-input request, and the
/// session error. `expanded` holds the activity disclosures the user toggled,
/// by turn key; untouched ones are open while their turn runs.
pub fn timeline_rows(
    thread: &Value,
    expanded: &std::collections::HashMap<String, bool>,
) -> Vec<TimelineRow> {
    let groups = group_turns(thread);
    let visible = |activity: &Value| activity["kind"] != "worktree-setup";
    let has_files = |checkpoint: &Value| {
        checkpoint["files"]
            .as_array()
            .is_some_and(|files| !files.is_empty())
    };
    let latest_activity = groups
        .iter()
        .rposition(|group| group.activities.iter().any(visible));
    let latest_checkpoint = groups
        .iter()
        .rposition(|group| group.checkpoint.as_ref().is_some_and(has_files));

    let mut rows = Vec::new();
    let mut push = |key: String, entry| rows.push(TimelineRow { key, entry });
    for (index, group) in groups.into_iter().enumerate() {
        let TurnGroup {
            key,
            messages,
            activities,
            plans,
            checkpoint,
            summary,
            ..
        } = group;
        let activities: Vec<Value> = activities.into_iter().filter(visible).collect();
        for (position, message) in messages.into_iter().enumerate() {
            let id = message["id"]
                .as_str()
                .map_or_else(|| format!("{key}:{position}"), str::to_owned);
            push(format!("message:{id}"), TimelineEntry::Message(message));
        }
        let running = matches!(summary, Some((_, TurnState::Running)));
        match &summary {
            Some((label, state)) if state.is_danger() => push(
                format!("status:{key}"),
                TimelineEntry::TurnStatus {
                    label: label.clone(),
                    danger: true,
                },
            ),
            Some((label, TurnState::Completed)) if activities.is_empty() => push(
                format!("status:{key}"),
                TimelineEntry::TurnStatus {
                    label: label.clone(),
                    danger: false,
                },
            ),
            _ => {}
        }
        if !activities.is_empty() {
            let label = match &summary {
                Some((label, TurnState::Completed)) => {
                    format!("{label} · {} steps", activities.len())
                }
                _ => format!("{} activity steps", activities.len()),
            };
            push(
                format!("activity:{key}"),
                TimelineEntry::Activity {
                    turn: key.clone(),
                    label,
                    expanded: expanded.get(&key).copied().unwrap_or(running),
                    activities,
                    latest: latest_activity == Some(index),
                },
            );
        }
        for plan in plans {
            if plan["planMarkdown"]
                .as_str()
                .is_some_and(|text| !text.is_empty())
            {
                let id = plan["id"].as_str().unwrap_or("unknown").to_owned();
                push(format!("plan:{id}"), TimelineEntry::Plan(plan));
            }
        }
        if let Some(checkpoint) = checkpoint.filter(has_files) {
            push(
                format!("checkpoint:{key}"),
                TimelineEntry::Checkpoint {
                    turn: key.clone(),
                    checkpoint,
                    latest: latest_checkpoint == Some(index),
                },
            );
        }
        if let Some((label, TurnState::Running)) = summary {
            push(format!("working:{key}"), TimelineEntry::Working(label));
        }
    }
    if let Some(approval) = thread["activities"]
        .as_array()
        .and_then(|activities| pending_approval(activities))
    {
        push(
            format!("approval:{}", approval.request_id),
            TimelineEntry::Approval(approval),
        );
    }
    if let Some(request) = thread["activities"]
        .as_array()
        .and_then(|activities| pending_user_input(activities))
    {
        push(
            format!("user-input:{}", request.request_id),
            TimelineEntry::UserInput(request),
        );
    }
    if let Some(error) = thread["session"]["lastError"].as_str() {
        push("error".to_owned(), TimelineEntry::Error(error.to_owned()));
    }
    rows
}

/// How a list showing `old` rows catches up with `new`: replace the rows in
/// `replaced` with `inserted` new ones, then remeasure the surviving rows at
/// `changed` (indices into `new`) whose content changed in place.
#[derive(Debug, PartialEq, Eq)]
pub struct RowsDiff {
    pub replaced: std::ops::Range<usize>,
    pub inserted: usize,
    pub changed: Vec<usize>,
}

pub fn diff_rows(old: &[TimelineRow], new: &[TimelineRow]) -> RowsDiff {
    let prefix = old
        .iter()
        .zip(new)
        .take_while(|(old, new)| old.key == new.key)
        .count();
    let suffix = old
        .iter()
        .rev()
        .zip(new.iter().rev())
        .take(old.len().min(new.len()) - prefix)
        .take_while(|(old, new)| old.key == new.key)
        .count();
    let changed = (0..prefix)
        .chain(new.len() - suffix..new.len())
        .filter(|&index| {
            let before = if index < prefix {
                index
            } else {
                old.len() - (new.len() - index)
            };
            old[before].entry != new[index].entry
        })
        .collect();
    RowsDiff {
        replaced: prefix..old.len() - suffix,
        inserted: new.len() - prefix - suffix,
        changed,
    }
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

    fn question_request(id: &str) -> Value {
        json!({"kind": "user-input.requested", "payload": {"requestId": id, "questions": [
            {"id": "q", "header": "Scope", "question": "Which?", "options": [{"label": "A", "description": "first"}, {"label": "B", "description": "", "value": "b"}]}
        ]}})
    }

    #[test]
    fn pending_user_input_is_the_oldest_unresolved_request() {
        let resolved = json!({"kind": "user-input.resolved", "payload": {"requestId": "a"}});
        assert!(pending_user_input(&[question_request("a"), resolved.clone()]).is_none());

        let pending = pending_user_input(&[question_request("a"), question_request("b"), resolved])
            .expect("one request is open");
        assert_eq!(pending.request_id, "b");
        assert!(!pending.dismissible);
        let question = &pending.questions[0];
        assert_eq!(
            (question.id.as_str(), question.header.as_str()),
            ("q", "Scope")
        );
        assert_eq!(
            question.options[0].value, "A",
            "a label stands in for a missing value"
        );
        assert_eq!(question.options[1].value, "b");
        assert!(question.allow_custom_answer && !question.multi_select);

        let failed = json!({"kind": "provider.user-input.respond.failed", "payload": {"requestId": "b", "detail": "Stale pending user-input request"}});
        assert!(pending_user_input(&[question_request("b"), failed]).is_none());
        let mut message = question_request("c");
        message["payload"]["responseMode"] = json!("message");
        assert!(pending_user_input(&[message]).unwrap().dismissible);
    }

    #[test]
    fn user_input_card_follows_the_approval_card() {
        let mut question = question_request("q");
        question["id"] = json!("y");
        question["createdAt"] = json!("2026-01-01T00:00:01Z");
        let thread = json!({"messages": [], "activities": [
            {"id": "x", "kind": "approval.requested", "createdAt": "2026-01-01T00:00:00Z", "payload": {"requestId": "r"}},
            question,
        ]});
        let keys: Vec<String> = timeline_rows(&thread, &Default::default())
            .into_iter()
            .map(|row| row.key)
            .collect();
        assert!(keys.ends_with(&["approval:r".to_owned(), "user-input:q".to_owned()]));
    }

    fn fixture(latest: Value) -> Value {
        json!({
            "messages": [
                {"id": "u1", "role": "user", "text": "first", "turnId": null, "createdAt": "2026-01-01T00:00:00Z", "updatedAt": "2026-01-01T00:00:00Z"},
                {"id": "a1", "role": "assistant", "text": "done", "turnId": "t1", "createdAt": "2026-01-01T00:00:05Z", "updatedAt": "2026-01-01T00:00:30Z"},
                {"id": "s1", "role": "system", "text": "note", "turnId": null, "createdAt": "2026-01-01T00:01:00Z", "updatedAt": "2026-01-01T00:01:00Z"},
                {"id": "u2", "role": "user", "text": "second", "turnId": null, "createdAt": "2026-01-01T00:02:00Z", "updatedAt": "2026-01-01T00:02:00Z"},
                {"id": "a2", "role": "assistant", "text": "working", "turnId": "t2", "streaming": true, "createdAt": "2026-01-01T00:02:10Z", "updatedAt": "2026-01-01T00:02:10Z"},
            ],
            "activities": [
                {"id": "e1", "kind": "tool.completed", "summary": "Ran tests", "turnId": "t1", "createdAt": "2026-01-01T00:00:10Z"},
                {"id": "e2", "kind": "tool.started", "summary": "Editing", "turnId": "t2", "createdAt": "2026-01-01T00:02:05Z"},
            ],
            "proposedPlans": [
                {"id": "p1", "turnId": "t1", "planMarkdown": "- step", "createdAt": "2026-01-01T00:00:20Z"},
            ],
            "checkpoints": [
                {"turnId": "t1", "checkpointTurnCount": 1, "files": [], "completedAt": "2026-01-01T00:00:42Z"},
            ],
            "latestTurn": latest,
        })
    }

    fn ids(items: &[Value]) -> Vec<&str> {
        items
            .iter()
            .map(|item| item["id"].as_str().unwrap())
            .collect()
    }

    #[test]
    fn group_turns_associates_items_with_their_turn_in_order() {
        let groups = group_turns(&fixture(
            json!({"turnId": "t2", "state": "running", "requestedAt": "2026-01-01T00:02:00Z"}),
        ));
        assert_eq!(groups.len(), 3);

        let first = &groups[0];
        assert_eq!(first.turn_id.as_deref(), Some("t1"));
        assert_eq!(ids(&first.messages), ["u1", "a1"]);
        assert_eq!(ids(&first.activities), ["e1"]);
        assert_eq!(ids(&first.plans), ["p1"]);
        assert_eq!(first.checkpoint.as_ref().unwrap()["checkpointTurnCount"], 1);
        assert_eq!(
            first.summary,
            Some(("Worked for 42s".to_owned(), TurnState::Completed))
        );

        let orphan = &groups[1];
        assert_eq!(orphan.turn_id, None);
        assert_eq!(ids(&orphan.messages), ["s1"]);
        assert_eq!(orphan.summary, None);

        let running = &groups[2];
        assert_eq!(running.key, "t2");
        assert_eq!(ids(&running.messages), ["u2", "a2"]);
        assert_eq!(ids(&running.activities), ["e2"]);
        assert!(running.checkpoint.is_none());
        assert_eq!(
            running.summary,
            Some(("Working…".to_owned(), TurnState::Running))
        );
    }

    #[test]
    fn group_turns_gives_a_just_started_turn_its_user_message() {
        let mut thread = fixture(Value::Null);
        thread["messages"].as_array_mut().unwrap().truncate(4);
        thread["activities"].as_array_mut().unwrap().truncate(1);
        thread["latestTurn"] =
            json!({"turnId": "t2", "state": "running", "requestedAt": "2026-01-01T00:02:00Z"});
        let groups = group_turns(&thread);
        let last = groups.last().unwrap();
        assert_eq!(last.turn_id.as_deref(), Some("t2"));
        assert_eq!(ids(&last.messages), ["u2"]);
        assert_eq!(last.summary.as_ref().unwrap().1, TurnState::Running);

        thread["messages"].as_array_mut().unwrap().truncate(3);
        let groups = group_turns(&thread);
        let last = groups.last().unwrap();
        assert_eq!(last.turn_id.as_deref(), Some("t2"));
        assert!(
            last.messages.is_empty(),
            "a running turn shows even before any item"
        );
    }

    #[test]
    fn timeline_rows_flatten_turns_into_one_row_per_entry() {
        let mut thread = fixture(
            json!({"turnId": "t2", "state": "running", "requestedAt": "2026-01-01T00:02:00Z"}),
        );
        let rows = timeline_rows(&thread, &Default::default());
        let keys: Vec<&str> = rows.iter().map(|row| row.key.as_str()).collect();
        assert_eq!(
            keys,
            [
                "message:u1",
                "message:a1",
                "activity:t1",
                "plan:p1",
                "message:s1",
                "message:u2",
                "message:a2",
                "activity:t2",
                "working:t2",
            ],
            "a checkpoint without files renders nothing, so it has no row"
        );
        assert!(matches!(
            &rows[7].entry,
            TimelineEntry::Activity {
                expanded: true,
                latest: true,
                ..
            }
        ));
        assert!(matches!(
            &rows[2].entry,
            TimelineEntry::Activity {
                expanded: false,
                latest: false,
                ..
            }
        ));

        thread["messages"].as_array_mut().unwrap().push(json!(
            {"id": "a3", "role": "assistant", "text": "more", "turnId": "t2", "createdAt": "2026-01-01T00:02:20Z", "updatedAt": "2026-01-01T00:02:20Z"}
        ));
        let appended = timeline_rows(&thread, &Default::default());
        assert_eq!(appended[7].key, "message:a3");
        assert_eq!(
            diff_rows(&rows, &appended),
            RowsDiff {
                replaced: 7..7,
                inserted: 1,
                changed: vec![]
            },
            "an appended message splices one row and keeps every other row"
        );

        thread["messages"][4]["text"] = json!("working harder");
        let streamed = timeline_rows(&thread, &Default::default());
        assert_eq!(
            diff_rows(&appended, &streamed),
            RowsDiff {
                replaced: 10..10,
                inserted: 0,
                changed: vec![6]
            },
            "streaming text remeasures its row in place"
        );
        let collapsed = timeline_rows(&thread, &[("t2".to_owned(), false)].into());
        assert_eq!(diff_rows(&streamed, &collapsed).changed, vec![8]);
        assert_eq!(
            diff_rows(&[], &collapsed),
            RowsDiff {
                replaced: 0..0,
                inserted: 10,
                changed: vec![]
            }
        );
    }

    #[test]
    fn group_turns_keeps_items_without_a_turn() {
        let thread = json!({
            "messages": [],
            "activities": [
                {"id": "x", "kind": "approval.requested", "turnId": null, "createdAt": "2026-01-01T00:00:01Z"},
                {"id": "y", "kind": "runtime.warning", "turnId": null, "createdAt": "2026-01-01T00:00:00Z"},
            ],
            "proposedPlans": [],
            "checkpoints": [],
            "latestTurn": null,
        });
        let groups = group_turns(&thread);
        assert_eq!(groups.len(), 1);
        assert_eq!(ids(&groups[0].activities), ["y", "x"]);
        assert_eq!(groups[0].key, "y");
        assert!(group_turns(&json!({})).is_empty());
    }
}
