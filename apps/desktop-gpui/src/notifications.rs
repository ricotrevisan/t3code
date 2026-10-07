//! Transient in-window toasts and the desktop notifications thread events raise.
use std::time::Duration;

use crate::shell::Thread;

/// How long an info or success toast stays before its scheduled dismissal.
/// Error toasts stay until the user closes them.
pub const TOAST_LIFETIME: Duration = Duration::from_secs(5);
/// The stack keeps the newest toasts; older ones drop off the top.
pub const MAX_TOASTS: usize = 4;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToastKind {
    Info,
    Success,
    Error,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Toast {
    pub id: u64,
    pub kind: ToastKind,
    pub message: String,
    /// The thread an "Open thread" action selects.
    pub thread: Option<String>,
}

/// Whether a status-bar outcome reports a failure.
pub fn is_error(status: &str) -> bool {
    let status = status.to_lowercase();
    ["failed", "error", "rejected", "could not", "cannot"]
        .iter()
        .any(|word| status.contains(word))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    ApprovalRequested,
    UserInputRequested,
    TurnCompleted,
}

impl Event {
    pub fn title(self) -> &'static str {
        match self {
            Self::ApprovalRequested => "Approval needed",
            Self::UserInputRequested => "Input needed",
            Self::TurnCompleted => "Thread completed",
        }
    }
}

/// The event a shell upsert of `next` raises over its previous summary `prior`.
/// Archived threads raise none.
pub fn thread_event(prior: &Thread, next: &Thread) -> Option<Event> {
    if next.archived_at.is_some() {
        return None;
    }
    if next.has_pending_approvals && !prior.has_pending_approvals {
        return Some(Event::ApprovalRequested);
    }
    if next.has_pending_user_input && !prior.has_pending_user_input {
        return Some(Event::UserInputRequested);
    }
    let completed = |thread: &Thread| {
        thread
            .latest_turn
            .as_ref()
            .filter(|turn| turn.state == "completed")
            .map(|turn| (turn.turn_id.clone(), turn.completed_at.clone()))
    };
    match completed(next) {
        Some(turn) if completed(prior).as_ref() != Some(&turn) => Some(Event::TurnCompleted),
        _ => None,
    }
}

/// The desktop notification a shell upsert should send: only for a thread the
/// user is not looking at, either because another thread is selected or the
/// window is in the background.
pub fn notification(
    prior: &Thread,
    next: &Thread,
    thread_selected: bool,
    window_active: bool,
) -> Option<Event> {
    thread_event(prior, next).filter(|_| !thread_selected || !window_active)
}

/// The notification tag for a thread; a click on the notification carries it back.
pub fn tag(thread_id: &str) -> String {
    format!("thread:{thread_id}")
}

pub fn thread_from_tag(tag: &str) -> Option<&str> {
    tag.strip_prefix("thread:").filter(|id| !id.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn thread(extra: serde_json::Value) -> Thread {
        let mut value = json!({"id": "t", "projectId": "p", "title": "Fix the build"});
        value
            .as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        serde_json::from_value(value).unwrap()
    }

    fn completed(turn: &str, at: &str) -> serde_json::Value {
        json!({"latestTurn": {"turnId": turn, "state": "completed", "completedAt": at}})
    }

    #[test]
    fn newly_pending_requests_and_completions_raise_events() {
        let idle = thread(json!({}));
        assert_eq!(
            thread_event(&idle, &thread(json!({"hasPendingApprovals": true}))),
            Some(Event::ApprovalRequested)
        );
        assert_eq!(
            thread_event(&idle, &thread(json!({"hasPendingUserInput": true}))),
            Some(Event::UserInputRequested)
        );
        let running = thread(json!({"latestTurn": {"turnId": "a", "state": "running"}}));
        assert_eq!(
            thread_event(&running, &thread(completed("a", "2026-01-01T00:00:00Z"))),
            Some(Event::TurnCompleted)
        );
    }

    #[test]
    fn unchanged_or_archived_threads_raise_nothing() {
        let waiting = thread(json!({"hasPendingApprovals": true}));
        assert_eq!(thread_event(&waiting, &waiting), None);
        let done = thread(completed("a", "2026-01-01T00:00:00Z"));
        assert_eq!(thread_event(&done, &done), None);
        let failed = thread(json!({"latestTurn": {"turnId": "a", "state": "error"}}));
        assert_eq!(thread_event(&thread(json!({})), &failed), None);
        let archived =
            thread(json!({"hasPendingApprovals": true, "archivedAt": "2026-01-01T00:00:00Z"}));
        assert_eq!(thread_event(&thread(json!({})), &archived), None);
    }

    #[test]
    fn notifies_unless_the_thread_is_in_view() {
        let prior = thread(json!({}));
        let next = thread(json!({"hasPendingApprovals": true}));
        assert_eq!(
            notification(&prior, &next, false, true),
            Some(Event::ApprovalRequested)
        );
        assert_eq!(
            notification(&prior, &next, true, false),
            Some(Event::ApprovalRequested)
        );
        assert_eq!(notification(&prior, &next, true, true), None);
        assert_eq!(notification(&prior, &prior, false, false), None);
    }

    #[test]
    fn tags_round_trip_to_the_thread() {
        assert_eq!(thread_from_tag(&tag("t-1")), Some("t-1"));
        assert_eq!(thread_from_tag("thread:"), None);
        assert_eq!(thread_from_tag("other"), None);
    }

    #[test]
    fn failure_wording_marks_errors() {
        assert!(is_error("Could not read a.png: denied"));
        assert!(is_error("Projection failed — reconnect required"));
        assert!(!is_error("Worktree created"));
    }
}
