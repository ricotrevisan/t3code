//! Sidebar membership derives timed snoozes locally; expiry needs no server event.
use crate::shell::Thread;
use chrono::{DateTime, Utc};
use serde_json::Value;

#[derive(Default)]
pub struct State {
    pub capabilities: Value,
    pub cleanup: CleanupState,
    pub project_settings_target: Option<(String, String)>,
    pub priority: bool,
    pub scroll: gpui_kit::ScrollHandle,
    pub selected_bounds:
        std::rc::Rc<std::cell::RefCell<Option<gpui_kit::Bounds<gpui_kit::Pixels>>>>,
    pub scope_picker: bool,
    local_path: Option<std::path::PathBuf>,
    pub project_scope: Option<String>,
    pub custom_snooze_target: Option<String>,
    pub custom_snooze_environment: Option<String>,
    pub custom_snooze_generation: Option<u64>,
    pub rename_target: Option<(String, String)>,
    pub rename_busy: Option<u64>,
    pub search_query: String,
    pub search_environment: String,
    pub search_epoch: u64,
    pub search_loading: bool,
    pub search_error: Option<String>,
    pub search_index: usize,
    pub order_busy: bool,
    pub order_generation: u64,
    pub selection_environment: String,
    pub selection: Vec<String>,
    pub selection_anchor: Option<String>,
    pub bulk_delete_review: Option<(u64, String, Vec<String>)>,
    pub bulk_snooze_review: Option<Vec<String>>,
    pub bulk_busy: bool,
    pub bulk_generation: u64,
    /// Search contracts expose timestamps, not message IDs. Timeline resolves
    /// this exact source/timestamp pair after the thread projection arrives.
    pub message_anchor: Option<MessageAnchor>,
    groups: std::cell::RefCell<GroupCache>,
    row_cache: std::cell::RefCell<RowCache>,
    pub read_at: std::collections::HashMap<(String, String), DateTime<Utc>>,
    pub woke_acknowledged: std::collections::HashMap<(String, String), DateTime<Utc>>,
    pub marked_unread: std::collections::HashSet<(String, String)>,
}

#[derive(Clone)]
pub struct Drag {
    pub id: String,
    pub title: String,
    pub environment: String,
    pub generation: u64,
}

#[derive(Default)]
pub struct CleanupState {
    pub generation: u64,
    pub target: Option<(String, String)>,
    pub epoch: u64,
    pub busy: bool,
    pub review: Option<CleanupReview>,
    pub result: Option<CleanupResult>,
    pub selected: Vec<String>,
    pub error: Option<String>,
}
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CleanupReview {
    pub review_id: String,
    pub thread_id: String,
    pub title: String,
    pub actions: Vec<CleanupAction>,
}
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CleanupAction {
    pub id: String,
    pub title: String,
    pub detail: String,
    pub blocked_reason: Option<String>,
}
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CleanupResult {
    pub thread_id: String,
    pub actions: Vec<CleanupOutcome>,
}
#[derive(serde::Deserialize)]
pub struct CleanupOutcome {
    pub id: String,
    pub status: String,
    pub detail: String,
}

#[derive(Default, serde::Serialize, serde::Deserialize)]
#[serde(default)]
struct LocalState {
    priority: bool,
    read_at: Vec<(String, String, DateTime<Utc>)>,
    marked_unread: Vec<(String, String)>,
    woke_acknowledged: Vec<(String, String, DateTime<Utc>)>,
}

pub struct RowData {
    pub status: Option<Status>,
    pub meta: Option<String>,
    pub age: Option<String>,
}
struct CachedRow {
    thread: Thread,
    queued: bool,
    clock: (i64, bool),
    data: std::rc::Rc<RowData>,
}
#[derive(Default)]
struct RowCache {
    environment: String,
    rows: std::collections::HashMap<String, CachedRow>,
    preparations: usize,
}

#[derive(Default)]
struct GroupCache {
    environment: String,
    identity: Option<(Option<u64>, usize, i64)>,
    expires_at: Option<DateTime<Utc>>,
    rows: std::collections::HashMap<(String, View), [Vec<usize>; 2]>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MessageAnchor {
    pub environment: String,
    pub thread_id: String,
    pub source: String,
    pub message_created_at: String,
}

/// Resolve the wire search timestamp to a real message after its thread loads.
/// Never manufacture a message ID or fall back to the timeline tail.
pub fn resolve_search_anchor(anchor: &MessageAnchor, thread: &Value) -> Option<String> {
    if thread["id"].as_str() != Some(anchor.thread_id.as_str()) {
        return None;
    }
    let at = DateTime::parse_from_rfc3339(&anchor.message_created_at).ok()?;
    thread["messages"].as_array()?.iter().find(|message| {
        message["role"].as_str() == Some(anchor.source.as_str())
            && message["createdAt"]
                .as_str()
                .and_then(|value| DateTime::parse_from_rfc3339(value).ok())
                == Some(at)
    })?["id"]
        .as_str()
        .map(str::to_owned)
}

impl State {
    /// Reveal only an offscreen row, using the next layout's bounds. Aligning
    /// every selection to the top hides shelf headers even when it was visible.
    pub fn scroll_selected(&self, window: &mut gpui_kit::Window, _cx: &mut gpui_kit::App) {
        let selected = self.selected_bounds.clone();
        let scroll = self.scroll.clone();
        window.on_next_frame(move |window, _| {
            let Some(row) = *selected.borrow() else {
                return;
            };
            let viewport = scroll.bounds();
            let delta = if row.top() < viewport.top() {
                viewport.top() - row.top()
            } else if row.bottom() > viewport.bottom() {
                viewport.bottom() - row.bottom()
            } else {
                return;
            };
            let mut offset = scroll.offset();
            offset.y += delta;
            scroll.set_offset(offset);
            window.refresh();
        });
    }

    pub fn retain_rows(&self, threads: &[Thread], environment: &str) {
        let mut cache = self.row_cache.borrow_mut();
        if cache.environment != environment {
            cache.rows.clear();
            cache.environment = environment.to_owned();
        }
        let ids: std::collections::HashSet<_> =
            threads.iter().map(|thread| thread.id.as_str()).collect();
        cache.rows.retain(|id, _| ids.contains(id.as_str()));
    }

    pub fn prepared_row(
        &self,
        thread: &Thread,
        queued: bool,
        now: DateTime<Utc>,
    ) -> std::rc::Rc<RowData> {
        let mut cache = self.row_cache.borrow_mut();
        let clock = (now.timestamp() / 60, snoozed(thread, now));
        if let Some(row) = cache.rows.get(&thread.id)
            && row.thread == *thread
            && row.queued == queued
            && row.clock == clock
        {
            return row.data.clone();
        }
        let data = std::rc::Rc::new(RowData {
            status: primary_status(thread, queued),
            meta: thread_meta(thread),
            age: row_time(thread, now),
        });
        cache.preparations += 1;
        cache.rows.insert(
            thread.id.clone(),
            CachedRow {
                thread: thread.clone(),
                queued,
                clock,
                data: data.clone(),
            },
        );
        data
    }

