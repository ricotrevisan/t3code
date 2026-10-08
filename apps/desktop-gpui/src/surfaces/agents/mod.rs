//! Persisted task fold matching client-runtime/state/subagentRuntime.ts.
//! Membership comes only from the server's agentKind stamp, never tool names.
use serde_json::Value;
#[cfg(test)]
mod tests;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Pending,
    Running,
    Waiting,
    Idle,
    Completed,
    Failed,
    Cancelled,
    Interrupted,
}
impl Status {
    pub fn terminal(self) -> bool {
        matches!(
            self,
            Self::Completed | Self::Failed | Self::Cancelled | Self::Interrupted
        )
    }
    pub fn active(self) -> bool {
        matches!(self, Self::Pending | Self::Running | Self::Waiting)
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Pending | Self::Running | Self::Waiting => "Working",
            Self::Idle => "Idle · resumable",
            Self::Completed => "Completed",
            Self::Failed => "Failed",
            Self::Cancelled | Self::Interrupted => "Stopped",
        }
    }
    fn parse(value: &Value) -> Option<Self> {
        Some(match value.as_str()? {
            "pending" => Self::Pending,
            "running" => Self::Running,
            "waiting" => Self::Waiting,
            "idle" => Self::Idle,
            "completed" => Self::Completed,
            "failed" => Self::Failed,
            "cancelled" => Self::Cancelled,
            "interrupted" => Self::Interrupted,
            _ => return None,
        })
    }
}
#[derive(Clone, Debug, PartialEq)]
pub struct Agent {
    pub id: String,
    pub title: String,
    pub kind: String,
    pub status: Status,
    pub parent: Option<String>,
    pub phase: Option<u64>,
    pub phase_title: Option<String>,
    pub phases: Vec<(u64, String)>,
    pub index: u64,
    pub role: Option<String>,
    pub model: Option<String>,
    pub effort: Option<String>,
    pub progress: Option<String>,
    pub tool: Option<String>,
    pub result: Option<String>,
    pub error: Option<String>,
    pub usage: Value,
    pub activations: u64,
    pub attempt: Option<u64>,
    pub first_seen: String,
    pub started: Option<String>,
    pub completed: Option<String>,
    pub updated: String,
    pub activity_id: String,
    pub script: Option<String>,
}
fn text(v: &Value) -> Option<String> {
    v.as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}
