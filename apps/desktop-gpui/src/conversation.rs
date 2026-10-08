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
fn step_count(count: usize) -> String {
    if count == 1 {
        "1 step".to_owned()
    } else {
        format!("{count} steps")
    }
}

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

/// How full the context window is, as the meter colors it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ContextWindowLevel {
    Normal,
    Warning,
    Danger,
}

/// The newest `context-window.updated` reading for a thread.
#[derive(Clone, Debug, PartialEq)]
pub struct ContextWindowSnapshot {
    pub used_tokens: f64,
    pub max_tokens: Option<f64>,
    pub remaining_tokens: Option<f64>,
    /// Share of `max_tokens` in use, capped at 100; None without a positive max.
    pub used_percentage: Option<f64>,
    pub total_processed_tokens: Option<f64>,
}

impl ContextWindowSnapshot {
    /// The meter chip's text: the used share, or the used tokens without a max.
    pub fn label(&self) -> String {
        match self.used_percentage {
            Some(percentage) => format_context_window_percentage(percentage),
            None => format_context_window_tokens(self.used_tokens),
        }
    }

    pub fn level(&self) -> ContextWindowLevel {
        match self.used_percentage.unwrap_or(0.) {
            percentage if percentage > 90. => ContextWindowLevel::Danger,
            percentage if percentage > 75. => ContextWindowLevel::Warning,
            _ => ContextWindowLevel::Normal,
        }
    }

    pub fn tooltip(&self) -> String {
        let used = format_context_window_tokens(self.used_tokens);
        let mut text = match (self.max_tokens, self.remaining_tokens) {
            (Some(max), Some(remaining)) => format!(
                "Context window: {used} / {} used, {} left",
                format_context_window_tokens(max),
                format_context_window_tokens(remaining)
            ),
            _ => format!("Context window: {used} tokens used"),
        };
        if let Some(total) = self.total_processed_tokens.filter(|total| *total > 0.) {
            text.push_str(&format!(
                " · {} processed in total",
                format_context_window_tokens(total)
            ));
        }
        text
    }
}

/// The latest valid context-window reading; entries without a non-negative
/// `usedTokens` are skipped in favour of older ones.
pub fn context_window_snapshot(activities: &[Value]) -> Option<ContextWindowSnapshot> {
    activities.iter().rev().find_map(|activity| {
        if activity["kind"] != "context-window.updated" {
            return None;
        }
        let payload = &activity["payload"];
        let used_tokens = payload["usedTokens"].as_f64().filter(|used| *used >= 0.)?;
        let max_tokens = payload["maxTokens"].as_f64();
        let used_percentage = max_tokens
            .filter(|max| *max > 0.)
            .map(|max| (used_tokens / max * 100.).min(100.));
        Some(ContextWindowSnapshot {
            used_tokens,
            max_tokens,
            remaining_tokens: max_tokens.map(|max| (max - used_tokens).round().max(0.)),
            used_percentage,
            total_processed_tokens: payload["totalProcessedTokens"].as_f64(),
        })
    })
}

/// One decimal, ties rounded up as the reference's `toFixed(1)` does, without a trailing `.0`.
fn trim_tenth(value: f64) -> String {
    let text = format!("{:.1}", (value * 10.).round() / 10.);
    text.strip_suffix(".0").map(str::to_owned).unwrap_or(text)
}

/// Token counts in the reference meter's style: `999`, `1k`, `9.5k`, `132k`, `1.3m`.
pub fn format_context_window_tokens(value: f64) -> String {
    if !value.is_finite() {
        return "0".to_owned();
    }
    if value < 1_000. {
        format!("{}", value.round())
    } else if value < 10_000. {
        format!("{}k", trim_tenth(value / 1_000.))
    } else if value < 1_000_000. {
        format!("{}k", (value / 1_000.).round())
    } else {
        format!("{}m", trim_tenth(value / 1_000_000.))
    }
}

/// `4.5%` below ten percent, whole percents above.
pub fn format_context_window_percentage(value: f64) -> String {
    if value < 10. {
        format!("{}%", trim_tenth(value))
    } else {
        format!("{}%", value.round())
    }
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

/// Where a tool call stands, as its row marks it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToolState {
    Running,
    Completed,
    Failed,
}

/// One tool call: the `tool.started`, `tool.updated`, and `tool.completed`
/// activities that share a `toolCallId`, merged into a single row.
#[derive(Clone, Debug, PartialEq)]
pub struct ToolCall {
    /// The call id, or the first activity's id when the provider sent none.
    pub id: String,
    /// The first activity's id: unique per row even when a provider reuses
    /// call ids, so it keys expansion and the row's debug selectors.
    pub key: String,
    pub name: String,
    /// The key argument: command line, file path, search pattern, or URL.
    pub headline: Option<String>,
    pub state: ToolState,
    /// Detail, output, diff, and touched files, shown when the row expands.
    pub body: Option<String>,
    /// Added and removed lines, when the payload reports them.
    pub line_counts: Option<(u64, u64)>,
    pub expanded: bool,
    pub activities: Vec<Value>,
}

