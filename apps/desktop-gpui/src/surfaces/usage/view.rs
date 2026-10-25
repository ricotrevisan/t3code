//! Scoped subscription usage interactions. Never reuse a receipt across reconnects.
use crate::NativeApp;
use gpui_kit::component::{
    ActiveTheme as _, Disableable, Sizable,
    button::{Button, ButtonVariants},
    popover::Popover,
};
use gpui_kit::{AnyElement, App, Context, div, prelude::*, rems};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use t3code_desktop_gpui::{connection, surfaces::usage};

#[derive(Default)]
pub(crate) struct State {
    scope: Option<(String, u64)>,
    open: bool,
    revealed: HashSet<String>,
    confirming: Option<(String, Value)>,
    busy: HashSet<String>,
    results: HashMap<String, String>,
}
// Retain the existing initialization hook; state is owned by each environment page.
pub(crate) fn init(_cx: &mut App) {}
impl NativeApp {
    #[cfg(test)]
    pub(crate) fn usage_feedback(&self, key: &str, _cx: &App) -> (Option<String>, bool) {
        let state = &self.usage_state;
        (
            state.results.get(key).cloned(),
            state.revealed.contains(key),
        )
    }

    pub(crate) fn toggle_usage_limits(&mut self, cx: &mut Context<Self>) {
        let scope = (self.environment_scope.clone(), self.generation);
        let state = &mut self.usage_state;
        if state.scope.as_ref() != Some(&scope) {
            *state = State {
                scope: Some(scope),
                ..Default::default()
            };
        }
        state.open = !state.open;
        if !state.open {
            state.confirming = None;
            state.revealed.clear();
        }
        cx.notify();
    }
    pub(crate) fn render_usage_limits_entry(
        &self,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let state = &self.usage_state;
        let open = state.open
            && state.scope.as_ref() == Some(&(self.environment_scope.clone(), self.generation));
        let app = cx.entity().downgrade();
        Popover::new("usage-popover")
            .anchor(gpui_kit::Anchor::BottomRight)
            .open(open)
            .trigger(
                Button::new("usage-limits-entry")
                    .debug_selector(|| "usage-limits-entry".into())
                    .label("Usage")
                    .ghost()
                    .xsmall()
                    .tooltip("Subscription usage limits (not context tokens)"),
            )
            .on_open_change(cx.listener(|app, requested: &bool, _, cx| {
                let state = &app.usage_state;
                let current = state.open
                    && state.scope.as_ref()
                        == Some(&(app.environment_scope.clone(), app.generation));
                if current != *requested {
                    app.toggle_usage_limits(cx);
                }
            }))
            .content(move |_, _, cx| {
                app.update(cx, |app, cx| {
                    app.render_usage_limits_panel(cx)
                        .unwrap_or_else(|| div().into_any_element())
                })
                .unwrap_or_else(|_| div().into_any_element())
            })
    }
    fn usage_accounts(&self) -> Vec<usage::Account> {
        usage::accounts(
            self.server_config
                .config(self.generation)
                .unwrap_or(&Value::Null),
        )
    }
    fn redeem_usage_credit(&mut self, key: &str, cx: &mut Context<Self>) {
        let scope = (self.environment_scope.clone(), self.generation);
        let state = &self.usage_state;
        if state.scope.as_ref() != Some(&scope)
            || state.busy.contains(key)
            || !self.connection_ready()
        {
            return;
        }
        let Some((confirmed_key, input)) = state.confirming.clone() else {
            return;
        };
        if confirmed_key != key {
            return;
        }
        let valid = self.usage_accounts().iter().any(|row| {
            row.key == key
                && row.credits.unwrap_or(0) > 0
                && row.reset_input.as_ref() == Some(&input)
        });
        if !valid {
            let state = &mut self.usage_state;
            state.confirming = None;
            state.results.insert(
                key.into(),
                "Credit changed; review the current account and try again.".into(),
            );
            cx.notify();
            return;
        }
        let Some(sender) = self.commands.clone() else {
            return;
        };
        let state = &mut self.usage_state;
        state.confirming = None;
        state.busy.insert(key.into());
        state.results.remove(key);
        let key = key.to_owned();
        cx.spawn(async move |entity, cx| {
            let result = connection::call(&sender, "provider.consumeResetCredit", input).await;
            _ = entity.update(cx, |app, cx| {
                if (app.environment_scope.clone(), app.generation) != scope || app.usage_state.scope.as_ref() != Some(&scope) { return; }
                let text = match result {
                    Ok(value) => value["warning"].as_str().map(str::to_owned).unwrap_or_else(|| match value["outcome"].as_str() {
                        Some("reset") => "Reset applied. Your windows have cleared.",
                        Some("nothingToReset") => "Nothing to reset right now.",
                        Some("noCredit") => "No reset credit left.",
                        Some("alreadyRedeemed") => "That credit was already redeemed.",
                        _ => "Server returned an unknown reset-credit outcome; refresh before retrying.",
                    }.into()),
                    Err(error) => format!("Could not use the reset credit: {error}"),
                };
                let state = &mut app.usage_state;
                state.busy.remove(&key);
                state.results.insert(key.clone(), text);
                cx.notify();
            });
        }).detach();
        cx.notify();
    }
    pub(crate) fn render_usage_limits_panel(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let state = &self.usage_state;
        if !state.open
            || state.scope.as_ref() != Some(&(self.environment_scope.clone(), self.generation))
        {
            return None;
        }
        let rows = self.usage_accounts();
        let now = usage::formatting::now();
        let muted = cx.theme().muted_foreground;
        let mut panel = div()
            .id("usage-limits-panel")
            .debug_selector(|| "usage-limits-panel".into())
            .occlude()
            .w(rems(24.))
            .max_h(rems(28.))
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .gap_2()
            .text_color(cx.theme().popover_foreground)
            .child(
                div()
                    .flex()
                    .justify_between()
                    .items_center()
                    .child("Usage limits")
                    .child(
                        Button::new("usage-close")
                            .debug_selector(|| "usage-close".into())
                            .label("Close")
                            .ghost()
                            .small()
                            .on_click(cx.listener(|app, _, _, cx| {
                                app.usage_state.open = false;
                                app.usage_state.confirming = None;
                                app.usage_state.revealed.clear();
                                cx.notify();
                            })),
                    ),
            );
        if rows.is_empty() {
            panel = panel.child(
                div()
                    .text_sm()
                    .text_color(muted)
                    .child("No account usage reported by this environment."),
            );
        }
        for (index, row) in rows.into_iter().enumerate() {
            let state = &self.usage_state;
            let key = row.key.clone();
            let busy = state.busy.contains(&key);
            let confirming = state.confirming.as_ref().is_some_and(|(k, _)| k == &key);
            let mut body = div()
                .flex()
                .flex_col()
                .gap_1()
                .child(div().text_sm().child(row.label));
            if let Some(identity) = row.sensitive_label {
                let revealed = state.revealed.contains(&key);
                let toggle_key = key.clone();
                body = body.child(
                    Button::new(("usage-reveal", index))
                        .debug_selector(move || format!("usage-reveal-{index}"))
                        .label(if revealed {
                            identity
                        } else {
                            "Reveal account".into()
                        })
                        .ghost()
                        .small()
                        .on_click(cx.listener(move |app, _, _, cx| {
                            let state = &mut app.usage_state;
                            if !state.revealed.remove(&toggle_key) {
                                state.revealed.insert(toggle_key.clone());
                            }
                            cx.notify();
                        })),
                );
            }
            body = body.child(
                div()
                    .text_xs()
                    .text_color(muted)
                    .child(usage::formatting::checked(&row.checked, now)),
            );
            if let Some(notice) = row.notice {
                body = body.child(div().text_xs().text_color(muted).child(notice));
            } else {
                for window in row.windows {
                    body = body.child(div().text_xs().child(format!(
                        "{} · {:.0}% used{}",
                        window.label,
                        window.used,
                        usage::formatting::resets(window.resets.as_deref(), now)
                    )));
                }
            }
            if let Some(count) = row.credits {
                let mut credit_row = div()
                    .debug_selector(move || format!("usage-credit-row-{index}"))
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .justify_start()
                    .gap_2()
                    .child(div().text_xs().child(usage::formatting::credits(
                        count,
                        row.credit_expiry.as_deref(),
                        now,
                    )));
                if count > 0
                    && let Some(input) = row.reset_input
                {
                    let action_key = key.clone();
                    credit_row = credit_row.child(
                        Button::new(("usage-credit", index))
                            .debug_selector(move || format!("usage-credit-{index}"))
                            .label(if busy { "Using…" } else { "Use reset" })
                            .secondary()
                            .small()
                            .disabled(busy || !self.connection_ready())
                            .on_click(cx.listener(move |app, _, _, cx| {
                                app.usage_state.confirming =
                                    Some((action_key.clone(), input.clone()));
                                cx.notify();
                            })),
                    );
                }
                body = body.child(credit_row);
            }
            if confirming {
                let confirm_key = key.clone();
                body = body
                    .child(div().text_sm().child(
                        "Spend one credit to clear the rate-limit windows? This cannot be undone.",
                    ))
                    .child(
                        div()
                            .flex()
                            .gap_2()
                            .child(
                                Button::new(("usage-confirm", index))
                                    .debug_selector(move || format!("usage-confirm-{index}"))
                                    .label("Use credit")
                                    .primary()
                                    .small()
                                    .disabled(busy || !self.connection_ready())
                                    .on_click(cx.listener(move |app, _, _, cx| {
                                        app.redeem_usage_credit(&confirm_key, cx)
                                    })),
                            )
                            .child(
                                Button::new(("usage-cancel", index))
                                    .debug_selector(move || format!("usage-cancel-{index}"))
                                    .label("Cancel")
                                    .ghost()
                                    .small()
                                    .on_click(cx.listener(|app, _, _, cx| {
                                        app.usage_state.confirming = None;
                                        cx.notify();
                                    })),
                            ),
                    );
            }
            if let Some(result) = self.usage_state.results.get(&key) {
                body = body.child(
                    div()
                        .debug_selector(move || format!("usage-result-{index}"))
                        .text_sm()
                        .child(result.clone()),
                );
            }
            if let Some((label, url)) = row.external {
                body = body.child(
                    Button::new(("usage-external", index))
                        .label(label)
                        .ghost()
                        .small()
                        .on_click(move |_, _, cx| cx.open_url(&url)),
                );
            }
            panel = panel.child(body);
        }
        Some(panel.into_any_element())
    }
}