fn bounded(v: Option<String>) -> Option<String> {
    v.map(|s| {
        if s.chars().count() > 180 {
            s.chars().take(179).collect::<String>() + "…"
        } else {
            s
        }
    })
}
impl Agent {
    fn new(id: &str, at: &str) -> Self {
        Self {
            id: id.into(),
            title: id.into(),
            kind: "subagent".into(),
            status: Status::Pending,
            parent: None,
            phase: None,
            phase_title: None,
            phases: vec![],
            index: 0,
            role: None,
            model: None,
            effort: None,
            progress: None,
            tool: None,
            result: None,
            error: None,
            usage: Value::Null,
            activations: 0,
            attempt: None,
            first_seen: at.into(),
            started: None,
            completed: None,
            updated: at.into(),
            activity_id: String::new(),
            script: None,
        }
    }
    fn metadata(&mut self, p: &Value) {
        if let Some(s) = text(&p["title"]) {
            self.title = s;
        }
        for (slot, key) in [
            (&mut self.parent, "parentAgentId"),
            (&mut self.phase_title, "phaseTitle"),
            (&mut self.role, "role"),
            (&mut self.model, "model"),
            (&mut self.effort, "effort"),
        ] {
            if let Some(s) = text(&p[key]) {
                *slot = Some(s);
            }
        }
        if let Some(i) = p["phaseIndex"].as_u64() {
            self.phase = Some(i);
        }
        if let Some(i) = p["agentIndex"].as_u64() {
            self.index = i;
        }
        self.kind = match p["taskType"].as_str() {
            Some("local_workflow") => "workflow",
            Some("subagent_batch") => "subagent_batch",
            _ if self.kind == "workflow" || self.kind == "subagent_batch" => &self.kind,
            _ if self.parent.is_some() || self.id.contains(":wf:") => "workflow_agent",
            _ => "subagent",
        }
        .into();
        if let Some(i) = p["attempt"].as_u64() {
            if self.attempt.is_some_and(|old| i > old) {
                self.result = None;
                self.error = None;
                self.completed = None;
            }
            self.attempt = Some(i);
        }
        if let Some(phases) = p["phases"].as_array() {
            let mut phases: Vec<_> = phases
                .iter()
                .filter_map(|p| Some((p["index"].as_u64()?, text(&p["title"])?)))
                .collect();
            phases.sort_by_key(|p| p.0);
            if !phases.is_empty() {
                self.phases = phases;
            }
        }
        if let Some(s) = text(&p["runHandles"]["scriptPath"]) {
            self.script = Some(s);
        }
    }
    fn transition(&mut self, status: Status, at: &str) {
        if self.status.terminal() && status.terminal() {
            return;
        }
        if (self.status.terminal() || self.status == Status::Idle)
            && matches!(status, Status::Running | Status::Pending)
        {
            self.activations += 1;
            self.result = None;
            self.error = None;
            self.completed = None;
            if status == Status::Running {
                self.started = Some(at.into());
            }
        }
        if status == Status::Running && self.started.is_none() {
            self.started = Some(at.into());
        }
        if status.terminal() && self.completed.is_none() {
            self.completed = Some(at.into());
        }
        self.status = status;
    }
    fn merge_usage(&mut self, p: &Value) {
        let u = &p["typedUsage"];
        if !u["totalTokens"]
            .as_f64()
            .is_some_and(|n| n.is_finite() && n >= 0.)
        {
            return;
        }
        if self.usage.is_null() {
            self.usage = serde_json::json!({});
        }
        for key in [
            "totalTokens",
            "inputTokens",
            "cachedInputTokens",
            "outputTokens",
            "reasoningOutputTokens",
            "toolUses",
            "durationMs",
        ] {
            if let Some(n) = u[key].as_f64().filter(|n| n.is_finite() && *n >= 0.) {
                self.usage[key] = Value::from(self.usage[key].as_f64().unwrap_or(0.).max(n));
            }
        }
    }
    pub fn activity(&self) -> Option<&str> {
        let choices = if self.status.active() {
            [&self.progress, &self.tool, &self.result, &self.error]
        } else {
            [&self.error, &self.result, &self.progress, &self.tool]
        };
        choices.into_iter().find_map(|v| v.as_deref())
    }
    pub fn elapsed(&self, now: &str) -> Option<String> {
        let start = chrono::DateTime::parse_from_rfc3339(self.started.as_deref()?).ok()?;
        let end = chrono::DateTime::parse_from_rfc3339(
            if matches!(self.status, Status::Running | Status::Waiting) {
                now
            } else {
                self.completed.as_deref().unwrap_or(now)
            },
        )
        .ok()?;
        let seconds = (end - start).num_seconds().max(0);
        Some(if seconds < 60 {
            format!("{seconds}s")
        } else if seconds < 3600 {
            format!("{}m {:02}s", seconds / 60, seconds % 60)
        } else {
            format!("{}h {:02}m", seconds / 3600, seconds / 60 % 60)
        })
    }
}
/// Revision-keyed presentation cache. Elapsed ticks never replay persisted history.
#[derive(Default)]
pub struct RosterCache {
    key: Option<(u64, u64, String)>,
    roster: std::rc::Rc<Vec<Agent>>,
}
impl RosterCache {
    pub fn get(
        &mut self,
        generation: u64,
        revision: u64,
        thread: &Value,
    ) -> std::rc::Rc<Vec<Agent>> {
        let key = (
            generation,
            revision,
            thread["id"].as_str().unwrap_or_default().to_owned(),
        );
        if self.key.as_ref() != Some(&key) {
            self.roster = std::rc::Rc::new(fold(thread));
            self.key = Some(key);
        }
        self.roster.clone()
    }
    pub fn live(&self) -> bool {
        self.roster
            .iter()
            .any(|a| matches!(a.status, Status::Running | Status::Waiting))
    }
}