impl ToolCall {
    fn new(activities: Vec<Value>) -> Self {
        let latest = |read: fn(&Value) -> Option<String>| activities.iter().rev().find_map(read);
        let name = latest(tool_name).unwrap_or_else(|| "Tool".to_owned());
        let completed = activities
            .iter()
            .any(|activity| activity["kind"] == "tool.completed");
        let errored = activities
            .last()
            .is_some_and(|activity| activity["tone"] == "error");
        let state = match latest(|activity| text(&activity["payload"]["status"])).as_deref() {
            Some("failed" | "declined" | "stopped" | "cancelled" | "interrupted") => {
                ToolState::Failed
            }
            _ if errored => ToolState::Failed,
            Some("completed") => ToolState::Completed,
            _ if completed => ToolState::Completed,
            _ => ToolState::Running,
        };
        let detail = latest(|activity| strip_exit_code(&text(&activity["payload"]["detail"])?));
        let paths = latest(|activity| {
            let paths = tool_paths(&activity["payload"]["data"]);
            (!paths.is_empty()).then(|| paths.join("\n"))
        });
        let headline = latest(tool_command)
            .or_else(|| Some(paths.as_deref()?.lines().next()?.to_owned()))
            .or_else(|| latest(tool_pattern))
            .or_else(|| latest(tool_url))
            .or_else(|| {
                let line = detail.as_deref()?.lines().next()?.trim();
                (!same_text(line, &name)).then(|| line.to_owned())
            });
        let mut seen = vec![name.clone()];
        seen.extend(headline.clone());
        let mut blocks = Vec::new();
        let candidates = [
            detail,
            latest(tool_output),
            latest(tool_diff),
            paths,
            latest(mcp_arguments),
        ];
        for block in candidates.into_iter().flatten() {
            if !seen.iter().any(|seen| same_text(seen, &block)) {
                seen.push(block.clone());
                blocks.push(block);
            }
        }
        let first_id = activities
            .first()
            .and_then(|activity| text(&activity["id"]));
        let id = activities
            .iter()
            .find_map(tool_call_id)
            .or_else(|| first_id.clone())
            .unwrap_or_else(|| "unknown".to_owned());
        Self {
            key: first_id.unwrap_or_else(|| id.clone()),
            id,
            name,
            headline,
            state,
            body: (!blocks.is_empty()).then(|| blocks.join("\n\n")),
            line_counts: activities.iter().rev().find_map(line_counts),
            expanded: false,
            activities,
        }
    }

    /// The key its expansion is stored under, beside the turn disclosures.
    pub fn expansion_key(&self) -> String {
        format!("tool:{}", self.key)
    }
}

/// One line of a turn's activity disclosure.
#[derive(Clone, Debug, PartialEq)]
pub enum ActivityItem {
    Tool(ToolCall),
    Activity(Value),
    Error(Value),
}

/// Folds a turn's activities into disclosure lines, each at the position of
/// its first activity. Tool lifecycle activities merge by call id until the
/// call completes; an id-less one joins the open call just before it when it
/// names the same tool. Nothing is dropped: other kinds stay plain lines, and
/// error-toned ones become errors.
pub fn activity_items(activities: &[Value]) -> Vec<ActivityItem> {
    let mut items = Vec::new();
    // Calls collect their activities here and are derived once all have arrived.
    let mut calls: Vec<Vec<Value>> = Vec::new();
    for activity in activities {
        let kind = activity["kind"].as_str().unwrap_or("");
        if !matches!(kind, "tool.started" | "tool.updated" | "tool.completed") {
            items.push(if activity["tone"] == "error" {
                ActivityItem::Error(activity.clone())
            } else {
                ActivityItem::Activity(activity.clone())
            });
            continue;
        }
        let open = |call: &Vec<Value>| !call.iter().any(|item| item["kind"] == "tool.completed");
        let target = match tool_call_id(activity) {
            Some(id) => calls.iter().rposition(|call| {
                open(call) && call.iter().find_map(tool_call_id).as_ref() == Some(&id)
            }),
            None => match items.last() {
                Some(ActivityItem::Tool(_)) => calls.len().checked_sub(1).filter(|&last| {
                    let call = &calls[last];
                    open(call)
                        && call[0]["payload"]["itemType"] == activity["payload"]["itemType"]
                        && tool_name(&call[0]) == tool_name(activity)
                }),
                _ => None,
            },
        };
        match target {
            Some(index) => calls[index].push(activity.clone()),
            None => {
                calls.push(vec![activity.clone()]);
                items.push(ActivityItem::Tool(ToolCall::new(Vec::new())));
            }
        }
    }
    let mut calls = calls.into_iter();
    for item in &mut items {
        if let ActivityItem::Tool(call) = item {
            *call = ToolCall::new(calls.next().unwrap_or_default());
        }
    }
    items
}

fn text(value: &Value) -> Option<String> {
    let text = value.as_str()?.trim();
    (!text.is_empty()).then(|| text.to_owned())
}

/// Compares labels the way the reference does: whitespace-insensitive, and
/// ignoring the trailing `complete` some providers append on completion.
fn same_text(left: &str, right: &str) -> bool {
    let normalize = |value: &str| {
        let words = value.split_whitespace().collect::<Vec<_>>().join(" ");
        let lower = words.to_lowercase();
        ["completed", "complete"]
            .iter()
            .find_map(|suffix| lower.strip_suffix(suffix))
            .unwrap_or(&lower)
            .trim()
            .to_owned()
    };
    normalize(left) == normalize(right)
}

fn tool_call_id(activity: &Value) -> Option<String> {
    let payload = &activity["payload"];
    text(&payload["toolCallId"]).or_else(|| text(&payload["data"]["toolCallId"]))
}

fn tool_name(activity: &Value) -> Option<String> {
    let payload = &activity["payload"];
    let summary = || {
        let summary = text(&activity["summary"])?;
        match summary.strip_suffix(" started") {
            Some(name) if activity["kind"] == "tool.started" => Some(name.to_owned()),
            _ => Some(summary),
        }
    };
    let name = text(&payload["title"])
        .or_else(|| text(&payload["data"]["toolName"]))
        .or_else(summary)?;
    let lower = name.to_ascii_lowercase();
    let cut = [" completed", " complete"]
        .iter()
        .find_map(|suffix| lower.strip_suffix(suffix).map(str::len))
        .unwrap_or(name.len());
    Some(name[..cut].trim_end().to_owned())
}

