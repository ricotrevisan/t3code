//! Mirrors shared/usageLimits duration and web timestampFormat relative labels.
//! Relative intervals are independent of the wall-clock 12/24-hour preference.
use chrono::{DateTime, Utc};

fn millis(instant: &str) -> Option<i64> {
    DateTime::parse_from_rfc3339(instant)
        .ok()
        .map(|date| date.timestamp_millis())
}

pub fn now() -> i64 {
    Utc::now().timestamp_millis()
}

pub fn duration(ms: i64) -> String {
    let minutes = ms.max(0) / 60_000;
    let hours = minutes / 60;
    let days = hours / 24;
    if days > 0 {
        format!("{days}d {}h", hours % 24)
    } else if hours > 0 {
        format!("{hours}h {}m", minutes % 60)
    } else {
        format!("{minutes}m")
    }
}

pub fn checked(instant: &str, now: i64) -> String {
    let Some(at) = millis(instant) else {
        return "Last checked unknown".into();
    };
    let minutes = now.saturating_sub(at).max(0) / 60_000;
    let age = if minutes < 1 {
        "just now".into()
    } else if minutes < 60 {
        format!("{minutes}m ago")
    } else if minutes < 1440 {
        format!("{}h ago", minutes / 60)
    } else {
        format!("{}d ago", minutes / 1440)
    };
    format!("Last checked {age}")
}

pub fn resets(instant: Option<&str>, now: i64) -> String {
    let Some(at) = instant.and_then(millis) else {
        return String::new();
    };
    if at <= now {
        " · resets now".into()
    } else {
        format!(" · resets in {}", duration(at.saturating_sub(now)))
    }
}

pub fn credits(count: u64, expiry: Option<&str>, now: i64) -> String {
    if count == 0 {
        return "No reset credits banked".into();
    }
    let expiry = expiry
        .and_then(millis)
        .map(|at| format!(" · next expires in {}", duration(at.saturating_sub(now))))
        .unwrap_or_default();
    format!(
        "{count} {} banked{expiry}",
        if count == 1 {
            "reset credit"
        } else {
            "reset credits"
        }
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    fn instant() -> i64 {
        millis("2026-10-07T20:16:06.587Z").unwrap()
    }
    #[test]
    fn r17_pol_rw28_checked_relative_boundaries_and_invalid_instants() {
        let at = "2026-10-07T20:16:06.587Z";
        for (offset, label) in [
            (-1, "just now"),
            (59_999, "just now"),
            (180_000, "3m ago"),
            (3_600_000, "1h ago"),
            (86_400_000, "1d ago"),
        ] {
            assert_eq!(
                checked(at, instant() + offset),
                format!("Last checked {label}")
            );
        }
        assert_eq!(checked("Unknown", instant()), "Last checked unknown");
    }
    #[test]
    fn r17_pol_rw28_reset_duration_matches_shared_reference() {
        let at = "2026-10-07T20:16:06.587Z";
        assert_eq!(resets(Some(at), instant()), " · resets now");
        assert_eq!(resets(Some(at), instant() + 1), " · resets now");
        assert_eq!(
            resets(Some(at), instant() - 133 * 60_000),
            " · resets in 2h 13m"
        );
        assert_eq!(
            resets(Some(at), instant() - 5 * 86_400_000),
            " · resets in 5d 0h"
        );
        assert_eq!(resets(None, instant()), "");
        assert_eq!(resets(Some("bad"), instant()), "");
        assert_eq!(duration(-1), "0m");
        assert_eq!(duration(3 * 86_400_000 + 4 * 3_600_000), "3d 4h");
    }
    #[test]
    fn r17_pol_rw28_credit_expiry_pluralization_and_missing_time() {
        let at = "2026-10-07T20:16:06.587Z";
        assert_eq!(credits(0, Some(at), instant()), "No reset credits banked");
        assert_eq!(credits(1, None, instant()), "1 reset credit banked");
        assert_eq!(
            credits(2, Some(at), instant() - 133 * 60_000),
            "2 reset credits banked · next expires in 2h 13m"
        );
        assert_eq!(
            credits(1, Some(at), instant() + 1),
            "1 reset credit banked · next expires in 0m"
        );
        assert_eq!(credits(1, Some("bad"), instant()), "1 reset credit banked");
    }
}