    pub fn row_preparation_count(&self) -> usize {
        self.row_cache.borrow().preparations
    }

    pub fn woke(&self, environment: &str, thread: &Thread, now: DateTime<Utc>) -> bool {
        let key = (environment.to_owned(), thread.id.clone());
        woke_at(thread, now).is_some_and(|at| {
            self.read_at.get(&key).is_none_or(|read| *read < at)
                && self.woke_acknowledged.get(&key).is_none_or(|ack| *ack < at)
        })
    }
    pub fn reconcile_selection(&mut self, environment: &str, order: &[String]) {
        if self.selection_environment != environment {
            self.selection.clear();
            self.selection_anchor = None;
            self.selection_environment = environment.to_owned();
        }
        self.selection.retain(|id| order.contains(id));
        if self
            .selection_anchor
            .as_ref()
            .is_some_and(|id| !order.contains(id))
        {
            self.selection_anchor = None;
        }
    }

    /// Only projection changes and a coarse minute boundary invalidate group
    /// membership/order. Unrelated composer/streaming notifies reuse indices.
    pub fn grouped<'a>(
        &self,
        threads: &'a [Thread],
        sequence: Option<u64>,
        project: &str,
        view: View,
        now: DateTime<Utc>,
        environment: &str,
    ) -> [(&'static str, &'static str, Vec<&'a Thread>); 2] {
        let mut cache = self.groups.borrow_mut();
        let identity = (sequence, threads.len(), now.timestamp() / 60);
        if cache.environment != environment
            || cache.identity != Some(identity)
            || cache.expires_at.is_some_and(|at| at <= now)
        {
            cache.rows.clear();
            cache.identity = Some(identity);
            cache.environment = environment.to_owned();
            cache.expires_at = threads
                .iter()
                .filter_map(|thread| thread.snoozed_until.as_deref())
                .filter_map(|at| DateTime::parse_from_rfc3339(at).ok())
                .map(|at| at.with_timezone(&Utc))
                .filter(|at| *at > now)
                .min();
        }
        let key = (project.to_owned(), view);
        let indices = cache.rows.entry(key).or_insert_with(|| {
            let groups = grouped(threads, project, view, now);
            let lookup: std::collections::HashMap<_, _> = threads
                .iter()
                .enumerate()
                .map(|(index, thread)| (thread.id.as_str(), index))
                .collect();
            groups.map(|(_, _, rows)| {
                rows.into_iter()
                    .filter_map(|thread| lookup.get(thread.id.as_str()).copied())
                    .collect()
            })
        });
        [
            (
                "pinned",
                "Pinned",
                indices[0].iter().map(|index| &threads[*index]).collect(),
            ),
            (
                "active",
                group_label(view),
                indices[1].iter().map(|index| &threads[*index]).collect(),
            ),
        ]
    }

    pub fn priority_groups<'a>(
        &self,
        threads: &'a [Thread],
        revision: u64,
        now: DateTime<Utc>,
        environment: &str,
    ) -> Vec<(&'static str, &'static str, Vec<&'a Thread>)> {
        let project = self.project_scope.as_deref().unwrap_or("");
        let active = self.grouped(
            threads,
            Some(revision),
            project,
            View::Active,
            now,
            environment,
        );
        let snoozed = self.grouped(
            threads,
            Some(revision),
            project,
            View::Snoozed,
            now,
            environment,
        );
        let settled = self.grouped(
            threads,
            Some(revision),
            project,
            View::Settled,
            now,
            environment,
        );
        let mut sleeping: Vec<_> = snoozed.into_iter().flat_map(|(_, _, rows)| rows).collect();
        sleeping.sort_by(|left, right| {
            left.snoozed_until
                .cmp(&right.snoozed_until)
                .then_with(|| left.id.cmp(&right.id))
        });
        let mut finished: Vec<_> = settled.into_iter().flat_map(|(_, _, rows)| rows).collect();
        finished.sort_by(|left, right| {
            last_activity(right)
                .cmp(&last_activity(left))
                .then_with(|| left.id.cmp(&right.id))
        });
        vec![
            active[0].clone(),
            active[1].clone(),
            ("snoozed", "Snoozed", sleeping),
            ("settled", "Settled", finished),
        ]
    }

    /// Tests pass a sandbox path; production opts in only on normal startup.
    pub fn load_local(&mut self, path: std::path::PathBuf) -> Result<(), String> {
        let local: LocalState = match std::fs::read(&path) {
            Ok(bytes) => serde_json::from_slice(&bytes).map_err(|error| error.to_string())?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => LocalState::default(),
            Err(error) => return Err(error.to_string()),
        };
        self.priority = local.priority;
        self.read_at = local
            .read_at
            .into_iter()
            .map(|(environment, thread, at)| ((environment, thread), at))
            .collect();
        self.marked_unread = local.marked_unread.into_iter().collect();
        self.woke_acknowledged = local
            .woke_acknowledged
            .into_iter()
            .map(|(environment, thread, at)| ((environment, thread), at))
            .collect();
        self.local_path = Some(path);
        Ok(())
    }

    /// Flush at shutdown; no synchronous disk writes on streaming/UI renders.
    pub fn save_local(&self) -> Result<(), String> {
        let Some(path) = &self.local_path else {
            return Ok(());
        };
        let local = LocalState {
            priority: self.priority,
            read_at: self
                .read_at
                .iter()
                .map(|((environment, thread), at)| (environment.clone(), thread.clone(), *at))
                .collect(),
            marked_unread: self.marked_unread.iter().cloned().collect(),
            woke_acknowledged: self
                .woke_acknowledged
                .iter()
                .map(|((environment, thread), at)| (environment.clone(), thread.clone(), *at))
                .collect(),
        };
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        let pending = path.with_extension("json.pending");
        std::fs::write(
            &pending,
            serde_json::to_vec(&local).map_err(|error| error.to_string())?,
        )
        .map_err(|error| error.to_string())?;
        std::fs::rename(pending, path).map_err(|error| error.to_string())
    }

    pub fn visit(&mut self, environment: &str, thread: &Thread) {
        let key = (environment.to_owned(), thread.id.clone());
        self.marked_unread.remove(&key);
        self.read_at.insert(key, Utc::now());
    }