/// The output before a trailing `<exited with exit code N>` marker.
fn strip_exit_code(value: &str) -> Option<String> {
    let value = value.trim();
    let output = value
        .strip_suffix('>')
        .and_then(|rest| rest.rfind("<exited with exit code ").map(|at| &value[..at]))
        .unwrap_or(value)
        .trim();
    (!output.is_empty()).then(|| output.to_owned())
}

/// A command array or string, without a `bash -lc '…'` style wrapper.
fn command_text(value: &Value) -> Option<String> {
    if let Some(parts) = value.as_array() {
        let parts: Vec<String> = parts
            .iter()
            .filter_map(text)
            .map(|part| {
                if part.contains(char::is_whitespace) || part.contains(['"', '\'']) {
                    format!("\"{}\"", part.replace('"', "\\\""))
                } else {
                    part
                }
            })
            .collect();
        return (!parts.is_empty()).then(|| unwrap_shell(&parts.join(" ")));
    }
    Some(unwrap_shell(&text(value)?))
}

fn unwrap_shell(command: &str) -> String {
    let unwrapped = command
        .split_once(char::is_whitespace)
        .and_then(|(shell, rest)| {
            let shell = shell.trim_matches(['"', '\'']).rsplit('/').next()?;
            if !matches!(shell, "bash" | "sh" | "zsh") {
                return None;
            }
            let rest = rest.trim_start();
            let script = rest
                .strip_prefix("-lc")
                .or_else(|| rest.strip_prefix("-c"))?
                .trim();
            let quoted = ['"', '\'']
                .iter()
                .find_map(|quote| script.strip_prefix(*quote)?.strip_suffix(*quote));
            Some(quoted.unwrap_or(script).trim().to_owned())
        });
    unwrapped
        .filter(|script| !script.is_empty())
        .unwrap_or_else(|| command.to_owned())
}

fn tool_command(activity: &Value) -> Option<String> {
    let payload = &activity["payload"];
    let data = &payload["data"];
    let item = &data["item"];
    [
        &item["command"],
        &item["input"]["command"],
        &item["result"]["command"],
        &data["command"],
        &data["input"]["command"],
    ]
    .into_iter()
    .find_map(command_text)
    .or_else(|| {
        (payload["itemType"] == "command_execution")
            .then(|| strip_exit_code(&text(&payload["detail"])?))?
    })
}

fn tool_paths(data: &Value) -> Vec<String> {
    let mut paths: Vec<String> = Vec::new();
    let listed = ["files", "changes"]
        .iter()
        .flat_map(|key| [&data[*key], &data["item"][*key]])
        .filter_map(Value::as_array)
        .flatten()
        .filter_map(|file| text(&file["path"]));
    let input = &data["input"];
    let single = [&input["file_path"], &input["filePath"], &input["path"]]
        .into_iter()
        .filter_map(text);
    for path in listed.chain(single) {
        if !paths.contains(&path) {
            paths.push(path);
        }
    }
    paths
}

fn tool_pattern(activity: &Value) -> Option<String> {
    let data = &activity["payload"]["data"];
    [
        &data["input"]["pattern"],
        &data["input"]["query"],
        &data["item"]["query"],
        &data["query"],
    ]
    .into_iter()
    .find_map(text)
}

fn tool_url(activity: &Value) -> Option<String> {
    let data = &activity["payload"]["data"];
    [&data["input"]["url"], &data["item"]["url"], &data["url"]]
        .into_iter()
        .find_map(text)
}

fn tool_output(activity: &Value) -> Option<String> {
    let data = &activity["payload"]["data"];
    let item = &data["item"];
    let raw = &data["rawOutput"];
    [
        &item["aggregatedOutput"],
        &item["result"]["content"],
        raw,
        &raw["content"],
        &raw["stdout"],
        &raw["stderr"],
        &raw["output"],
        &data["result"]["content"],
        &data["result"],
    ]
    .into_iter()
    .find_map(|value| strip_exit_code(&text(value)?))
}

/// A unified diff the payload carries, per changed file when it has several.
fn tool_diff(activity: &Value) -> Option<String> {
    let data = &activity["payload"]["data"];
    let direct = [&data["diff"], &data["patch"], &data["item"]["diff"]]
        .into_iter()
        .find_map(text);
    direct.or_else(|| {
        let diffs: Vec<String> = [&data["changes"], &data["item"]["changes"]]
            .into_iter()
            .filter_map(Value::as_array)
            .flatten()
            .filter_map(|change| text(&change["diff"]))
            .collect();
        (!diffs.is_empty()).then(|| diffs.join("\n"))
    })
}

fn mcp_arguments(activity: &Value) -> Option<String> {
    if activity["payload"]["itemType"] != "mcp_tool_call" {
        return None;
    }
    let data = &activity["payload"]["data"];
    [&data["item"]["arguments"], &data["input"]]
        .into_iter()
        .find(|value| value.as_object().is_some_and(|map| !map.is_empty()))
        .and_then(|value| serde_json::to_string_pretty(value).ok())
}

/// Added and removed lines from explicit counts, per-file counts, or a diff.
fn line_counts(activity: &Value) -> Option<(u64, u64)> {
    let payload = &activity["payload"];
    let data = &payload["data"];
    let counts = |value: &Value| Some((value["additions"].as_u64()?, value["deletions"].as_u64()?));
    if let Some(counts) = [payload, data, &data["item"]].into_iter().find_map(counts) {
        return Some(counts);
    }
    let files: Vec<(u64, u64)> = ["files", "changes"]
        .iter()
        .flat_map(|key| [&data[*key], &data["item"][*key]])
        .filter_map(Value::as_array)
        .flatten()
        .filter_map(counts)
        .collect();
    if !files.is_empty() {
        return Some(
            files
                .iter()
                .fold((0, 0), |(a, d), (fa, fd)| (a + fa, d + fd)),
        );
    }
    let diff = tool_diff(activity)?;
    Some(diff.lines().fold((0, 0), |(additions, deletions), line| {
        if line.starts_with('+') && !line.starts_with("+++") {
            (additions + 1, deletions)
        } else if line.starts_with('-') && !line.starts_with("---") {
            (additions, deletions + 1)
        } else {
            (additions, deletions)
        }
    }))
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
    /// `show_header` is false for a message that continues a run of messages
    /// from the same role, which renders without its role and time label.
    Message {
        message: Value,
        show_header: bool,
        /// The first message anchors the turn folding control.
        turn: Option<String>,
    },
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
        items: Vec<ActivityItem>,
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

/// Tool families used by both work summaries and semantic icons.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToolFamily {
    Command,
    Search,
    Edit,
    Agent,
    Other,
}

