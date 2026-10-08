//! Cleanup review uses the server's review token and per-action execution findings.
use crate::NativeApp;
use gpui_kit::component::{
    ActiveTheme as _, Disableable, Sizable,
    button::{Button, ButtonVariants as _},
    checkbox::Checkbox,
};
use gpui_kit::{Context, SharedString, div, prelude::*, px};
use serde_json::json;
use t3code_desktop_gpui::{connection, sidebar};

impl NativeApp {
    pub(crate) fn review_sidebar_cleanup(&mut self, run: bool, cx: &mut Context<Self>) {
        let state = &self.sidebar_state.cleanup;
        if state.busy
            || self.sidebar_state.capabilities["threadCleanupReview"].as_bool() != Some(true)
        {
            return;
        }
        let Some((environment, target)) = state
            .target
            .clone()
            .filter(|(environment, _)| environment == &self.environment_scope)
        else {
            return;
        };
        let Some(sender) = self.commands.clone() else {
            self.report_error("Connect before reviewing cleanup", cx);
            return;
        };
        let payload = if run {
            let Some(review) = &state.review else {
                return;
            };
            let selected: Vec<_> = state
                .selected
                .iter()
                .filter(|id| {
                    review
                        .actions
                        .iter()
                        .any(|action| &action.id == *id && action.blocked_reason.is_none())
                })
                .cloned()
                .collect();
            if selected.is_empty() {
                return;
            }
            json!({"threadId":target,"reviewId":review.review_id,"selected":selected})
        } else {
            json!({"threadId":target})
        };
        if run && self.selected_thread.as_ref() == Some(&target) && !self.can_change_checkout(cx) {
            return;
        }
        let state = &mut self.sidebar_state.cleanup;
        state.epoch += 1;
        let epoch = state.epoch;
        state.busy = true;
        state.generation = self.generation;
        state.error = None;
        if !run {
            state.review = None;
            state.result = None;
            state.selected.clear();
        }
        let generation = self.generation;
        cx.spawn(async move |entity, cx| {
            let response = connection::call(
                &sender,
                if run {
                    "thread.cleanupRun"
                } else {
                    "thread.cleanupReview"
                },
                payload,
            )
            .await;
            let _ = entity.update(cx, |app, cx| {
                if app.generation != generation || app.environment_scope != environment {
                    return;
                }
                let state = &mut app.sidebar_state.cleanup;
                if state.epoch != epoch
                    || state.target.as_ref() != Some(&(environment, target.clone()))
                {
                    return;
                }
                state.busy = false;
                let result = response.and_then(|value| {
                    if run {
                        let result: sidebar::CleanupResult =
                            serde_json::from_value(value).map_err(|error| error.to_string())?;
                        if result.thread_id != target
                            || result.actions.iter().any(|action| {
                                !matches!(
                                    action.status.as_str(),
                                    "completed" | "deferred" | "failed"
                                )
                            })
                        {
                            return Err("Invalid cleanup result".into());
                        }
                        state.result = Some(result);
                    } else {
                        let review: sidebar::CleanupReview =
                            serde_json::from_value(value).map_err(|error| error.to_string())?;
                        if review.thread_id != target
                            || review.review_id.is_empty()
                            || review.actions.len() > 3
                            || review.actions.iter().any(|action| {
                                !matches!(
                                    action.id.as_str(),
                                    "worktree" | "remote-branch" | "archive"
                                )
                            })
                        {
                            return Err("Invalid cleanup review".into());
                        }
                        state.review = Some(review);
                    }
                    Ok(())
                });
                if let Err(error) = result {
                    state.error = Some(error);
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    pub(crate) fn render_sidebar_cleanup(
        &self,
        cx: &mut Context<Self>,
    ) -> Option<impl IntoElement + use<>> {
        let state = &self.sidebar_state.cleanup;
        let muted = cx.theme().muted_foreground;
        let error = cx.theme().danger;
        state.target.as_ref().filter(|(environment, _)| environment == &self.environment_scope).map(|_| {
            let mut panel = div().id("sidebar-cleanup").debug_selector(|| "sidebar-cleanup".into()).flex().flex_col().gap_2().p_2().max_h(px(320.)).overflow_y_scroll()
                .child(if state.result.is_some() { "Cleanup results" } else { "Finish tidying up?" });
            if let Some(review) = &state.review {
                panel = panel.child(review.title.clone());
                for action in &review.actions {
                    let id = action.id.clone();
                    let outcome = state.result.as_ref().and_then(|result| result.actions.iter().find(|outcome| outcome.id == id));
                    panel = panel.child(div().debug_selector({ let id = id.clone(); move || format!("cleanup-choice-{id}") }).flex().flex_col().gap_1()
                        .child(Checkbox::new(SharedString::from(format!("cleanup-select-{id}")))
                            .label(action.title.clone()).checked(state.selected.contains(&id))
                            .disabled(state.busy || state.result.is_some() || action.blocked_reason.is_some())
                            .on_click(cx.listener(move |app, checked, _, cx| {
                                app.sidebar_state.cleanup.selected.retain(|entry| entry != &id);
                                if *checked { app.sidebar_state.cleanup.selected.push(id.clone()); }
                                cx.notify();
                            })))
                        .child(div().text_xs().text_color(muted).child(outcome.map(|outcome| format!("{}: {}", outcome.status, outcome.detail)).unwrap_or_else(|| action.blocked_reason.clone().unwrap_or_else(|| if state.result.is_some() { "Kept: you did not select this action.".into() } else { action.detail.clone() })))));
                }
            }
            if state.busy { panel = panel.child("Checking sessions, worktrees and files…"); }
            if let Some(message) = &state.error { panel = panel.child(div().text_color(error).child(message.clone())); }
            panel.child(div().text_xs().text_color(muted).child("Each action is checked again before it runs. Nothing happens until Run selected."))
                .child(Button::new("cleanup-review-again").debug_selector(|| "cleanup-review-again".into()).label("Review again").ghost().small().disabled(state.busy)
                    .on_click(cx.listener(|app, _, _, cx| app.review_sidebar_cleanup(false, cx))))
                .when(state.result.is_none(), |panel| panel.child(Button::new("cleanup-run").debug_selector(|| "cleanup-run".into()).label("Run selected").danger().small().disabled(state.busy || state.selected.is_empty() || !self.connection_ready())
                    .on_click(cx.listener(|app, _, _, cx| app.review_sidebar_cleanup(true, cx)))))
                .child(Button::new("cleanup-close").debug_selector(|| "cleanup-close".into()).label(if state.result.is_some() { "Done" } else { "Do nothing" }).ghost().small().disabled(state.busy)
                    .on_click(cx.listener(|app, _, _, cx| { app.sidebar_state.cleanup.target = None; app.sidebar_state.cleanup.epoch += 1; cx.notify(); })))
        })
    }
}