/// Completion can create a member when its start fell outside retained history.
pub fn fold(thread: &Value) -> Vec<Agent> {
    let mut agents: Vec<Agent> = vec![];
    let mut indices = std::collections::HashMap::new();
    for row in thread["activities"].as_array().into_iter().flatten() {
        let kind = row["kind"].as_str().unwrap_or_default();
        if !matches!(
            kind,
            "task.started" | "task.progress" | "task.updated" | "task.completed" | "tool.progress"
        ) {
            continue;
        }
        let p = &row["payload"];
        let Some(id) = text(&p["taskId"]) else {
            continue;
        };
        let at = row["createdAt"].as_str().unwrap_or_default();
        let index = indices.get(&id).copied();
        if index.is_none() && (kind == "tool.progress" || p["agentKind"] != "agent") {
            continue;
        }
        let index = index.unwrap_or_else(|| {
            agents.push(Agent::new(&id, at));
            let index = agents.len() - 1;
            indices.insert(id.clone(), index);
            index
        });
        let existed = agents[index].activations > 0;
        let a = &mut agents[index];
        a.metadata(p);
        a.activity_id = row["id"].as_str().unwrap_or_default().into();
        match kind {
            "task.started" => {
                if a.activations == 0 && !a.status.terminal() {
                    a.activations = 1;
                    a.started = Some(at.into());
                    a.status = Status::Running;
                } else if a.status == Status::Idle {
                    a.transition(Status::Running, at);
                }
                if a.title == a.id
                    && let Some(s) = text(&p["detail"])
                {
                    a.title = s;
                }
            }
            "task.progress" => {
                a.activations = a.activations.max(1);
                if let Some(status) = Status::parse(&p["status"]) {
                    a.transition(status, at);
                } else if (p["usageSnapshot"] != true || !existed)
                    && !a.status.terminal()
                    && a.status != Status::Idle
                {
                    a.transition(Status::Running, at);
                }
                if let Some(s) = bounded(text(&p["summary"])) {
                    a.progress = Some(s);
                }
                if let Some(s) = text(&p["lastToolName"]) {
                    a.tool = Some(format!("▸ {s}"));
                }
                if let Some(s) = bounded(text(&p["error"])) {
                    a.error = Some(s);
                }
                a.merge_usage(p);
            }
            "task.updated" => {
                a.activations = a.activations.max(1);
                if let Some(s) = bounded(text(&p["detail"])) {
                    a.progress = Some(s);
                }
                let was_terminal = a.status.terminal();
                if let Some(status) = Status::parse(&p["status"]) {
                    a.transition(status, at);
                }
                if !was_terminal
                    && a.status.terminal()
                    && let Some(s) = text(&p["endedAt"])
                {
                    a.completed = Some(s);
                }
                if let Some(s) = bounded(text(&p["error"])) {
                    a.error = Some(s);
                }
            }
            "task.completed" => {
                a.activations = a.activations.max(1);
                let status = match p["status"].as_str() {
                    Some("failed") => Status::Failed,
                    Some("stopped") => Status::Interrupted,
                    _ => Status::Completed,
                };
                a.transition(status, at);
                let slot = if a.status == Status::Failed {
                    &mut a.error
                } else {
                    &mut a.result
                };
                if slot.is_none() {
                    *slot = bounded(text(&p["summary"]).or_else(|| text(&p["detail"])));
                }
                a.merge_usage(p);
            }
            "tool.progress" => {
                if let Some(s) = text(&p["toolName"]) {
                    a.tool = Some(format!("▸ {s}"));
                }
            }
            _ => unreachable!(),
        }
        a.updated = at.into();
    }
    let settled: std::collections::HashMap<_, _> = agents
        .iter()
        .filter(|a| a.kind == "workflow" && a.status.terminal())
        .map(|a| {
            (
                a.id.clone(),
                (a.status, a.completed.clone(), a.updated.clone()),
            )
        })
        .collect();
    // Match derivePhase / deriveAgentSessionLive: a ready session with an
    // explicit null background signal has no surviving child processes. An
    // omitted signal is an older server, not authoritative absence.
    let session_live = match thread["session"]["status"].as_str() {
        Some("starting" | "running") => true,
        Some("idle" | "ready") => thread
            .get("backgroundLiveness")
            .is_none_or(|v| !v.is_null()),
        _ => false,
    };
    for a in &mut agents {
        if a.status.active()
            && let Some((status, completed, updated)) =
                a.parent.as_ref().and_then(|id| settled.get(id))
        {
            a.status = if *status == Status::Completed {
                Status::Completed
            } else {
                Status::Interrupted
            };
            a.completed = completed.clone().or_else(|| Some(updated.clone()));
        }
        if !session_live && a.status.active() {
            a.status = Status::Interrupted;
            a.completed = Some(a.updated.clone());
        }
    }
    if agents.len() > 100 {
        agents.sort_by(|a, b| {
            let rank = |a: &Agent| {
                if a.status.active() {
                    0
                } else if a.status == Status::Idle {
                    1
                } else {
                    2
                }
            };
            rank(a).cmp(&rank(b)).then(b.updated.cmp(&a.updated))
        });
        agents.truncate(100);
    }
    agents.sort_by(|a, b| a.first_seen.cmp(&b.first_seen).then(a.id.cmp(&b.id)));
    agents
}
/// Coordinators aggregate member usage; exclude them when members exist.
pub fn totals(agents: &[Agent]) -> (usize, usize, usize, f64) {
    let mut counts = (0, 0, 0, 0.);
    for a in agents {
        if a.kind == "workflow" && agents.iter().any(|m| m.parent.as_deref() == Some(&a.id)) {
            continue;
        }
        if a.status.active() {
            counts.0 += 1;
        } else if a.status == Status::Idle {
            counts.1 += 1;
        } else {
            counts.2 += 1;
        }
        counts.3 += a.usage["totalTokens"].as_f64().unwrap_or(0.);
    }
    counts
}