    pub fn unread(&self, environment: &str, thread: &Thread) -> bool {
        let key = (environment.to_owned(), thread.id.clone());
        self.marked_unread.contains(&key)
            || thread
                .latest_turn
                .as_ref()
                .and_then(|turn| turn.completed_at.as_deref())
                .and_then(|at| DateTime::parse_from_rfc3339(at).ok())
                .is_some_and(|at| self.read_at.get(&key).is_some_and(|read| at > *read))
    }
}

/// Shared availability for the header and real sidebar row menu. A missing
/// projection is an unsent draft, never a wildcard for lifecycle operations.
pub fn thread_action_policy(
    thread: Option<&Thread>,
    action: &str,
    now: DateTime<Utc>,
    capabilities: &Value,
) -> bool {
    let Some(thread) = thread else {
        return matches!(action, "rename" | "discard");
    };
    let running = thread
        .session
        .as_ref()
        .is_some_and(|session| matches!(session.status.as_str(), "running" | "starting"));
    let supported = |name: &str| capabilities[name].as_bool() == Some(true);
    if thread.archived_at.is_some()
        && matches!(
            action,
            "settle" | "unsettle" | "snooze" | "unsnooze" | "pin" | "unpin"
        )
    {
        return false;
    }
    match action {
        "discard" => false,
        "settle" | "unsettle" => {
            supported("threadSettlement")
                && (action == "unsettle"
                    || (!running
                        && !thread.has_pending_approvals
                        && !thread.has_pending_user_input
                        && !queued_turn_start(thread, now)))
                && action_applies(thread, action, now)
        }
        "pin" | "unpin" => supported("threadPinning") && action_applies(thread, action, now),
        "snooze" | "unsnooze" => {
            supported("threadSnooze")
                && (action == "unsnooze"
                    || (!thread.has_pending_approvals
                        && !thread.has_pending_user_input
                        && !queued_turn_start(thread, now)))
                && action_applies(thread, action, now)
        }
        "archive" => !running && action_applies(thread, action, now),
        "regenerate-title" => {
            supported("threadTitleRegeneration") && thread.title_regeneration.is_none()
        }
        "new-thread-on-branch" | "copy-branch" => thread.branch.is_some(),
        _ => action_applies(thread, action, now),
    }
}

/// Client twin of the bounded adoption-window invariant from threadSettled.ts.
pub fn queued_turn_start(thread: &Thread, now: DateTime<Utc>) -> bool {
    if thread
        .session
        .as_ref()
        .is_some_and(|session| session.status == "error")
    {
        return false;
    }
    let Some(message) = thread
        .latest_user_message_at
        .as_deref()
        .and_then(|at| DateTime::parse_from_rfc3339(at).ok())
    else {
        return false;
    };
    if (now - message.with_timezone(&Utc))
        .num_seconds()
        .unsigned_abs()
        > 120
    {
        return false;
    }
    thread.latest_turn.as_ref().is_none_or(|turn| {
        [&turn.requested_at, &turn.started_at, &turn.completed_at]
            .into_iter()
            .filter_map(|value| value.as_deref())
            .filter_map(|at| DateTime::parse_from_rfc3339(at).ok())
            .all(|at| at < message)
    })
}

/// Single-write variant retained for callers that only need a fractional key.
/// Interactive ordering uses `move_assignments`, including section repair.
pub fn move_key(rows: &[&Thread], target: &str, forward: bool, pinned: bool) -> Option<String> {
    let from = rows.iter().position(|thread| thread.id == target)?;
    let to = if forward {
        from.checked_add(1)?
    } else {
        from.checked_sub(1)?
    };
    if to >= rows.len() {
        return None;
    }
    let mut order = rows.to_vec();
    let moved = order.remove(from);
    order.insert(to, moved);
    let key = |thread: &Thread| {
        if pinned {
            thread.pin_order_key.clone()
        } else {
            thread.active_order_key.clone()
        }
    };
    let before = if to > 0 {
        Some(key(order[to - 1])?)
    } else {
        None
    };
    let after = if to + 1 < order.len() {
        Some(key(order[to + 1])?)
    } else {
        None
    };
    key_between(before.as_deref(), after.as_deref())
}

/// Reference threadSort planner: repair only the visible section and reserve
/// keys belonging to hidden rows. Accepted writes remain authoritative even
/// if a subsequent write fails.
pub fn order_assignments(
    rows: &[&Thread],
    all: &[Thread],
    target: &str,
    pinned: bool,
) -> Vec<(String, String)> {
    fn order_key(thread: &Thread, pinned: bool) -> Option<&str> {
        if pinned {
            thread.pin_order_key.as_deref()
        } else {
            thread.active_order_key.as_deref()
        }
    }
    let key = |thread| order_key(thread, pinned);
    let Some(index) = rows.iter().position(|row| row.id == target) else {
        return vec![];
    };
    let reserved: std::collections::HashSet<_> = all
        .iter()
        .filter(|thread| !rows.iter().any(|row| row.id == thread.id))
        .filter_map(key)
        .collect();
    let before = index.checked_sub(1).map(|index| rows[index]);
    let after = rows.get(index + 1).copied();
    if before.is_none_or(|row| key(row).is_some()) && after.is_none_or(|row| key(row).is_some()) {
        let mut next = key_between(before.and_then(key), after.and_then(key));
        while let Some(value) = &next {
            if !reserved.contains(value.as_str()) {
                return vec![(target.into(), value.clone())];
            }
            next = key_between(Some(value), after.and_then(key));
        }
    }
    let count = rows.len() + reserved.len();
    let mut width = 2;
    let mut space = 26_u64.pow(width);
    while space <= (count as u64 + 1) * 2 {
        width += 1;
        space = 26_u64.pow(width);
    }
    let keys = (1..=count)
        .map(|index| {
            let mut value = (space as f64 / (count + 1) as f64 * index as f64).round() as u64;
            if value.is_multiple_of(26) {
                value += 1;
            }
            let mut bytes = vec![b'a'; width as usize];
            for digit in bytes.iter_mut().rev() {
                *digit += (value % 26) as u8;
                value /= 26;
            }
            String::from_utf8(bytes).expect("base-26 ASCII")
        })
        .filter(|value| !reserved.contains(value.as_str()));
    rows.iter()
        .zip(keys)
        .filter_map(|(row, value)| {
            (key(row) != Some(value.as_str())).then(|| (row.id.clone(), value))
        })
        .collect()
}

pub fn move_assignments(
    rows: &[&Thread],
    all: &[Thread],
    target: &str,
    forward: bool,
    pinned: bool,
) -> Option<Vec<(String, String)>> {
    let from = rows.iter().position(|row| row.id == target)?;
    let to = if forward {
        from.checked_add(1)?
    } else {
        from.checked_sub(1)?
    };
    if to >= rows.len() {
        return None;
    }
    let mut order = rows.to_vec();
    let moved = order.remove(from);
    order.insert(to, moved);
    Some(order_assignments(&order, all, target, pinned))
}

