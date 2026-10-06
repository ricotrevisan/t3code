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

/// A short status marker a thread row shows beside its title.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Badge {
    Working,
    Approval,
    Input,
    Plan,
    PullRequest,
    Pinned,
    Snoozed,
    Settled,
}

impl Badge {
    pub fn label(self) -> &'static str {
        match self {
            Self::Working => "Working",
            Self::Approval => "Approval",
            Self::Input => "Input",
            Self::Plan => "Plan",
            Self::PullRequest => "PR",
            Self::Pinned => "Pinned",
            Self::Snoozed => "Snoozed",
            Self::Settled => "Settled",
        }
    }
}

/// Every status marker the row for `thread` should show, in display order.
pub fn badges(thread: &Thread, now: DateTime<Utc>) -> Vec<Badge> {
    let mut badges = Vec::new();
    if thread
        .session
        .as_ref()
        .is_some_and(|session| matches!(session.status.as_str(), "running" | "starting"))
    {
        badges.push(Badge::Working);
    }
    if thread.has_pending_approvals {
        badges.push(Badge::Approval);
    }
    if thread.has_pending_user_input {
        badges.push(Badge::Input);
    }
    if thread.has_actionable_proposed_plan {
        badges.push(Badge::Plan);
    }
    if thread.linked_pull_request.is_some() {
        badges.push(Badge::PullRequest);
    }
    if thread.pinned_at.is_some() {
        badges.push(Badge::Pinned);
    }
    if snoozed(thread, now) {
        badges.push(Badge::Snoozed);
    }
    if thread.settled_override.as_deref() == Some("settled") {
        badges.push(Badge::Settled);
    }
    badges
}

/// The sidebar name for a project: its workspace directory name, falling back
/// to the title when the path has no usable segment.
/// A project's rows for a view, split into the groups the sidebar renders, in
/// render order. Thread navigation walks the same result.
pub fn grouped<'a>(
    threads: &'a [Thread],
    project_id: &str,
    view: View,
    now: DateTime<Utc>,
) -> [(&'static str, &'static str, Vec<&'a Thread>); 2] {
    let (pinned, active) = threads
        .iter()
        .filter(|thread| thread.project_id == project_id && belongs(thread, view, now))
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

    #[test]
    fn rows_expose_only_live_status_markers() {
        let now = Utc::now();
        let mut thread = thread();
        assert!(badges(&thread, now).is_empty());
        thread.pinned_at = Some(now.to_rfc3339());
        thread.snoozed_until = Some((now + chrono::Duration::hours(1)).to_rfc3339());
        thread.settled_override = Some("settled".into());
        assert_eq!(
            badges(&thread, now),
            vec![Badge::Pinned, Badge::Snoozed, Badge::Settled]
        );
        // An expired snooze drops its marker without a server mutation.
        thread.snoozed_until = Some((now - chrono::Duration::hours(1)).to_rfc3339());
        assert_eq!(badges(&thread, now), vec![Badge::Pinned, Badge::Settled]);
    }

    #[test]
    fn rows_expose_live_shell_status() {
        let now = Utc::now();
        let mut thread = thread();
        thread.session = Some(crate::shell::Session {
            status: "running".into(),
            last_error: None,
        });
        thread.has_pending_approvals = true;
        thread.has_actionable_proposed_plan = true;
        thread.linked_pull_request = Some(serde_json::json!({"number": 7}));
        assert_eq!(
            badges(&thread, now),
            vec![
                Badge::Working,
                Badge::Approval,
                Badge::Plan,
                Badge::PullRequest
            ]
        );
        thread.session.as_mut().unwrap().status = "idle".into();
        assert_eq!(
            badges(&thread, now),
            vec![Badge::Approval, Badge::Plan, Badge::PullRequest]
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
