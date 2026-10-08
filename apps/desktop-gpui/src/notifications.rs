//! Transient in-window toasts and the desktop notifications thread events raise.
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, time::Duration};

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

/// The status bar's right-hand text: the last status message, prefixed with
/// "Offline" when there is no connection. The status dot carries "connected".
pub fn status_bar_text(connected: bool, status: &str) -> String {
    match (connected, status.is_empty()) {
        (true, _) => status.to_owned(),
        (false, true) => "Offline".to_owned(),
        (false, false) => format!("Offline · {status}"),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    ApprovalRequested,
    UserInputRequested,
    TurnCompleted,
    ThreadFailed,
}

impl Event {
    pub fn title(self) -> &'static str {
        match self {
            Self::ApprovalRequested => "Approval needed",
            Self::UserInputRequested => "Input needed",
            Self::TurnCompleted => "Thread completed",
            Self::ThreadFailed => "Thread failed",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum NotificationMode {
    #[default]
    Off,
    Notifications,
    Sound,
    NotificationsAndSound,
}

impl NotificationMode {
    pub const ALL: [Self; 4] = [
        Self::Off,
        Self::Notifications,
        Self::Sound,
        Self::NotificationsAndSound,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::Off => "Off",
            Self::Notifications => "Notifications only",
            Self::Sound => "Sound only",
            Self::NotificationsAndSound => "Notifications with sound",
        }
    }
    pub fn desktop(self) -> bool {
        matches!(self, Self::Notifications | Self::NotificationsAndSound)
    }
    pub fn sound(self) -> bool {
        matches!(self, Self::Sound | Self::NotificationsAndSound)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Preferences {
    pub mode: NotificationMode,
    pub in_app: bool,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Decision {
    pub toast: bool,
    pub desktop: bool,
    pub sound: bool,
}

pub fn decision(prefs: Preferences, selected: bool, focused: bool) -> Decision {
    Decision {
        toast: prefs.in_app && focused && !selected,
        desktop: prefs.mode.desktop() && !focused,
        sound: prefs.mode.sound(),
    }
}

#[derive(Default, Clone)]
struct Summary {
    attention: Option<(String, Event)>,
    completion: Option<i64>,
}

impl Summary {
    fn from_thread(thread: &Thread, prior: Option<&Self>) -> Self {
        let turn = thread.latest_turn.as_ref();
        let session = thread
            .session
            .as_ref()
            .map(|session| session.status.as_str());
        let working = matches!(session, Some("running" | "starting"));
        let background = matches!(
            thread.background_liveness.as_deref(),
            Some("working" | "monitoring")
        );
        let attention = if thread.has_pending_approvals {
            Some(Event::ApprovalRequested)
        } else if thread.has_pending_user_input {
            Some(Event::UserInputRequested)
        } else if !working
            && (session == Some("error")
                || (!background && turn.is_some_and(|turn| turn.state == "error")))
        {
            Some(Event::ThreadFailed)
        } else {
            None
        };
        let completed = turn
            .filter(|turn| turn.state == "completed")
            .and_then(|turn| turn.completed_at.as_deref())
            .and_then(|at| chrono::DateTime::parse_from_rfc3339(at).ok())
            .map(|at| at.timestamp_millis());
        Self {
            attention: attention.map(|event| {
                (
                    turn.map(|turn| turn.turn_id.clone()).unwrap_or_default(),
                    event,
                )
            }),
            // Keep the high water mark even across running/attention/invalid upserts.
            completion: prior
                .and_then(|prior| prior.completion)
                .into_iter()
                .chain(completed.filter(|_| attention.is_none() && !working && !background))
                .max(),
        }
    }
}

/// Snapshot baselines never emit; only subsequent live upserts can raise events.
#[derive(Default)]
pub struct Tracker {
    previous: HashMap<String, Summary>,
}
impl Tracker {
    pub fn baseline<'a>(&mut self, threads: impl Iterator<Item = &'a Thread>) {
        self.previous = threads
            .map(|thread| (thread.id.clone(), Summary::from_thread(thread, None)))
            .collect();
    }
    pub fn clear(&mut self) {
        self.previous.clear();
    }
    pub fn retain<'a>(&mut self, threads: impl Iterator<Item = &'a Thread>) {
        let ids: std::collections::HashSet<_> = threads.map(|thread| thread.id.as_str()).collect();
        self.previous.retain(|id, _| ids.contains(id.as_str()));
    }
    pub fn observe(&mut self, thread: &Thread) -> Option<Event> {
        let prior = self.previous.get(&thread.id);
        let next = Summary::from_thread(thread, prior);
        let event = prior.and_then(|prior| {
            if thread.archived_at.is_some() {
                return None;
            }
            if next.attention.is_some() && next.attention != prior.attention {
                next.attention.as_ref().map(|(_, event)| *event)
            } else if next.attention.is_none()
                && next.completion.is_some()
                && next.completion > prior.completion
            {
                Some(Event::TurnCompleted)
            } else {
                None
            }
        });
        self.previous.insert(thread.id.clone(), next);
        event
    }
}

/// JSON tuple encoding avoids ambiguous separators in origins or thread ids.
pub fn tag(origin: &str, thread_id: &str) -> String {
    serde_json::to_string(&(origin, thread_id)).expect("string tuple")
}

pub fn thread_from_tag(tag: &str, origin: &str) -> Option<String> {
    let (scope, id) = serde_json::from_str::<(String, String)>(tag)
        .ok()
        .or_else(|| {
            let (scope, id, _, _): (String, String, u64, u64) = serde_json::from_str(tag).ok()?;
            Some((scope, id))
        })?;
    (scope == origin && !id.is_empty()).then_some(id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn status_bar_shows_the_message_and_only_names_the_offline_state() {
        let message = "Connected · provider catalog loaded (7 instances)";
        assert_eq!(status_bar_text(true, message), message);
        assert_eq!(status_bar_text(true, ""), "");
        assert_eq!(
            status_bar_text(false, "Pairing failed"),
            "Offline · Pairing failed"
        );
        assert_eq!(status_bar_text(false, ""), "Offline");
    }

    fn thread_event(prior: &Thread, next: &Thread) -> Option<Event> {
        let mut tracker = Tracker::default();
        tracker.baseline(std::iter::once(prior));
        tracker.observe(next)
    }

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
        assert_eq!(
            thread_event(&thread(json!({})), &failed),
            Some(Event::ThreadFailed)
        );
        let archived =
            thread(json!({"hasPendingApprovals": true, "archivedAt": "2026-01-01T00:00:00Z"}));
        assert_eq!(thread_event(&thread(json!({})), &archived), None);
    }

    #[test]
    fn decision_matrix_separates_visual_and_audio_routes() {
        for mode in NotificationMode::ALL {
            for in_app in [false, true] {
                for selected in [false, true] {
                    for focused in [false, true] {
                        let result = decision(Preferences { mode, in_app }, selected, focused);
                        assert_eq!(result.toast, in_app && focused && !selected);
                        assert_eq!(result.desktop, mode.desktop() && !focused);
                        assert_eq!(result.sound, mode.sound());
                        assert!(!(result.toast && result.desktop));
                    }
                }
            }
        }
    }

    #[test]
    fn baseline_attention_turn_keys_and_completion_high_water_mark() {
        let mut tracker = Tracker::default();
        let waiting = thread(
            json!({"hasPendingApprovals":true,"latestTurn":{"turnId":"a","state":"completed","completedAt":"2026-01-02T00:00:00Z"}}),
        );
        assert_eq!(tracker.observe(&waiting), None);
        assert_eq!(tracker.observe(&waiting), None);
        let mut new_turn = waiting.clone();
        new_turn.latest_turn.as_mut().unwrap().turn_id = "b".into();
        assert_eq!(tracker.observe(&new_turn), Some(Event::ApprovalRequested));
        new_turn.has_pending_approvals = false;
        assert_eq!(tracker.observe(&new_turn), Some(Event::TurnCompleted));
        for at in ["invalid", "2026-01-01T00:00:00Z", "2026-01-02T00:00:00Z"] {
            assert_eq!(tracker.observe(&thread(completed("c", at))), None);
        }
        assert_eq!(
            tracker.observe(&thread(completed("d", "2026-01-03T00:00:00Z"))),
            Some(Event::TurnCompleted)
        );
        tracker.clear();
        tracker.baseline(std::iter::once(&new_turn));
        assert_eq!(tracker.observe(&new_turn), None);
    }

    #[test]
    fn working_suppresses_completion_and_status_attention_is_keyed() {
        let mut tracker = Tracker::default();
        let idle = thread(json!({}));
        tracker.baseline(std::iter::once(&idle));
        let mut done = thread(completed("a", "2026-01-02T00:00:00Z"));
        done.session = Some(crate::shell::Session {
            status: "running".into(),
            last_error: None,
        });
        assert_eq!(tracker.observe(&done), None);
        done.session = None;
        done.has_pending_user_input = true;
        assert_eq!(tracker.observe(&done), Some(Event::UserInputRequested));
        done.has_pending_approvals = true;
        assert_eq!(tracker.observe(&done), Some(Event::ApprovalRequested));
        assert_eq!(tracker.observe(&done), None);
        done.has_pending_approvals = false;
        done.has_pending_user_input = false;
        done.session = Some(crate::shell::Session {
            status: "error".into(),
            last_error: None,
        });
        assert_eq!(tracker.observe(&done), Some(Event::ThreadFailed));
        assert_eq!(tracker.observe(&done), None);
        done.session = None;
        assert_eq!(tracker.observe(&done), Some(Event::TurnCompleted));
        assert_eq!(
            tracker.observe(&thread(completed("b", "2026-01-01T19:00:00-05:00"))),
            None
        );
    }

    #[test]
    fn background_liveness_gates_ready_but_not_requests_or_session_error() {
        for liveness in ["working", "monitoring"] {
            let mut tracker = Tracker::default();
            tracker.baseline(std::iter::once(&thread(json!({}))));
            let mut done = thread(completed("a", "2026-01-02T00:00:00Z"));
            done.background_liveness = Some(liveness.into());
            assert_eq!(tracker.observe(&done), None);
            done.background_liveness = None;
            assert_eq!(tracker.observe(&done), Some(Event::TurnCompleted));
            assert_eq!(tracker.observe(&done), None);
            let mut failed = thread(
                json!({"latestTurn":{"turnId":"b","state":"error"},"backgroundLiveness":liveness}),
            );
            assert_eq!(tracker.observe(&failed), None);
            failed.session = Some(crate::shell::Session {
                status: "error".into(),
                last_error: None,
            });
            assert_eq!(tracker.observe(&failed), Some(Event::ThreadFailed));
            failed.has_pending_user_input = true;
            assert_eq!(tracker.observe(&failed), Some(Event::UserInputRequested));
            failed.has_pending_approvals = true;
            assert_eq!(tracker.observe(&failed), Some(Event::ApprovalRequested));
            failed.has_pending_approvals = false;
            failed.has_pending_user_input = false;
            failed.session = None;
            assert_eq!(tracker.observe(&failed), None);
            failed.background_liveness = None;
            assert_eq!(tracker.observe(&failed), Some(Event::ThreadFailed));
        }
    }

    #[test]
    fn tags_round_trip_to_the_thread() {
        assert_eq!(
            thread_from_tag(&tag("https://one", "t-1"), "https://one"),
            Some("t-1".into())
        );
        assert_eq!(
            thread_from_tag(&tag("https://one", "t-1"), "https://two"),
            None
        );
        assert_eq!(
            thread_from_tag(&tag("https://one", ""), "https://one"),
            None
        );
        assert_eq!(thread_from_tag("other", "https://one"), None);
        let receipt_tag = serde_json::to_string(&("https://one", "t-1", 2_u64, 3_u64)).unwrap();
        assert_eq!(
            thread_from_tag(&receipt_tag, "https://one"),
            Some("t-1".into())
        );
        assert_eq!(thread_from_tag(&receipt_tag, "https://two"), None);
    }

    #[test]
    fn failure_wording_marks_errors() {
        assert!(is_error("Could not read a.png: denied"));
        assert!(is_error("Projection failed — reconnect required"));
        assert!(!is_error("Worktree created"));
    }
}