pub fn key_between(before: Option<&str>, after: Option<&str>) -> Option<String> {
    let a = before.unwrap_or("");
    let b = after.unwrap_or("");
    let valid = |key: &str| {
        key.is_empty()
            || (key.bytes().all(|digit| digit.is_ascii_lowercase()) && !key.ends_with('a'))
    };
    if !valid(a) || !valid(b) || (!b.is_empty() && a >= b) {
        return None;
    }
    fn midpoint(a: &[u8], b: &[u8]) -> Vec<u8> {
        if !b.is_empty() {
            let prefix = b
                .iter()
                .enumerate()
                .take_while(|(index, digit)| a.get(*index).copied().unwrap_or(b'a') == **digit)
                .count();
            if prefix > 0 {
                let mut key = b[..prefix].to_vec();
                key.extend(midpoint(a.get(prefix..).unwrap_or_default(), &b[prefix..]));
                return key;
            }
        }
        let low = a.first().map_or(0, |digit| digit - b'a');
        let high = b.first().map_or(26, |digit| digit - b'a');
        if high - low > 1 {
            return vec![b'a' + (low + high).div_ceil(2)];
        }
        if b.len() > 1 {
            return vec![b[0]];
        }
        let mut key = vec![b'a' + low];
        key.extend(midpoint(a.get(1..).unwrap_or_default(), &[]));
        key
    }
    String::from_utf8(midpoint(a.as_bytes(), b.as_bytes())).ok()
}

/// Byte ranges stay on Unicode boundaries even when lowercase expands a
/// character. The original snippet, not a lowercase copy, is rendered.
pub fn match_ranges(text: &str, query: &str) -> Vec<std::ops::Range<usize>> {
    if query.is_empty() {
        return vec![];
    }
    let needle = query.to_lowercase();
    let boundaries: Vec<_> = text
        .char_indices()
        .map(|(index, _)| index)
        .chain(std::iter::once(text.len()))
        .collect();
    let mut ranges = Vec::new();
    let mut next = 0;
    for (index, start) in boundaries.iter().copied().enumerate() {
        if start < next {
            continue;
        }
        for end in boundaries.iter().copied().skip(index + 1) {
            let value = text[start..end].to_lowercase();
            if value == needle {
                ranges.push(start..end);
                next = end;
                break;
            }
            if value.len() >= needle.len() {
                break;
            }
        }
    }
    ranges
}

pub fn row_time(thread: &Thread, now: DateTime<Utc>) -> Option<String> {
    if snoozed(thread, now) {
        let until = DateTime::parse_from_rfc3339(thread.snoozed_until.as_deref()?).ok()?;
        return Some(format!(
            "Wakes {}",
            until.with_timezone(&chrono::Local).format("%a %H:%M")
        ));
    }
    last_activity(thread).map(|at| relative_time(at, now))
}

/// Resolve calendar presets in local time, preserving DST and deduplicating
/// Sunday's Tomorrow/Next week. Ambiguous/nonexistent wall times are omitted.
pub fn snooze_presets(now: DateTime<chrono::Local>) -> Vec<(&'static str, DateTime<Utc>)> {
    use chrono::{Datelike, TimeZone};
    let mut choices = vec![
        (
            "In 1 hour",
            (now + chrono::Duration::hours(1)).with_timezone(&Utc),
        ),
        (
            "In 3 hours",
            (now + chrono::Duration::hours(3)).with_timezone(&Utc),
        ),
    ];
    let at = |date: chrono::NaiveDate, hour| {
        chrono::Local
            .from_local_datetime(&date.and_hms_opt(hour, 0, 0)?)
            .single()
            .map(|value| value.with_timezone(&Utc))
    };
    if let Some(evening) = at(now.date_naive(), 18)
        && evening > now.with_timezone(&Utc) + chrono::Duration::hours(1)
    {
        choices.push(("This evening", evening));
    }
    if let Some(tomorrow) = now.date_naive().succ_opt().and_then(|date| at(date, 9)) {
        choices.push(("Tomorrow", tomorrow));
    }
    let days = match now.weekday().num_days_from_monday() {
        0 => 7,
        day => 7 - day,
    };
    if let Some(next) = now
        .date_naive()
        .checked_add_days(chrono::Days::new(days.into()))
        .and_then(|date| at(date, 9))
        && !choices.iter().any(|(_, value)| *value == next)
    {
        choices.push(("Next week", next));
    }
    choices
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub enum View {
    Active,
    Settled,
    Snoozed,
    Archived,
}

fn timestamp(value: Option<&str>) -> Option<DateTime<Utc>> {
    value
        .and_then(|value| DateTime::parse_from_rfc3339(value).ok())
        .map(|at| at.with_timezone(&Utc))
}

pub fn raised_hand(thread: &Thread) -> bool {
    thread.has_pending_approvals
        || thread.has_pending_user_input
        || (thread
            .session
            .as_ref()
            .is_some_and(|session| session.status == "error")
            && (thread.snoozed_at.is_none()
                || timestamp(thread.session_updated_at.as_deref())
                    .zip(timestamp(thread.snoozed_at.as_deref()))
                    .is_some_and(|(error, snooze)| error > snooze)))
        || thread.latest_turn.as_ref().is_some_and(|turn| {
            turn.state == "completed"
                && timestamp(thread.snoozed_at.as_deref()).is_some()
                && timestamp(turn.completed_at.as_deref()) > timestamp(thread.snoozed_at.as_deref())
        })
}

pub fn woke_at(thread: &Thread, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
    let until = timestamp(thread.snoozed_until.as_deref())?;
    if raised_hand(thread) {
        if let Some(turn) = &thread.latest_turn
            && turn.state == "completed"
            && timestamp(thread.snoozed_at.as_deref()).is_some()
            && timestamp(turn.completed_at.as_deref()) > timestamp(thread.snoozed_at.as_deref())
        {
            return timestamp(turn.completed_at.as_deref());
        }
        return timestamp(thread.session_updated_at.as_deref())
            .or_else(|| timestamp(thread.snoozed_at.as_deref()));
    }
    (until <= now).then_some(until)
}

pub fn snoozed(thread: &Thread, now: DateTime<Utc>) -> bool {
    if raised_hand(thread) {
        return false;
    }
    thread
        .snoozed_until
        .as_deref()
        .and_then(|value| DateTime::parse_from_rfc3339(value).ok())
        .is_some_and(|until| until > now)
}

pub fn belongs(thread: &Thread, view: View, now: DateTime<Utc>) -> bool {
    if thread.archived_at.is_some() {
        return view == View::Archived;
    }
    if view == View::Archived {
        return false;
    }
    if snoozed(thread, now) {
        return view == View::Snoozed;
    }
    if view == View::Snoozed {
        return false;
    }
    // Automatic settlement needs the full fork policy; this slice only classifies explicit overrides.
    match thread.settled_override.as_deref() {
        Some("settled") => view == View::Settled,
        _ => view == View::Active,
    }
}

pub fn expiry(threads: &[Thread], now: DateTime<Utc>) -> Option<std::time::Duration> {
    threads
        .iter()
        .filter_map(|thread| thread.snoozed_until.as_deref())
        .filter_map(|value| DateTime::parse_from_rfc3339(value).ok())
        .filter(|until| *until > now)
        .min()
        .and_then(|until| (until.with_timezone(&Utc) - now).to_std().ok())
}

/// The one live status a thread row shows beside its title.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Status {
    Approval,
    Input,
    Working,
    Plan,
    Error,
}