pub fn tool_family(name: &str) -> ToolFamily {
    let name = name.to_ascii_lowercase();
    if ["spawn", "agent", "delegate"]
        .iter()
        .any(|word| name.contains(word))
    {
        ToolFamily::Agent
    } else if ["exec", "shell", "bash", "command", "terminal"]
        .iter()
        .any(|word| name.contains(word))
    {
        ToolFamily::Command
    } else if ["search", "grep", "glob", "find"]
        .iter()
        .any(|word| name.contains(word))
    {
        ToolFamily::Search
    } else if ["edit", "write", "patch"]
        .iter()
        .any(|word| name.contains(word))
    {
        ToolFamily::Edit
    } else {
        ToolFamily::Other
    }
}

fn activity_time(item: &ActivityItem) -> &str {
    let value = match item {
        ActivityItem::Tool(call) => call.activities.first(),
        ActivityItem::Activity(value) | ActivityItem::Error(value) => Some(value),
    };
    value
        .and_then(|value| value["createdAt"].as_str())
        .unwrap_or("")
}

fn item_family(item: Option<&ActivityItem>) -> ToolFamily {
    match item {
        Some(ActivityItem::Tool(call)) => tool_family(&call.name),
        _ => ToolFamily::Other,
    }
}

fn work_label(items: &[ActivityItem]) -> String {
    let mut commands = 0;
    let mut searches = 0;
    let mut edits = 0;
    for item in items {
        if let ActivityItem::Tool(call) = item {
            match tool_family(&call.name) {
                ToolFamily::Command => commands += 1,
                ToolFamily::Search => searches += 1,
                ToolFamily::Edit => edits += 1,
                _ => {}
            }
        }
    }
    if commands == items.len() {
        format!("Ran {commands} commands")
    } else if searches == items.len() {
        format!("Ran {searches} searches")
    } else if edits == items.len() {
        format!("Made {edits} edits")
    } else {
        step_count(items.len())
    }
}

/// Merge work at its first recorded timestamp, keeping lifecycle updates in
/// their original call and never grouping across an intervening message/plan.
fn interleave_turn_rows(rows: Vec<TimelineRow>) -> Vec<TimelineRow> {
    let mut dated = Vec::new();
    let mut footer = Vec::new();
    let mut elapsed = None;
    for row in rows {
        match row.entry {
            TimelineEntry::Activity {
                turn,
                items,
                expanded,
                latest,
                label,
            } => {
                if label.starts_with("Worked for ") {
                    elapsed = label.split(" · ").next().map(str::to_owned);
                }
                for item in items {
                    let time = activity_time(&item).to_owned();
                    let item_key = match &item {
                        ActivityItem::Tool(call) => call.key.clone(),
                        ActivityItem::Activity(value) | ActivityItem::Error(value) => {
                            value["id"].as_str().unwrap_or("unknown").to_owned()
                        }
                    };
                    dated.push((
                        time,
                        TimelineRow {
                            key: format!("activity:{turn}:{item_key}"),
                            entry: TimelineEntry::Activity {
                                turn: turn.clone(),
                                label: String::new(),
                                items: vec![item],
                                expanded,
                                latest,
                            },
                        },
                    ));
                }
            }
            TimelineEntry::Message { ref message, .. } => {
                dated.push((message["createdAt"].as_str().unwrap_or("").to_owned(), row));
            }
            TimelineEntry::Plan(ref plan) => {
                dated.push((plan["createdAt"].as_str().unwrap_or("").to_owned(), row));
            }
            _ => footer.push(row),
        }
    }
    dated.sort_by(|(a, _), (b, _)| match (parse(a), parse(b)) {
        (Some(a), Some(b)) => a.cmp(&b),
        _ => a.cmp(b),
    });
    let mut merged: Vec<TimelineRow> = Vec::new();
    let mut first_activity = true;
    for (_, mut row) in dated {
        if let TimelineEntry::Activity {
            turn, items, label, ..
        } = &mut row.entry
        {
            if let Some(TimelineRow {
                entry:
                    TimelineEntry::Activity {
                        items: prior,
                        label: prior_label,
                        ..
                    },
                ..
            }) = merged.last_mut()
                && item_family(prior.last()) == item_family(items.first())
            {
                prior.append(items);
                *prior_label = work_label(prior);
                continue;
            }
            if first_activity {
                row.key = format!("activity:{turn}");
                first_activity = false;
            }
            *label = work_label(items);
        }
        merged.push(row);
    }
    // Only the last work disclosure gets the unqualified latest selector.
    let last_activity = merged
        .iter()
        .rposition(|row| matches!(row.entry, TimelineEntry::Activity { .. }));
    for (index, row) in merged.iter_mut().enumerate() {
        if let TimelineEntry::Activity { latest, label, .. } = &mut row.entry {
            *latest &= Some(index) == last_activity;
            if Some(index) == last_activity
                && let Some(elapsed) = &elapsed
            {
                *label = format!("{elapsed} · {label}");
            }
        }
    }
    merged.extend(footer);
    merged
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
    timeline_rows_counted(thread, expanded).0
}

