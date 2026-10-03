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
}
