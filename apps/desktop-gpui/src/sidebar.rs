//! Sidebar membership derives timed snoozes locally; expiry needs no server event.
use crate::shell::Thread;
use chrono::{DateTime, Utc};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum View {
    Active,
    Settled,
    Snoozed,
    Archived,
}

pub fn snoozed(thread: &Thread, now: DateTime<Utc>) -> bool {
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
}

impl Status {
    pub fn label(self) -> &'static str {
        match self {
            Self::Approval => "Approval",
            Self::Input => "Input",
            Self::Working => "Working",
            Self::Plan => "Plan",
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

/// A project's rows for a view, split into the groups the sidebar renders, in
/// render order, most recently active first. Thread navigation walks the same
/// result.
pub fn grouped<'a>(
    threads: &'a [Thread],
    project_id: &str,
    view: View,
    now: DateTime<Utc>,
) -> [(&'static str, &'static str, Vec<&'a Thread>); 2] {
    let mut rows: Vec<_> = threads
        .iter()
        .filter(|thread| thread.project_id == project_id && belongs(thread, view, now))
        .collect();
    rows.sort_by_key(|thread| std::cmp::Reverse(last_activity(thread)));
    let (pinned, active) = rows
        .into_iter()
        .partition(|thread| thread.pinned_at.is_some());
    [("pinned", "Pinned", pinned), ("active", "Active", active)]
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