/// Return the actual number of turn preparations for performance regression tests.
pub fn timeline_rows_counted(
    thread: &Value,
    expanded: &std::collections::HashMap<String, bool>,
) -> (Vec<TimelineRow>, usize) {
    let groups = group_turns(thread);
    let prepared_turns = groups.len();
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
    macro_rules! push {
        ($key:expr, $entry:expr $(,)?) => {
            rows.push(TimelineRow {
                key: $key,
                entry: $entry,
            })
        };
    }
    for (index, group) in groups.into_iter().enumerate() {
        let turn_start = rows.len();
        let TurnGroup {
            key,
            turn_id,
            messages,
            activities,
            plans,
            checkpoint,
            summary,
        } = group;
        let folded = expanded.get(&format!("fold:{key}")) == Some(&true);
        let activities: Vec<Value> = activities.into_iter().filter(visible).collect();
        let mut items = activity_items(&activities);
        for item in &mut items {
            if let ActivityItem::Tool(call) = item {
                call.expanded = expanded.get(&call.expansion_key()) == Some(&true);
            }
        }
        let empty_reasoning =
            |message: &Value| message["role"] == "reasoning" && text(&message["text"]).is_none();
        for (position, message) in messages.into_iter().enumerate() {
            if empty_reasoning(&message) || (folded && position > 0) {
                continue;
            }
            let id = message["id"]
                .as_str()
                .map_or_else(|| format!("{key}:{position}"), str::to_owned);
            push!(
                format!("message:{id}"),
                TimelineEntry::Message {
                    message,
                    show_header: true,
                    turn: (position == 0 && turn_id.is_some()).then(|| key.clone()),
                },
            );
        }
        if folded {
            continue;
        }
        let running = matches!(summary, Some((_, TurnState::Running)));
        match &summary {
            Some((label, state)) if state.is_danger() => push!(
                format!("status:{key}"),
                TimelineEntry::TurnStatus {
                    label: label.clone(),
                    danger: true,
                },
            ),
            Some((label, TurnState::Completed)) if activities.is_empty() => push!(
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
                    format!("{label} · {}", step_count(items.len()))
                }
                _ => step_count(items.len()),
            };
            push!(
                format!("activity:{key}"),
                TimelineEntry::Activity {
                    turn: key.clone(),
                    label,
                    expanded: expanded.get(&key).copied().unwrap_or(running),
                    items,
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
                push!(format!("plan:{id}"), TimelineEntry::Plan(plan));
            }
        }
        if let Some(checkpoint) = checkpoint.filter(has_files) {
            push!(
                format!("checkpoint:{key}"),
                TimelineEntry::Checkpoint {
                    turn: key.clone(),
                    checkpoint,
                    latest: latest_checkpoint == Some(index),
                },
            );
        }
        if let Some((label, TurnState::Running)) = summary {
            push!(format!("working:{key}"), TimelineEntry::Working(label));
        }
        let prepared = interleave_turn_rows(rows.split_off(turn_start));
        rows.extend(prepared);
    }
    if let Some(approval) = thread["activities"]
        .as_array()
        .and_then(|activities| pending_approval(activities))
    {
        push!(
            format!("approval:{}", approval.request_id),
            TimelineEntry::Approval(approval),
        );
    }
    if let Some(request) = thread["activities"]
        .as_array()
        .and_then(|activities| pending_user_input(activities))
    {
        push!(
            format!("user-input:{}", request.request_id),
            TimelineEntry::UserInput(request),
        );
    }
    if let Some(error) = thread["session"]["lastError"].as_str() {
        push!("error".to_owned(), TimelineEntry::Error(error.to_owned()));
    }
    let mut previous_role = None;
    for row in &mut rows {
        let role = match &mut row.entry {
            TimelineEntry::Message {
                message,
                show_header,
                ..
            } => {
                let role = message["role"].as_str().map(str::to_owned);
                *show_header = previous_role.is_none() || previous_role != role;
                role
            }
            _ => None,
        };
        previous_role = role;
    }
    (rows, prepared_turns)
}

