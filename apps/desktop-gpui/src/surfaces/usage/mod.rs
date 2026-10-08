//! Subscription quotas, not context-window token usage. Consume live server
//! config snapshots/providersUpdated and usageLimitSourcesUpdated projections.
use gpui_kit::component::{
    ActiveTheme as _, Sizable,
    button::{Button, ButtonVariants as _},
};
use gpui_kit::{AnyElement, App, div, prelude::*};
use serde_json::Value;

#[derive(Debug, PartialEq)]
pub struct Account {
    pub label: String,
    pub checked: String,
    pub windows: Vec<Window>,
    pub notice: Option<String>,
    pub credits: Option<u64>,
    pub credit_expiry: Option<String>,
    pub external: Option<(String, String)>,
}
#[derive(Debug, PartialEq)]
pub struct Window {
    pub label: String,
    pub used: f64,
    pub resets: Option<String>,
}
fn string(v: &Value) -> Option<String> {
    v.as_str().map(str::to_owned)
}
fn account(label: String, limits: &Value) -> Account {
    let windows: Vec<Window> = limits["windows"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|w| {
            let used = w["usedPercent"].as_f64()?;
            if !used.is_finite() || !(0.0..=100.0).contains(&used) {
                return None;
            }
            Some(Window {
                label: string(&w["label"])?,
                used,
                resets: string(&w["resetsAt"]),
            })
        })
        .collect();
    let notice = if limits["unavailable"].is_object() {
        string(&limits["unavailable"]["message"]).or_else(|| {
            Some(
                if limits["unavailable"]["reason"] == "probeFailed" {
                    "Could not refresh usage; last reported values may be stale."
                } else {
                    "This account does not report usage windows."
                }
                .into(),
            )
        })
    } else if windows.is_empty() {
        Some("No limits reported.".into())
    } else {
        None
    };
    let external = string(&limits["externalUsage"]["url"])
        .filter(|s| url::Url::parse(s).is_ok_and(|u| matches!(u.scheme(), "http" | "https")))
        .map(|url| {
            (
                string(&limits["externalUsage"]["label"]).unwrap_or_else(|| "Manage usage".into()),
                url,
            )
        });
    Account {
        label: if label.contains('@') {
            "Account (identity hidden)".into()
        } else {
            label
        },
        checked: string(&limits["checkedAt"]).unwrap_or_else(|| "Unknown".into()),
        windows,
        notice,
        credits: limits["resetCredits"]["availableCount"].as_u64(),
        credit_expiry: string(&limits["resetCredits"]["nextExpiresAt"]),
        external,
    }
}
/// Uses configured provider instances and external usage-source accounts.
/// Absent quota data is absent, never rendered as zero-percent usage.
pub fn accounts(config: &Value) -> Vec<Account> {
    let mut out = vec![];
    for p in config["providers"].as_array().into_iter().flatten() {
        if p["enabled"] == false
            || p["installed"] == false
            || p["availability"] == "unavailable"
            || !p["usageLimits"].is_object()
        {
            continue;
        }
        let label = string(&p["displayName"])
            .filter(|s| !s.trim().is_empty())
            .or_else(|| string(&p["instanceId"]))
            .unwrap_or_else(|| "Provider account".into());
        out.push(account(label, &p["usageLimits"]));
    }
    for source in config["usageLimitSources"].as_array().into_iter().flatten() {
        let label = source["label"].as_str().unwrap_or("Usage source");
        for a in source["accounts"].as_array().into_iter().flatten() {
            out.push(account(
                format!(
                    "{label} · {}",
                    a["plan"]
                        .as_str()
                        .unwrap_or(a["driver"].as_str().unwrap_or("Account"))
                ),
                &a["usageLimits"],
            ));
        }
        if let Some(error) = string(&source["error"]) {
            out.push(Account {
                label: label.into(),
                checked: source["checkedAt"].as_str().unwrap_or("Unknown").into(),
                windows: vec![],
                notice: Some(error),
                credits: None,
                credit_expiry: None,
                external: None,
            });
        }
    }
    out
}
/// Settings glue: render from the authoritative server-config projection.
/// External URLs open on the client machine; never invoke a remote editor.
pub fn render(config: &Value, cx: &mut App) -> AnyElement {
    let muted = cx.theme().muted_foreground;
    let accounts = accounts(config);
    let mut body = div()
        .id("subscription-usage")
        .flex()
        .flex_col()
        .gap_2()
        .child(div().text_sm().child("Usage limits"));
    if accounts.is_empty() {
        return body
            .child(
                div()
                    .text_xs()
                    .text_color(muted)
                    .child("No account usage reported by this environment."),
            )
            .into_any_element();
    }
    for (i, account) in accounts.into_iter().enumerate() {
        let mut row = div()
            .flex()
            .flex_col()
            .gap_1()
            .child(div().text_sm().child(account.label))
            .child(
                div()
                    .text_xs()
                    .text_color(muted)
                    .child(format!("Last checked {}", account.checked)),
            );
        let show_windows = account.notice.is_none();
        if let Some(notice) = account.notice {
            row = row.child(div().text_xs().text_color(muted).child(notice));
        }
        for w in account.windows.into_iter().filter(|_| show_windows) {
            let limit = if w.used >= 100. {
                " · Limit reached"
            } else if w.used >= 90. {
                " · Near limit"
            } else {
                ""
            };
            row = row.child(div().text_xs().child(format!(
                    "{} · {:.0}% used{limit}{}",
                    w.label,
                    w.used,
                    w.resets
                        .map(|s| format!(" · resets {s}"))
                        .unwrap_or_default()
                )));
        }
        if let Some(credits) = account.credits {
            row = row.child(div().text_xs().text_color(muted).child(format!(
                    "{credits} reset credits{}",
                    account
                        .credit_expiry
                        .map(|s| format!(" · next expires {s}"))
                        .unwrap_or_default()
                )));
        }
        if let Some((label, url)) = account.external {
            row = row.child(
                Button::new(("manage-usage", i))
                    .ghost()
                    .small()
                    .label(label)
                    .on_click(move |_, _, cx| cx.open_url(&url)),
            );
        }
        body = body.child(row);
    }
    body.into_any_element()
}
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn quota_windows_credits_external_and_probe_failure_are_not_context_usage() {
        let rows = accounts(
            &json!({"providers":[{"instanceId":"private@example.com","usageLimits":{"checkedAt":"now","windows":[{"label":"Weekly","usedPercent":100,"resetsAt":"later"},{"label":"Bad","usedPercent":110}],"resetCredits":{"availableCount":2,"nextExpiresAt":"soon"},"externalUsage":{"label":"Manage","url":"https://example.com/usage"},"unavailable":{"reason":"probeFailed"}}},{"instanceId":"absent"}]}),
        );
        assert_eq!(rows.len(), 1);
        assert!(!rows[0].label.contains('@'));
        assert_eq!(rows[0].windows.len(), 1);
        assert_eq!(rows[0].windows[0].used, 100.);
        assert_eq!(rows[0].windows[0].resets.as_deref(), Some("later"));
        assert_eq!(rows[0].credits, Some(2));
        assert!(rows[0].notice.as_deref().unwrap().contains("stale"));
        assert!(rows[0].external.is_some());
    }
    #[test]
    fn disabled_unavailable_and_unreported_instances_do_not_claim_zero_usage() {
        let rows = accounts(&json!({"providers":[
            {"instanceId":"absent"},
            {"instanceId":"disabled","enabled":false,"usageLimits":{"windows":[]}},
            {"instanceId":"unavailable","availability":"unavailable","usageLimits":{"windows":[]}},
            {"instanceId":"unknown","enabled":true,"installed":true,"usageLimits":{"checkedAt":"now","windows":[]}}
        ]}));
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].label, "unknown");
        assert!(rows[0].windows.is_empty());
        assert_eq!(rows[0].notice.as_deref(), Some("No limits reported."));
    }
    #[test]
    fn source_errors_and_unsupported_accounts_stay_visible_and_unsafe_urls_do_not() {
        let rows = accounts(
            &json!({"usageLimitSources":[{"label":"Hub","error":"Probe denied","accounts":[{"driver":"codex","usageLimits":{"windows":[],"unavailable":{"reason":"unsupported"},"externalUsage":{"label":"Bad","url":"javascript:alert(1)"}}}]}]}),
        );
        assert_eq!(rows.len(), 2);
        assert!(rows[0].windows.is_empty());
        assert!(rows[0].notice.is_some());
        assert!(rows[0].external.is_none());
        assert_eq!(rows[1].notice.as_deref(), Some("Probe denied"));
    }
}