impl Status {
    pub fn label(self) -> &'static str {
        match self {
            Self::Approval => "Approval",
            Self::Input => "Input",
            Self::Working => "Working",
            Self::Plan => "Plan",
            Self::Error => "Error",
        }
    }
}

/// The row status for `thread`, by priority: Approval > Input > Working > Plan.
/// A thread with `queued` messages counts as Working until they have gone out.
pub fn primary_status(thread: &Thread, queued: bool) -> Option<Status> {
    if thread.has_pending_approvals {
        Some(Status::Approval)
    } else if thread.has_pending_user_input {
        Some(Status::Input)
    } else if queued
        || thread
            .session
            .as_ref()
            .is_some_and(|session| matches!(session.status.as_str(), "running" | "starting"))
    {
        Some(Status::Working)
    } else if thread
        .session
        .as_ref()
        .is_some_and(|session| session.status == "error" || session.last_error.is_some())
    {
        Some(Status::Error)
    } else if thread.has_actionable_proposed_plan {
        Some(Status::Plan)
    } else {
        None
    }
}

/// When the thread last saw activity: its latest user message, else its last
/// update, else its creation. `None` when the server sent none of them.
pub fn last_activity(thread: &Thread) -> Option<DateTime<Utc>> {
    [
        &thread.latest_user_message_at,
        &thread.updated_at,
        &thread.created_at,
    ]
    .into_iter()
    .filter_map(|value| value.as_deref())
    .find_map(|value| DateTime::parse_from_rfc3339(value).ok())
    .map(|at| at.with_timezone(&Utc))
}

/// Compact age of `at` relative to `now`: "now", "5m", "3h", "2d", "3w".
pub fn relative_time(at: DateTime<Utc>, now: DateTime<Utc>) -> String {
    let seconds = (now - at).num_seconds();
    let minutes = seconds / 60;
    let hours = minutes / 60;
    let days = hours / 24;
    if minutes < 1 {
        "now".to_owned()
    } else if hours < 1 {
        format!("{minutes}m")
    } else if days < 1 {
        format!("{hours}h")
    } else if days < 7 {
        format!("{days}d")
    } else {
        format!("{}w", days / 7)
    }
}

/// A project's rows in render/navigation order. Active honors server-arranged
/// keys; keyless rows use creation/re-entry anchors (legacy activity fallback).
pub fn grouped<'a>(
    threads: &'a [Thread],
    project_id: &str,
    view: View,
    now: DateTime<Utc>,
) -> [(&'static str, &'static str, Vec<&'a Thread>); 2] {
    let mut rows: Vec<_> = threads
        .iter()
        .filter(|thread| {
            (project_id.is_empty() || thread.project_id == project_id) && belongs(thread, view, now)
        })
        .collect();
    let anchor = |thread: &Thread, pinned: bool| {
        (if pinned {
            thread.created_at.as_deref()
        } else {
            thread
                .unsettled_at
                .as_deref()
                .or(thread.created_at.as_deref())
        })
        .and_then(|at| DateTime::parse_from_rfc3339(at).ok())
        .map(|at| at.with_timezone(&Utc))
        .or_else(|| last_activity(thread))
    };
    rows.sort_by(|left, right| {
        if left.pinned_at.is_some() != right.pinned_at.is_some() {
            return right.pinned_at.is_some().cmp(&left.pinned_at.is_some());
        }
        if view != View::Active {
            return last_activity(right)
                .cmp(&last_activity(left))
                .then_with(|| left.id.cmp(&right.id));
        }
        let pinned = left.pinned_at.is_some() && right.pinned_at.is_some();
        let (left_key, right_key) = if pinned {
            (&left.pin_order_key, &right.pin_order_key)
        } else {
            (&left.active_order_key, &right.active_order_key)
        };
        match (left_key, right_key) {
            (Some(a), Some(b)) => a.cmp(b).then_with(|| left.id.cmp(&right.id)),
            (Some(_), None) => {
                if pinned {
                    std::cmp::Ordering::Less
                } else {
                    std::cmp::Ordering::Greater
                }
            }
            (None, Some(_)) => {
                if pinned {
                    std::cmp::Ordering::Greater
                } else {
                    std::cmp::Ordering::Less
                }
            }
            _ => anchor(right, pinned)
                .cmp(&anchor(left, pinned))
                .then_with(|| left.id.cmp(&right.id)),
        }
    });
    let (pinned, active) = rows
        .into_iter()
        .partition(|thread| thread.pinned_at.is_some());
    [
        ("pinned", "Pinned", pinned),
        ("active", group_label(view), active),
    ]
}

/// The unpinned group's label in each view; only the Active view's rows are active.
pub fn group_label(view: View) -> &'static str {
    match view {
        View::Active => "Active",
        View::Settled => "Settled",
        View::Snoozed => "Snoozed",
        View::Archived => "Archived",
    }
}

/// Collapse-state key for one project's thread group.
pub fn group_key(project_id: &str, group: &str) -> String {
    format!("{project_id}:{group}")
}

/// The thread before or after `current` in `order`, wrapping at the ends.
pub fn neighbor(order: &[String], current: Option<&str>, forward: bool) -> Option<String> {
    if order.is_empty() {
        return None;
    }
    let index = match current.and_then(|id| order.iter().position(|entry| entry == id)) {
        Some(index) if forward => (index + 1) % order.len(),
        Some(index) => (index + order.len() - 1) % order.len(),
        None if forward => 0,
        None => order.len() - 1,
    };
    Some(order[index].clone())
}

/// Whether a thread menu action applies to the thread's current state, so the
/// menu offers one direction of each reversible action instead of both.
pub fn action_applies(thread: &Thread, action: &str, now: DateTime<Utc>) -> bool {
    let archived = thread.archived_at.is_some();
    let settled = thread.settled_override.as_deref() == Some("settled");
    let pinned = thread.pinned_at.is_some();
    match action {
        "archive" => !archived,
        "unarchive" => archived,
        "settle" => !settled,
        "unsettle" => settled,
        "snooze" => !snoozed(thread, now),
        "unsnooze" => snoozed(thread, now),
        "pin" => !pinned,
        "unpin" => pinned,
        _ => true,
    }
}