/// Prepare only a previously rendered latest turn for contiguous message appends.
/// Raw projection filtering still walks the arrays, but historical turns are not
/// grouped, sorted, parsed or prepared. Unsafe boundaries fall back to full preparation.
pub fn timeline_tail_rows(
    thread: &Value,
    expanded: &std::collections::HashMap<String, bool>,
    old: &[TimelineRow],
    turn: &str,
) -> Option<Vec<TimelineRow>> {
    let start = old.iter().position(|row| {
        matches!(&row.entry,
        TimelineEntry::Message { turn: Some(key), .. } if key == turn)
    })?;
    let mut message_ids = std::collections::HashSet::new();
    for (index, row) in old[start..].iter().enumerate() {
        match &row.entry {
            TimelineEntry::Message {
                message,
                turn: header,
                ..
            } => {
                if header.as_ref().is_some_and(|id| id != turn)
                    || message["turnId"].as_str().is_some_and(|id| id != turn)
                    || (message["turnId"].is_null() && (index != 0 || message["role"] != "user"))
                    || (message["turnId"].is_null()
                        && thread["activities"]
                            .as_array()
                            .is_some_and(|items| items.iter().any(|item| item["turnId"].is_null())))
                {
                    return None;
                }
                message_ids.insert(message["id"].clone());
            }
            TimelineEntry::Activity { turn: key, .. }
            | TimelineEntry::Checkpoint { turn: key, .. }
                if key != turn =>
            {
                return None;
            }
            TimelineEntry::Plan(plan) if plan["turnId"] != turn => return None,
            TimelineEntry::Approval(_) | TimelineEntry::UserInput(_) | TimelineEntry::Error(_) => {
                return None;
            }
            _ => {}
        }
    }
    let mut tail = serde_json::json!({"id":thread["id"], "latestTurn":thread["latestTurn"], "session":thread["session"]});
    for key in ["messages", "activities", "proposedPlans", "checkpoints"] {
        tail[key] = Value::Array(
            thread[key]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|item| {
                    item["turnId"] == turn
                        || (key == "messages" && message_ids.contains(&item["id"]))
                })
                .cloned()
                .collect(),
        );
    }
    let mut prepared = timeline_rows(&tail, expanded);
    // Full preparation carries role continuity across turn boundaries; the
    // isolated tail starts with no previous role. Restore that boundary without
    // re-preparing any historical turn.
    if let Some(TimelineRow {
        entry:
            TimelineEntry::Message {
                message,
                show_header,
                ..
            },
        ..
    }) = prepared.first_mut()
    {
        let previous_role = start
            .checked_sub(1)
            .and_then(|index| match &old[index].entry {
                TimelineEntry::Message { message, .. } => message["role"].as_str(),
                _ => None,
            });
        *show_header = previous_role.is_none() || previous_role != message["role"].as_str();
    }
    let has_activity = prepared
        .iter()
        .any(|row| matches!(row.entry, TimelineEntry::Activity { .. }));
    let has_checkpoint = prepared
        .iter()
        .any(|row| matches!(row.entry, TimelineEntry::Checkpoint { .. }));
    let mut rows = old[..start].to_vec();
    // New work in the tail retires the previous turn's 'latest' disclosure identity.
    for row in &mut rows {
        match &mut row.entry {
            TimelineEntry::Activity { latest, .. } if has_activity => *latest = false,
            TimelineEntry::Checkpoint { latest, .. } if has_checkpoint => *latest = false,
            _ => {}
        }
    }
    rows.extend(prepared);
    Some(rows)
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
    fn context_window_snapshot_takes_the_latest_valid_reading() {
        let reading = |used: Value, max: Value| json!({"kind": "context-window.updated", "payload": {"usedTokens": used, "maxTokens": max}});
        let activities = vec![
            reading(json!(50_000), json!(200_000)),
            reading(json!(132_400), json!(200_000)),
            json!({"kind": "tool.completed", "payload": {"usedTokens": 1}}),
            reading(json!(-1), json!(200_000)),
            reading(Value::Null, json!(200_000)),
        ];
        let snapshot = context_window_snapshot(&activities).expect("a valid reading exists");
        assert_eq!(snapshot.used_tokens, 132_400.);
        assert_eq!(snapshot.remaining_tokens, Some(67_600.));
        assert_eq!(snapshot.used_percentage, Some(66.2));
        assert_eq!(snapshot.label(), "66%");
        assert_eq!(snapshot.level(), ContextWindowLevel::Normal);
        assert_eq!(
            snapshot.tooltip(),
            "Context window: 132k / 200k used, 68k left"
        );
        assert!(context_window_snapshot(&activities[2..]).is_none());
        assert!(context_window_snapshot(&[]).is_none());
    }

    #[test]
    fn context_window_percentage_needs_a_positive_max_and_caps_at_full() {
        let snapshot = |payload: Value| {
            context_window_snapshot(&[
                json!({"kind": "context-window.updated", "payload": payload}),
            ])
            .unwrap()
        };
        let unbounded = snapshot(json!({"usedTokens": 1_500}));
        assert_eq!(unbounded.used_percentage, None);
        assert_eq!(unbounded.remaining_tokens, None);
        assert_eq!(unbounded.label(), "1.5k");
        assert_eq!(unbounded.tooltip(), "Context window: 1.5k tokens used");
        assert_eq!(
            snapshot(json!({"usedTokens": 10, "maxTokens": 0})).used_percentage,
            None
        );
        let over = snapshot(json!({"usedTokens": 250_000, "maxTokens": 200_000}));
        assert_eq!(over.used_percentage, Some(100.));
        assert_eq!(over.remaining_tokens, Some(0.));
        assert_eq!(over.level(), ContextWindowLevel::Danger);
        let small = snapshot(json!({"usedTokens": 9_000, "maxTokens": 200_000}));
        assert_eq!(small.label(), "4.5%");
        assert_eq!(
            snapshot(json!({"usedTokens": 0, "maxTokens": 100})).label(),
            "0%"
        );
        let level = |used: u64| snapshot(json!({"usedTokens": used, "maxTokens": 100})).level();
        assert_eq!(level(75), ContextWindowLevel::Normal);
        assert_eq!(level(76), ContextWindowLevel::Warning);
        assert_eq!(level(90), ContextWindowLevel::Warning);
        assert_eq!(level(91), ContextWindowLevel::Danger);
    }

    #[test]
    fn context_window_tokens_use_compact_units() {
        assert_eq!(format_context_window_tokens(999.), "999");
        assert_eq!(format_context_window_tokens(1_000.), "1k");
        assert_eq!(format_context_window_tokens(132_400.), "132k");
        assert_eq!(format_context_window_tokens(1_250_000.), "1.3m");
        assert_eq!(format_context_window_tokens(f64::NAN), "0");
    }

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

    fn tool(kind: &str, id: &str, call: Option<&str>, payload: Value) -> Value {
        let mut payload = payload;
        if let Some(call) = call {
            payload["toolCallId"] = json!(call);
        }
        json!({"id": id, "kind": kind, "tone": "tool", "summary": "Tool", "payload": payload})
    }

    fn tools(items: &[ActivityItem]) -> Vec<&ToolCall> {
        items
            .iter()
            .filter_map(|item| match item {
                ActivityItem::Tool(call) => Some(call),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn activity_items_merge_a_call_with_its_completion() {
        let command = json!({"itemType": "command_execution", "title": "Ran command", "data": {"item": {"command": "/bin/zsh -lc 'cargo test'"}}});
        let mut done = command.clone();
        done["status"] = json!("completed");
        done["data"]["item"]["aggregatedOutput"] = json!("test result: ok");
        let items = activity_items(&[
            tool("tool.started", "a", Some("c1"), command.clone()),
            tool("tool.updated", "b", Some("c1"), command),
            tool("tool.completed", "c", Some("c1"), done),
        ]);
        assert_eq!(items.len(), 1, "one call is one row");
        let call = tools(&items)[0];
        assert_eq!(call.id, "c1");
        assert_eq!(call.expansion_key(), "tool:a");
        assert_eq!(call.name, "Ran command");
        assert_eq!(call.headline.as_deref(), Some("cargo test"));
        assert_eq!(call.state, ToolState::Completed);
        assert_eq!(call.body.as_deref(), Some("test result: ok"));
        assert_eq!(
            call.activities.len(),
            3,
            "the merged call keeps every activity"
        );
    }

    #[test]
    fn activity_items_keep_a_call_without_completion_running() {
        let items = activity_items(&[tool(
            "tool.started",
            "a",
            Some("c1"),
            json!({"itemType": "web_search", "title": "Web search", "data": {"input": {"query": "gpui list"}}}),
        )]);
        let call = tools(&items)[0];
        assert_eq!(call.state, ToolState::Running);
        assert_eq!(call.name, "Web search");
        assert_eq!(call.headline.as_deref(), Some("gpui list"));
        assert_eq!(call.body, None);
    }

    #[test]
    fn activity_items_mark_a_failed_completion_and_count_diff_lines() {
        let edit = json!({"itemType": "file_change", "title": "File change", "data": {"item": {"changes": [{"path": "src/lib.rs", "diff": "--- a\n+++ b\n+one\n+two\n-three"}]}}});
        let mut failed = edit.clone();
        failed["status"] = json!("failed");
        failed["detail"] = json!("patch rejected");
        let items = activity_items(&[
            tool("tool.started", "a", Some("f1"), edit),
            tool("tool.completed", "b", Some("f1"), failed),
        ]);
        let call = tools(&items)[0];
        assert_eq!(call.state, ToolState::Failed);
        assert_eq!(call.headline.as_deref(), Some("src/lib.rs"));
        assert_eq!(call.line_counts, Some((2, 1)));
        let body = call.body.as_deref().unwrap();
        assert!(body.starts_with("patch rejected") && body.contains("+one"));

        let mut errored = tool("tool.completed", "c", Some("f2"), json!({"title": "Read"}));
        errored["tone"] = json!("error");
        assert_eq!(
            tools(&activity_items(&[errored]))[0].state,
            ToolState::Failed
        );
    }

    #[test]
    fn activity_items_preserve_unknown_kinds_and_errors() {
        let unknown = json!({"id": "u", "kind": "brand.new-kind", "tone": "info", "summary": "Something new"});
        let error = json!({"id": "e", "kind": "runtime.error", "tone": "error", "summary": "Provider crashed"});
        assert_eq!(
            activity_items(&[unknown.clone(), error.clone()]),
            [ActivityItem::Activity(unknown), ActivityItem::Error(error)]
        );
    }

    #[test]
    fn activity_items_keep_first_arrival_order() {
        let read = json!({"itemType": "dynamic_tool_call", "title": "Read"});
        let grep = json!({"itemType": "dynamic_tool_call", "title": "Grep"});
        let note =
            json!({"id": "n", "kind": "task.progress", "tone": "info", "summary": "Thinking"});
        let items = activity_items(&[
            tool("tool.started", "a1", Some("a"), read.clone()),
            note.clone(),
            tool("tool.started", "b1", Some("b"), read.clone()),
            tool("tool.completed", "b2", Some("b"), read.clone()),
            tool("tool.completed", "a2", Some("a"), read.clone()),
            tool("tool.updated", "a3", Some("a"), read.clone()),
            tool("tool.updated", "x1", None, grep.clone()),
            tool("tool.completed", "x2", None, grep),
        ]);
        let ids: Vec<&str> = items
            .iter()
            .map(|item| match item {
                ActivityItem::Tool(call) => call.id.as_str(),
                ActivityItem::Activity(activity) | ActivityItem::Error(activity) => {
                    activity["id"].as_str().unwrap()
                }
            })
            .collect();
        assert_eq!(
            ids,
            ["a", "n", "b", "a", "x1"],
            "a completion stays at its call's row; a reused id after completion starts a new call; id-less updates join the open call before them"
        );
        let calls = tools(&items);
        assert_eq!(calls[0].activities.len(), 2);
        assert_eq!(calls[1].state, ToolState::Completed);
        assert_eq!(calls[2].state, ToolState::Running);
        assert_eq!(calls[2].activities.len(), 1);
        assert_eq!(calls[3].activities.len(), 2);
        assert_eq!(calls[3].state, ToolState::Completed);
        let keys: Vec<String> = calls.iter().map(|call| call.expansion_key()).collect();
        assert_eq!(keys, ["tool:a1", "tool:b1", "tool:a3", "tool:x1"]);
    }

    #[test]
    fn timeline_rows_hide_empty_thinking_and_carry_tool_expansion() {
        let thread = json!({
            "messages": [
                {"id": "r1", "role": "reasoning", "text": "  ", "turnId": "t1", "createdAt": "2026-01-01T00:00:01Z"},
                {"id": "r2", "role": "reasoning", "text": "plan", "turnId": "t1", "createdAt": "2026-01-01T00:00:02Z"},
            ],
            "activities": [
                {"id": "e1", "kind": "tool.completed", "tone": "tool", "summary": "Read", "turnId": "t1", "createdAt": "2026-01-01T00:00:03Z", "payload": {"toolCallId": "c1", "detail": "file body"}},
            ],
        });
        let rows = timeline_rows(&thread, &[("tool:e1".to_owned(), true)].into());
        let keys: Vec<&str> = rows.iter().map(|row| row.key.as_str()).collect();
        assert_eq!(keys, ["message:r2", "activity:t1"]);
        let TimelineEntry::Activity { items, .. } = &rows[1].entry else {
            panic!("the turn has an activity disclosure");
        };
        assert!(matches!(&items[0], ActivityItem::Tool(call) if call.expanded));
    }

    #[test]
    fn timeline_rows_expand_one_of_two_calls_that_reuse_an_id() {
        let read = |id: &str, kind: &str, turn: &str, at: &str| json!({"id": id, "kind": kind, "tone": "tool", "summary": "Read", "turnId": turn, "createdAt": at, "payload": {"toolCallId": "1", "title": "Read"}});
        let thread = json!({
            "messages": [],
            "activities": [
                read("a1", "tool.started", "t1", "2026-01-01T00:00:01Z"),
                read("a2", "tool.completed", "t1", "2026-01-01T00:00:02Z"),
                read("a3", "tool.started", "t1", "2026-01-01T00:00:03Z"),
                read("b1", "tool.started", "t2", "2026-01-01T00:00:04Z"),
            ],
        });
        let rows = timeline_rows(&thread, &[("tool:a3".to_owned(), true)].into());
        let calls: Vec<(String, bool)> = rows
            .iter()
            .flat_map(|row| match &row.entry {
                TimelineEntry::Activity { items, .. } => items.clone(),
                _ => Vec::new(),
            })
            .filter_map(|item| match item {
                ActivityItem::Tool(call) => Some((call.expansion_key(), call.expanded)),
                _ => None,
            })
            .collect();
        assert_eq!(
            calls,
            [
                ("tool:a1".to_owned(), false),
                ("tool:a3".to_owned(), true),
                ("tool:b1".to_owned(), false),
            ],
            "a reused call id, in the same turn or the next, gets its own expansion"
        );
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
    fn conversation_work_is_interleaved_and_never_grouped_across_a_message() {
        let thread = json!({"messages":[
            {"id":"u","role":"user","text":"go","createdAt":"2026-01-01T00:00:00Z"},
            {"id":"a","role":"assistant","text":"progress","turnId":"t","createdAt":"2026-01-01T00:00:03Z"},
            {"id":"b","role":"assistant","text":"done","turnId":"t","createdAt":"2026-01-01T00:00:06Z"}],
            "activities":[
            {"id":"c1","turnId":"t","kind":"tool.completed","createdAt":"2026-01-01T00:00:01Z","payload":{"title":"bash","toolCallId":"c1"}},
            {"id":"c2","turnId":"t","kind":"tool.completed","createdAt":"2026-01-01T00:00:02Z","payload":{"title":"bash","toolCallId":"c2"}},
            {"id":"c3","turnId":"t","kind":"tool.completed","createdAt":"2026-01-01T00:00:04Z","payload":{"title":"bash","toolCallId":"c3"}}]});
        let rows = timeline_rows(&thread, &Default::default());
        let keys: Vec<_> = rows.iter().map(|row| row.key.as_str()).collect();
        assert_eq!(
            &keys[..5],
            &[
                "message:u",
                "activity:t",
                "message:a",
                "activity:t:c3",
                "message:b"
            ]
        );
        let TimelineEntry::Activity { label, items, .. } = &rows[1].entry else {
            panic!("work row")
        };
        assert_eq!(label, "Ran 2 commands");
        assert_eq!(items.len(), 2);
        let folded = timeline_rows(&thread, &[("fold:t".into(), true)].into());
        assert_eq!(folded.len(), 1);
        assert_eq!(folded[0].key, "message:u");
        assert_eq!(
            timeline_rows(&thread, &Default::default()),
            rows,
            "expanding restores chronological work"
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
                "activity:t2",
                "message:a2",
                "working:t2",
            ],
            "a checkpoint without files renders nothing, so it has no row"
        );
        assert!(matches!(
            &rows[6].entry,
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
        assert_eq!(appended[8].key, "message:a3");
        assert_eq!(
            diff_rows(&rows, &appended),
            RowsDiff {
                replaced: 8..8,
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
                changed: vec![7]
            },
            "streaming text remeasures its row in place"
        );
        let collapsed = timeline_rows(&thread, &[("t2".to_owned(), false)].into());
        assert_eq!(diff_rows(&streamed, &collapsed).changed, vec![6]);
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
    fn timeline_rows_label_only_the_first_message_of_a_run() {
        let message = |id: &str, role: &str, second: u32| json!({"id": id, "role": role, "text": id, "turnId": "t1", "createdAt": format!("2026-01-01T00:00:{second:02}Z")});
        let thread = json!({
            "messages": [
                message("u1", "user", 1),
                message("a1", "assistant", 2),
                message("a2", "assistant", 3),
                message("a3", "assistant", 4),
                message("u2", "user", 6),
                message("a4", "assistant", 7),
            ],
            "activities": [
                {"id": "e1", "kind": "tool.completed", "tone": "tool", "summary": "Read", "turnId": "t1", "createdAt": "2026-01-01T00:00:05Z"},
            ],
        });
        let rows = timeline_rows(&thread, &Default::default());
        let headers: Vec<(&str, bool)> = rows
            .iter()
            .filter_map(|row| match &row.entry {
                TimelineEntry::Message {
                    message,
                    show_header,
                    ..
                } => Some((message["id"].as_str().unwrap(), *show_header)),
                _ => None,
            })
            .collect();
        assert_eq!(
            headers,
            [
                ("u1", true),
                ("a1", true),
                ("a2", false),
                ("a3", false),
                ("u2", true),
                ("a4", true),
            ]
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