/// The sidebar name for a project: its workspace directory name, falling back
/// to the title when the path has no usable segment.
pub fn project_name<'a>(title: &'a str, workspace_root: &'a str) -> &'a str {
    workspace_root
        .trim_end_matches('/')
        .rsplit('/')
        .find(|segment| !segment.is_empty())
        .unwrap_or(title)
}

/// Secondary metadata for a thread row: its branch, else its selected model.
pub fn thread_meta(thread: &Thread) -> Option<String> {
    if let Some(branch) = thread.branch.as_deref().filter(|branch| !branch.is_empty()) {
        return Some(branch.to_owned());
    }
    let instance = thread.model_selection["instanceId"].as_str()?;
    let model = thread.model_selection["model"].as_str().unwrap_or("");
    Some(if model.is_empty() {
        instance.to_owned()
    } else {
        format!("{instance} · {model}")
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn r13_b_materialization_reserves_hidden_keys_and_repairs_corruption() {
        let mut a = thread();
        a.id = "a".into();
        a.active_order_key = None;
        let mut b = thread();
        b.id = "b".into();
        b.active_order_key = Some("bad-key".into());
        let mut hidden = thread();
        hidden.id = "hidden".into();
        hidden.active_order_key = Some("ir".into());
        let all = vec![a.clone(), b.clone(), hidden];
        let assignments = order_assignments(&[&b, &a], &all, "b", false);
        assert_eq!(assignments.len(), 2);
        assert_eq!(assignments[0].0, "b");
        assert!(assignments[0].1 < assignments[1].1);
        assert!(
            assignments
                .iter()
                .all(|(_, key)| key != "ir" && key_between(Some(key), None).is_some())
        );
        assert_eq!(all[2].active_order_key.as_deref(), Some("ir"));
    }

    #[test]
    fn r13_b_dismissed_wake_survives_restart_without_acknowledging_history() {
        let now = Utc::now();
        let path = std::env::temp_dir().join(format!("r13-b-wake-{}.json", uuid::Uuid::new_v4()));
        let mut row = thread();
        row.snoozed_at = Some((now - chrono::Duration::hours(2)).to_rfc3339());
        row.snoozed_until = Some((now - chrono::Duration::hours(1)).to_rfc3339());
        let mut state = State::default();
        state.load_local(path.clone()).unwrap();
        state.marked_unread.insert(("env".into(), row.id.clone()));
        state
            .woke_acknowledged
            .insert(("env".into(), row.id.clone()), woke_at(&row, now).unwrap());
        state.save_local().unwrap();
        let mut restored = State::default();
        restored.load_local(path.clone()).unwrap();
        assert!(!restored.woke("env", &row, now));
        assert!(restored.unread("env", &row));
        assert!(restored.woke("other", &row, now));
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn sidebar_fixb_prepares_only_changed_rows() {
        let state = State::default();
        let now = Utc::now();
        let mut first = thread();
        first.id = "a".into();
        let mut second = first.clone();
        second.id = "b".into();
        state.retain_rows(&[first.clone(), second.clone()], "env");
        let a = state.prepared_row(&first, false, now);
        state.prepared_row(&second, false, now);
        for _ in 0..100 {
            assert!(std::rc::Rc::ptr_eq(
                &a,
                &state.prepared_row(&first, false, now)
            ));
            state.prepared_row(&second, false, now);
        }
        assert_eq!(state.row_preparation_count(), 2);
        second.title = "Changed by stream".into();
        state.prepared_row(&second, false, now);
        assert_eq!(state.row_preparation_count(), 3);
        assert!(std::rc::Rc::ptr_eq(
            &a,
            &state.prepared_row(&first, false, now)
        ));
        state.prepared_row(&first, true, now);
        assert_eq!(state.row_preparation_count(), 4);
    }

    #[test]
    fn sidebar_fixb_local_restart_keeps_environment_read_markers_and_view() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("sidebar.json");
        let mut state = State::default();
        state.load_local(path.clone()).unwrap();
        state.priority = true;
        let mut row = thread();
        row.latest_turn = Some(
            serde_json::from_value(serde_json::json!({
                "turnId":"turn", "state":"completed", "completedAt":"2026-04-01T00:00:00Z"
            }))
            .unwrap(),
        );
        // Never-visited history is not automatically unread.
        assert!(!state.unread("env", &row));
        state.visit("env", &row);
        state
            .marked_unread
            .insert(("other-env".into(), row.id.clone()));
        state.save_local().unwrap();
        let mut restarted = State::default();
        restarted.load_local(path).unwrap();
        assert!(restarted.priority);
        assert!(!restarted.unread("env", &row));
        assert!(restarted.unread("other-env", &row));
        restarted.visit("other-env", &row);
        assert!(!restarted.unread("other-env", &row));
    }

    #[test]
    fn sidebar_fixb_raised_hand_and_timer_wake_keep_original_early_timestamp() {
        let now = DateTime::parse_from_rfc3339("2026-04-01T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let mut row = thread();
        row.snoozed_at = Some("2026-04-01T09:00:00Z".into());
        row.snoozed_until = Some("2026-04-01T13:00:00Z".into());
        assert!(snoozed(&row, now));
        row.has_pending_approvals = true;
        assert!(
            !snoozed(&row, now),
            "approval must never be hidden on snoozed shelf"
        );
        row.has_pending_approvals = false;
        row.latest_turn = Some(
            serde_json::from_value(serde_json::json!({
                "turnId":"turn", "state":"completed", "completedAt":"2026-04-01T11:00:00Z"
            }))
            .unwrap(),
        );
        let state = State::default();
        assert!(state.woke("env", &row, now));
        let early = woke_at(&row, now).unwrap();
        assert_eq!(woke_at(&row, now + chrono::Duration::hours(2)), Some(early));
        row.latest_turn = None;
        assert_eq!(woke_at(&row, now), None);
        assert_eq!(
            woke_at(&row, now + chrono::Duration::hours(2)),
            timestamp(row.snoozed_until.as_deref())
        );
    }

    #[test]
    fn sidebar_fixb_reconcile_drops_hidden_anchor_and_environment_selection() {
        let mut state = State {
            selection_environment: "env".into(),
            selection: vec!["hidden".into(), "visible".into()],
            selection_anchor: Some("hidden".into()),
            ..Default::default()
        };
        state.reconcile_selection("env", &["visible".into()]);
        assert_eq!(state.selection, vec!["visible"]);
        assert!(state.selection_anchor.is_none());
        state.reconcile_selection("other", &["visible".into()]);
        assert!(state.selection.is_empty());
    }

    fn thread() -> Thread {
        serde_json::from_value(json!({"id":"t","projectId":"p","title":"Test"})).unwrap()
    }
    #[test]
    fn timed_snooze_expires_without_server_mutation() {
        let mut thread = thread();
        thread.snoozed_until = Some("2026-04-01T01:00:00Z".into());
        let before = DateTime::parse_from_rfc3339("2026-04-01T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let after = before + chrono::Duration::hours(1);
        assert!(belongs(&thread, View::Snoozed, before));
        assert!(!belongs(&thread, View::Active, before));
        assert!(belongs(&thread, View::Active, after));
        assert!(!belongs(&thread, View::Snoozed, after));
        assert_eq!(
            expiry(&[thread.clone()], before),
            Some(std::time::Duration::from_secs(3600))
        );
        assert_eq!(expiry(&[thread], after), None);
    }
    #[test]
    fn archive_and_explicit_settlement_have_reverse_views() {
        let now = Utc::now();
        let mut thread = thread();
        thread.settled_override = Some("settled".into());
        assert!(belongs(&thread, View::Settled, now));
        assert!(!belongs(&thread, View::Active, now));
        thread.settled_override = Some("active".into());
        assert!(belongs(&thread, View::Active, now));
        thread.archived_at = Some(now.to_rfc3339());
        assert!(belongs(&thread, View::Archived, now));
        assert!(!belongs(&thread, View::Active, now));
        thread.archived_at = None;
        assert!(belongs(&thread, View::Active, now));
    }

    fn at(value: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(value)
            .unwrap()
            .with_timezone(&Utc)
    }

    #[test]
    fn rows_expose_only_live_status_markers() {
        let now = Utc::now();
        let mut thread = thread();
        assert_eq!(primary_status(&thread, false), None);
        // Pinned, snoozed, and settled are implied by the group or view, not the row.
        thread.pinned_at = Some(now.to_rfc3339());
        thread.snoozed_until = Some((now + chrono::Duration::hours(1)).to_rfc3339());
        thread.settled_override = Some("settled".into());
        thread.linked_pull_request = Some(json!({"number": 7}));
        assert_eq!(primary_status(&thread, false), None);
    }

    #[test]
    fn rows_expose_live_shell_status() {
        let mut thread = thread();
        thread.has_actionable_proposed_plan = true;
        assert_eq!(primary_status(&thread, false), Some(Status::Plan));
        thread.session = Some(crate::shell::Session {
            status: "running".into(),
            last_error: None,
        });
        assert_eq!(primary_status(&thread, false), Some(Status::Working));
        thread.has_pending_user_input = true;
        assert_eq!(primary_status(&thread, false), Some(Status::Input));
        thread.has_pending_approvals = true;
        assert_eq!(primary_status(&thread, false), Some(Status::Approval));
        thread.has_pending_approvals = false;
        thread.has_pending_user_input = false;
        thread.session.as_mut().unwrap().status = "idle".into();
        assert_eq!(primary_status(&thread, false), Some(Status::Plan));
        assert_eq!(primary_status(&thread, true), Some(Status::Working));
    }

    #[test]
    fn relative_time_steps_at_each_boundary() {
        let now = at("2026-04-01T00:00:00Z");
        let ago = |seconds: i64| relative_time(now - chrono::Duration::seconds(seconds), now);
        assert_eq!(
            relative_time(now + chrono::Duration::minutes(5), now),
            "now"
        );
        assert_eq!(ago(0), "now");
        assert_eq!(ago(59), "now");
        assert_eq!(ago(60), "1m");
        assert_eq!(ago(59 * 60 + 59), "59m");
        assert_eq!(ago(3600), "1h");
        assert_eq!(ago(24 * 3600 - 1), "23h");
        assert_eq!(ago(24 * 3600), "1d");
        assert_eq!(ago(7 * 24 * 3600 - 1), "6d");
        assert_eq!(ago(7 * 24 * 3600), "1w");
        assert_eq!(ago(21 * 24 * 3600), "3w");
    }

    #[test]
    fn last_activity_prefers_user_message_and_falls_back() {
        let mut thread = thread();
        assert_eq!(last_activity(&thread), None);
        thread.created_at = Some("2026-04-01T00:00:00Z".into());
        assert_eq!(last_activity(&thread), Some(at("2026-04-01T00:00:00Z")));
        thread.updated_at = Some("2026-04-02T00:00:00Z".into());
        assert_eq!(last_activity(&thread), Some(at("2026-04-02T00:00:00Z")));
        thread.latest_user_message_at = Some("2026-04-01T12:00:00Z".into());
        assert_eq!(last_activity(&thread), Some(at("2026-04-01T12:00:00Z")));
    }

    #[test]
    fn groups_order_rows_most_recently_active_first() {
        let now = Utc::now();
        let row = |id: &str, activity: Option<&str>, pinned: bool| {
            let mut row = thread();
            row.id = id.into();
            row.latest_user_message_at = activity.map(str::to_owned);
            row.pinned_at = pinned.then(|| now.to_rfc3339());
            row
        };
        let threads = vec![
            row("old", Some("2026-04-01T00:00:00Z"), false),
            row("unknown", None, false),
            row("new", Some("2026-04-03T00:00:00Z"), false),
            row("pin-old", Some("2026-03-01T00:00:00Z"), true),
            row("pin-new", Some("2026-03-02T00:00:00Z"), true),
        ];
        let ids: Vec<_> = grouped(&threads, "p", View::Active, now)
            .into_iter()
            .map(|(key, _, rows)| (key, rows.iter().map(|t| t.id.as_str()).collect::<Vec<_>>()))
            .collect();
        assert_eq!(
            ids,
            vec![
                ("pinned", vec!["pin-new", "pin-old"]),
                ("active", vec!["new", "old", "unknown"]),
            ]
        );
    }

    #[test]
    fn sidebar_parity_policy_drafts_capabilities_and_busy_state() {
        let now = Utc::now();
        let caps = json!({"threadPinning":true,"threadSettlement":true,"threadSnooze":true});
        for action in ["pin", "unpin", "archive", "settle", "snooze", "delete"] {
            assert!(!thread_action_policy(None, action, now, &caps));
        }
        assert!(thread_action_policy(None, "rename", now, &caps));
        assert!(thread_action_policy(None, "discard", now, &caps));
        let mut row = thread();
        assert!(!thread_action_policy(Some(&row), "pin", now, &json!({})));
        row.session = Some(crate::shell::Session {
            status: "running".into(),
            last_error: None,
        });
        assert!(!thread_action_policy(Some(&row), "archive", now, &caps));
        assert!(thread_action_policy(Some(&row), "snooze", now, &caps));
        row.has_pending_approvals = true;
        assert!(!thread_action_policy(Some(&row), "snooze", now, &caps));
    }

    #[test]
    fn sidebar_parity_fractional_keys_stay_between_neighbors() {
        for (before, after) in [
            (None, None),
            (Some("b"), Some("c")),
            (None, Some("ab")),
            (Some("z"), None),
            (Some("bbb"), Some("bbc")),
        ] {
            let key = key_between(before, after).unwrap();
            assert!(before.is_none_or(|before| before < key.as_str()));
            assert!(after.is_none_or(|after| key.as_str() < after));
            assert!(!key.ends_with('a'));
        }
        assert!(key_between(Some("a"), None).is_none());
        assert!(key_between(Some("z"), Some("b")).is_none());
    }

    #[test]
    fn sidebar_parity_anchor_resolves_source_and_instant_not_tail() {
        let anchor = MessageAnchor {
            environment: "e".into(),
            thread_id: "t".into(),
            source: "assistant".into(),
            message_created_at: "2026-04-01T00:00:00Z".into(),
        };
        let thread = json!({"id":"t","messages":[
            {"id":"u","role":"user","createdAt":"2026-04-01T00:00:00Z"},
            {"id":"a","role":"assistant","createdAt":"2026-04-01T01:00:00+01:00"},
            {"id":"tail","role":"assistant","createdAt":"2026-04-02T00:00:00Z"}
        ]});
        assert_eq!(
            resolve_search_anchor(&anchor, &thread).as_deref(),
            Some("a")
        );
        assert!(resolve_search_anchor(&anchor, &json!({"id":"other"})).is_none());
    }

    #[test]
    fn sidebar_parity_unicode_highlights_are_original_byte_ranges() {
        let text = "Écho éCHO 日本";
        assert_eq!(match_ranges(text, "écho"), vec![0..5, 6..11]);
        assert_eq!(match_ranges(text, "日本"), vec![12..18]);
        assert!(match_ranges(text, "").is_empty());
    }

    #[test]
    fn sidebar_parity_saved_order_overrides_activity_and_cache_invalidates() {
        let now = Utc::now();
        let mut first = thread();
        first.id = "first".into();
        first.active_order_key = Some("z".into());
        first.latest_user_message_at = Some(now.to_rfc3339());
        let mut second = thread();
        second.id = "second".into();
        second.active_order_key = Some("b".into());
        let mut rows = vec![first, second];
        let state = State::default();
        assert_eq!(
            state.grouped(&rows, Some(1), "p", View::Active, now, "test")[1].2[0].id,
            "second"
        );
        rows[0].active_order_key = Some("a".into());
        assert_eq!(
            state.grouped(&rows, Some(2), "p", View::Active, now, "test")[1].2[0].id,
            "first"
        );
    }

    #[test]
    fn navigation_wraps_and_starts_from_either_end() {
        let order = vec!["a".to_owned(), "b".to_owned(), "c".to_owned()];
        assert_eq!(neighbor(&order, Some("a"), true).as_deref(), Some("b"));
        assert_eq!(neighbor(&order, Some("c"), true).as_deref(), Some("a"));
        assert_eq!(neighbor(&order, Some("a"), false).as_deref(), Some("c"));
        assert_eq!(neighbor(&order, None, true).as_deref(), Some("a"));
        assert_eq!(neighbor(&order, Some("gone"), false).as_deref(), Some("c"));
        assert_eq!(neighbor(&[], Some("a"), true), None);
    }

    #[test]
    fn groups_list_pinned_before_active_for_one_project() {
        let now = Utc::now();
        let mut pinned = thread();
        pinned.id = "pinned".into();
        pinned.pinned_at = Some(now.to_rfc3339());
        let mut other = thread();
        other.id = "other".into();
        other.project_id = "q".into();
        let threads = vec![thread(), pinned, other];
        let ids: Vec<_> = grouped(&threads, "p", View::Active, now)
            .into_iter()
            .map(|(key, _, rows)| (key, rows.iter().map(|t| t.id.as_str()).collect::<Vec<_>>()))
            .collect();
        assert_eq!(ids, vec![("pinned", vec!["pinned"]), ("active", vec!["t"])]);
    }

    #[test]
    fn group_label_names_the_view_not_the_active_state() {
        let now = Utc::now();
        let mut settled = thread();
        settled.settled_override = Some("settled".into());
        let threads = vec![settled];
        assert_eq!(grouped(&threads, "p", View::Settled, now)[1].1, "Settled");
        assert_eq!(grouped(&threads, "p", View::Active, now)[1].1, "Active");
        assert_eq!(group_label(View::Archived), "Archived");
    }

    #[test]
    fn menu_offers_only_the_applicable_direction() {
        let now = Utc::now();
        let mut thread = thread();
        for (forward, reverse) in [
            ("pin", "unpin"),
            ("settle", "unsettle"),
            ("snooze", "unsnooze"),
            ("archive", "unarchive"),
        ] {
            assert!(action_applies(&thread, forward, now), "{forward}");
            assert!(!action_applies(&thread, reverse, now), "{reverse}");
        }
        thread.pinned_at = Some(now.to_rfc3339());
        thread.settled_override = Some("settled".into());
        thread.snoozed_until = Some((now + chrono::Duration::hours(1)).to_rfc3339());
        thread.archived_at = Some(now.to_rfc3339());
        for (forward, reverse) in [
            ("pin", "unpin"),
            ("settle", "unsettle"),
            ("snooze", "unsnooze"),
            ("archive", "unarchive"),
        ] {
            assert!(!action_applies(&thread, forward, now), "{forward}");
            assert!(action_applies(&thread, reverse, now), "{reverse}");
        }
        assert!(action_applies(&thread, "delete", now));
        assert!(action_applies(&thread, "rename", now));
    }

    #[test]
    fn project_name_falls_back_to_the_title() {
        assert_eq!(project_name("Fixture", "/tmp/fixture/"), "fixture");
        assert_eq!(project_name("My Project", "/"), "My Project");
        assert_eq!(project_name("My Project", ""), "My Project");
    }

    #[test]
    fn thread_meta_prefers_branch_and_falls_back_to_model() {
        let mut thread = thread();
        thread.model_selection = json!({"instanceId": "codex", "model": "gpt-5.4"});
        assert_eq!(thread_meta(&thread).as_deref(), Some("codex · gpt-5.4"));
        thread.branch = Some("t3code/feature".into());
        assert_eq!(thread_meta(&thread).as_deref(), Some("t3code/feature"));
        thread.branch = None;
        thread.model_selection = json!({});
        assert_eq!(thread_meta(&thread), None);
    }
}
