//! Status-backed header git actions. Status receipts are scoped to the checkout.
use crate::NativeApp;
use gpui_kit::component::{
    ActiveTheme as _, Disableable, Sizable,
    button::{Button, ButtonVariants},
};
use gpui_kit::{Context, div, prelude::*};
use serde_json::{Value, json};
use t3code_desktop_gpui::connection;

#[derive(Default)]
pub(crate) struct GitHeader {
    pub open: bool,
    pub loading: bool,
    pub status: Option<Value>,
    pub error: Option<String>,
    pub result_url: Option<String>,
    pub context: Option<(Option<String>, Option<String>, Option<String>)>,
    pub confirm: Option<String>,
    pub epoch: u64,
    pub progress: Vec<String>,
    pub trigger_bounds: std::rc::Rc<std::cell::Cell<Option<gpui_kit::Bounds<gpui_kit::Pixels>>>>,
    pub commit_message: Option<String>,
    pub file_paths: Option<Vec<String>>,
    pub form: Option<super::git_forms::GitForm>,
}

#[derive(Debug, PartialEq, Clone, Copy)]
pub(crate) struct GitChoice {
    pub label: &'static str,
    pub action: &'static str,
    pub reason: Option<&'static str>,
}
fn choice(label: &'static str, action: &'static str, reason: Option<&'static str>) -> GitChoice {
    GitChoice {
        label,
        action,
        reason,
    }
}
pub(crate) fn quick(status: &Value) -> GitChoice {
    let changes = status["hasWorkingTreeChanges"] == true;
    let remote = status["hasPrimaryRemote"] == true;
    let upstream = status["hasUpstream"] == true;
    let default = status["isDefaultRef"] == true;
    let pr = status["pr"]["state"] == "open";
    let ahead = status["aheadCount"].as_u64().unwrap_or(0);
    let behind = status["behindCount"].as_u64().unwrap_or(0);
    let delta = status["aheadOfDefaultCount"].as_u64().unwrap_or(ahead);
    if status["isRepo"] != true {
        return choice("Initialize Git", "init", None);
    }
    if !status["refName"].is_string() {
        return choice(
            "Commit",
            "commit",
            (!changes).then_some("No working-tree changes"),
        );
    }
    if remote && !status["hasUpstream"].is_boolean() {
        return choice(
            "Commit",
            "commit",
            (!changes).then_some("Remote status is unavailable"),
        );
    }
    if changes {
        return if !upstream && !remote {
            choice("Commit", "commit", None)
        } else if pr || default {
            choice("Commit & push", "commit_push", None)
        } else {
            choice("Commit, push & PR", "commit_push_pr", None)
        };
    }
    if !upstream && !remote {
        return if pr && ahead == 0 {
            choice("View PR", "open_pr", None)
        } else {
            choice("Publish repository", "publish", None)
        };
    }
    if !upstream && ahead == 0 {
        return if pr {
            choice("View PR", "open_pr", None)
        } else {
            choice("Push", "push", Some("No local commits to push"))
        };
    }
    if upstream && ahead > 0 && behind > 0 {
        return choice(
            "Sync ref",
            "pull",
            Some("Branch has diverged from upstream. Rebase/merge first."),
        );
    }
    if upstream && behind > 0 {
        return choice("Pull", "pull", None);
    }
    if ahead > 0 {
        return if pr || default {
            choice("Push", if default { "commit_push" } else { "push" }, None)
        } else {
            choice("Push & create PR", "create_pr", None)
        };
    }
    if pr {
        return choice("View PR", "open_pr", None);
    }
    if delta > 0 && !default {
        return choice("Create PR", "create_pr", None);
    }
    choice(
        "Commit",
        "commit",
        Some("Branch is up to date. No action needed."),
    )
}

impl NativeApp {
    pub(crate) fn initialize_header_git(&mut self, cx: &mut Context<Self>) {
        if !self.connection_ready() || self.pending_actions > 0 {
            return;
        }
        let (Some(cwd), Some(sender)) = (self.selected_workspace(), self.commands.clone()) else {
            return;
        };
        let generation = self.generation;
        let context = self.checkout_context();
        self.pending_actions += 1;
        self.git_header.error = None;
        cx.spawn(async move |entity, cx| {
            let result = connection::call(&sender, "vcs.init", json!({"cwd":cwd})).await;
            let _ = entity.update(cx, |app, cx| {
                app.pending_actions = app.pending_actions.saturating_sub(1);
                if app.generation != generation || app.checkout_context() != context {
                    cx.notify();
                    return;
                }
                match result {
                    Ok(_) => {
                        app.git_header.open = false;
                        app.toggle_git_header(cx);
                    }
                    Err(error) => {
                        app.git_header.error = Some(error.clone());
                        app.report_error(error, cx);
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
    pub(crate) fn render_git_split(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let action =
            self.live_git_status()
                .or_else(|| {
                    self.git_header.status.as_ref().filter(|_| {
                        self.git_header.context.as_ref() == Some(&self.checkout_context())
                    })
                })
                .map(quick);
        let bounds = self.git_header.trigger_bounds.clone();
        div()
            .relative()
            .flex()
            .flex_col()
            .child(
                gpui_kit::canvas(move |rect, _, _| bounds.set(Some(rect)), |_, _, _, _| {})
                    .absolute()
                    .size_full(),
            )
            .child(
                div()
                    .flex()
                    .gap_1()
                    .child(
                        Button::new("git-primary-action")
                            .debug_selector(|| "git-primary-action".into())
                            .small()
                            .secondary()
                            .label(action.map_or("Git", |a| a.label))
                            .disabled(
                                !self.connection_ready()
                                    || self.pending_actions > 0
                                    || self.git_header.form.is_some()
                                    || action.is_some_and(|a| a.reason.is_some()),
                            )
                            .tooltip(
                                action
                                    .and_then(|a| a.reason)
                                    .unwrap_or("Run quick action; use options to refresh status"),
                            )
                            .on_click(cx.listener(move |app, _, window, cx| {
                                if let Some(action) = action {
                                    app.execute_git_header(action.action, window, cx);
                                } else {
                                    app.focus.focus(window, cx);
                                    app.toggle_git_header(cx);
                                }
                            })),
                    )
                    .child(
                        Button::new("push-pr")
                            .debug_selector(|| "push-pr".into())
                            .small()
                            .secondary()
                            .icon(gpui_kit::assets::IconName::ChevronDown)
                            .tooltip("Git action options")
                            .disabled(!self.connection_ready())
                            .on_click(cx.listener(|app, _, window, cx| {
                                app.focus.focus(window, cx);
                                app.toggle_git_header(cx);
                            })),
                    ),
            )
            .when(self.git_header.open, |view| {
                view.child(gpui_kit::deferred(
                    gpui_kit::anchored()
                        .offset(gpui_kit::point(
                            gpui_kit::px(0.),
                            self.git_header
                                .trigger_bounds
                                .get()
                                .map_or(gpui_kit::px(21.), |bounds| bounds.size.height)
                                + gpui_kit::px(4.),
                        ))
                        .snap_to_window()
                        .child(self.render_git_header(cx)),
                ))
            })
    }
    pub(crate) fn toggle_git_header(&mut self, cx: &mut Context<Self>) {
        if self.git_header.open {
            self.git_header.open = false;
            cx.notify();
            return;
        }
        let (Some(sender), Some(cwd)) = (self.commands.clone(), self.selected_workspace()) else {
            self.report_error("Connect and select a checkout first", cx);
            return;
        };
        let context = self.checkout_context();
        let generation = self.generation;
        let epoch = self.git_header.epoch.wrapping_add(1);
        if self.pending_actions > 0 {
            self.git_header.open = true;
            cx.notify();
            return;
        }
        self.git_header = GitHeader {
            epoch,
            open: true,
            loading: true,
            context: Some(context.clone()),
            status: self.live_git_status().cloned(),
            ..Default::default()
        };
        cx.spawn(async move |entity, cx| {
            let result = connection::call(&sender, "vcs.refreshStatus", json!({"cwd":cwd})).await;
            let _ = entity.update(cx, |app, cx| {
                if app.generation != generation
                    || app.checkout_context() != context
                    || app.git_header.epoch != epoch
                {
                    return;
                }
                app.git_header.loading = false;
                match result {
                    Ok(value) => {
                        app.git_header.status =
                            Some(app.live_git_status().cloned().unwrap_or(value))
                    }
                    Err(error) => app.git_header.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
    pub(crate) fn execute_git_header(
        &mut self,
        action: &str,
        window: &mut gpui_kit::Window,
        cx: &mut Context<Self>,
    ) {
        if action == "init" {
            self.initialize_header_git(cx);
            return;
        }
        if !self.connection_ready()
            || self.git_header.context.as_ref() != Some(&self.checkout_context())
            || self.pending_actions > 0
        {
            return;
        }
        let Some(status) = self.git_header.status.as_ref() else {
            return;
        };
        if action == "open_pr" {
            if let Some(url) = status["pr"]["url"]
                .as_str()
                .filter(|url| url.starts_with("https://"))
            {
                cx.open_url(url);
            }
            return;
        }
        if status["isDefaultRef"] == true
            && action != "commit"
            && action != "pull"
            && self.git_header.confirm.as_deref() != Some(action)
        {
            self.git_header.confirm = Some(action.into());
            self.git_header.open = true;
            self.focus.focus(window, cx);
            cx.notify();
            return;
        }
        self.git_header.confirm = None;
        if action == "publish"
            || action.starts_with("commit") && status["hasWorkingTreeChanges"] == true
        {
            self.begin_git_form(action, window, cx);
            return;
        }
        if action == "pull" {
            if let Some(cwd) = self.selected_workspace() {
                self.perform("vcs.pull", json!({"cwd":cwd}), None, cx);
            }
        } else {
            self.run_git_action(action, false, cx);
        }
    }
    pub(crate) fn render_git_header(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let current = self.git_header.context.as_ref() == Some(&self.checkout_context());
        let busy = self.pending_actions > 0;
        let status = self
            .live_git_status()
            .or_else(|| self.git_header.status.as_ref().filter(|_| current));
        let primary = status.map(quick);
        let mut actions = Vec::new();
        if let Some(status) = status {
            let blocked = if busy {
                Some("Git action in progress")
            } else {
                None
            };
            actions.push(choice(
                "Commit",
                "commit",
                blocked
                    .or((status["isRepo"] != true).then_some("Not a repository"))
                    .or((status["hasWorkingTreeChanges"] != true)
                        .then_some("No working-tree changes")),
            ));
            if status["hasPrimaryRemote"] == true && status["hasUpstream"].is_boolean() {
                actions.push(choice(
                    "Push",
                    "push",
                    blocked.or_else(|| {
                        if !status["refName"].is_string() {
                            Some("Detached ref")
                        } else if status["behindCount"].as_u64().unwrap_or(0) > 0 {
                            Some("Pull or resolve divergence first")
                        } else if status["aheadCount"].as_u64().unwrap_or(0) == 0 {
                            Some("No local commits to push")
                        } else {
                            None
                        }
                    }),
                ));
                actions.push(if status["pr"]["state"] == "open" {
                    choice("View PR", "open_pr", blocked)
                } else {
                    let delta = status["aheadOfDefaultCount"]
                        .as_u64()
                        .or(status["aheadCount"].as_u64())
                        .unwrap_or(0);
                    choice(
                        "Create PR",
                        "create_pr",
                        blocked
                            .or(
                                (!status["refName"].is_string() || status["isDefaultRef"] == true)
                                    .then_some("Create PR from a feature branch"),
                            )
                            .or((status["hasWorkingTreeChanges"] == true
                                || delta == 0
                                || status["behindCount"].as_u64().unwrap_or(0) > 0)
                                .then_some(
                                    "Commit changes and bring the branch ahead of its base first",
                                )),
                    )
                });
            }
        }
        div()
            .id("git-menu-popover")
            .debug_selector(|| "git-menu-popover".into())
            .occlude()
            .w(gpui_kit::px(360.))
            .max_h(gpui_kit::px(480.))
            .overflow_y_scroll()
            .on_mouse_down_out(cx.listener(|app, event: &gpui_kit::MouseDownEvent, _, cx| {
                // The trigger handles its own toggle; outside dismissal must not
                // close it on mouse-down and reopen it again on click.
                if !app
                    .git_header
                    .trigger_bounds
                    .get()
                    .is_some_and(|bounds| bounds.contains(&event.position))
                {
                    app.dismiss_git_header(cx);
                }
            }))
            .flex()
            .flex_col()
            .gap_1()
            .p_2()
            .bg(cx.theme().popover)
            .border_1()
            .border_color(cx.theme().border)
            .child(
                Button::new("git-refresh-status")
                    .small()
                    .ghost()
                    .label("Refresh status")
                    .disabled(busy || self.git_header.loading)
                    .on_click(cx.listener(|app, _, _, cx| {
                        app.git_header.open = false;
                        app.toggle_git_header(cx);
                    })),
            )
            .when(self.git_header.loading, |view| {
                view.child("Loading git status…")
            })
            .when(!current, |view| {
                view.child("Checkout changed; reopen Git to refresh")
            })
            .children(self.git_header.error.clone())
            .when(status.is_some_and(|status| status["isRepo"] == true && !status["refName"].is_string()), |view| {
                view.child(div().text_xs().child("Detached HEAD: commit is available; create and check out a branch before pushing or opening a PR"))
            })
            .when_some(self.live_git_error(), |view, error| {
                view.child(format!("Live checkout status unavailable: {error}"))
            })
            .when_some(
                self.git_header.result_url.clone().filter(|_| current),
                |view, url| {
                    view.child(
                        Button::new("git-publish-open")
                            .small()
                            .ghost()
                            .label("Open published repository")
                            .on_click(cx.listener(move |_, _, _, cx| cx.open_url(&url))),
                    )
                },
            )
            .children(
                self.git_header
                    .progress
                    .iter()
                    .cloned()
                    .map(|text| div().text_xs().child(text)),
            )
            .when_some(
                self.git_header.form.as_ref().filter(|_| current),
                |view, _| view.child(self.render_git_form(cx)),
            )
            .when_some(
                primary.filter(|_| self.git_header.form.is_none()),
                |view, action| {
                    view.child(
                        Button::new("git-quick-action")
                            .debug_selector(|| "git-quick-action".into())
                            .small()
                            .secondary()
                            .label(action.label)
                            .disabled(busy || action.reason.is_some() || !self.connection_ready())
                            .tooltip(action.reason.unwrap_or("Run the applicable git action"))
                            .on_click(cx.listener(move |app, _, window, cx| {
                                app.execute_git_header(action.action, window, cx)
                            })),
                    )
                    .when_some(
                        action.reason.or(busy.then_some("Git action in progress")),
                        |view, reason| view.child(div().text_xs().child(reason)),
                    )
                },
            )
            .children(
                actions
                    .into_iter()
                    .filter(|_| self.git_header.form.is_none())
                    .map(|action| {
                        div()
                            .flex()
                            .flex_col()
                            .child(
                                Button::new(gpui_kit::SharedString::from(format!(
                                    "git-menu-{}",
                                    action.action
                                )))
                                .small()
                                .ghost()
                                .justify_start()
                                .label(action.label)
                                .disabled(action.reason.is_some() || !self.connection_ready())
                                .tooltip(action.reason.unwrap_or(action.label))
                                .on_click(cx.listener(
                                    move |app, _, window, cx| {
                                        app.execute_git_header(action.action, window, cx)
                                    },
                                )),
                            )
                            .when_some(action.reason, |view, reason| {
                                view.child(div().text_xs().child(reason))
                            })
                    }),
            )
            .when_some(
                self.git_header.confirm.clone().filter(|_| current),
                |view, action| {
                    view.child(
                        Button::new("git-default-confirm")
                            .debug_selector(|| "git-default-confirm".into())
                            .small()
                            .label("Confirm action on default branch")
                            .on_click(cx.listener(move |app, _, window, cx| {
                                app.execute_git_header(&action, window, cx)
                            })),
                    )
                },
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn status() -> Value {
        json!({"isRepo":true,"refName":"feature","hasPrimaryRemote":true,"hasUpstream":true,"aheadCount":0,"behindCount":0,"hasWorkingTreeChanges":false})
    }
    #[test]
    fn git_quick_actions_follow_changes_pr_and_remote_state() {
        let mut s = status();
        assert!(quick(&s).reason.is_some());
        s["hasWorkingTreeChanges"] = json!(true);
        assert_eq!(quick(&s).action, "commit_push_pr");
        s["pr"] = json!({"state":"open"});
        assert_eq!(quick(&s).action, "commit_push");
        s["hasUpstream"] = json!(false);
        s["hasPrimaryRemote"] = json!(false);
        assert_eq!(quick(&s).action, "commit");
    }
    #[test]
    fn git_quick_actions_do_not_push_diverged_or_detached_refs() {
        let mut s = status();
        s["aheadCount"] = json!(2);
        s["behindCount"] = json!(1);
        assert_eq!(quick(&s).label, "Sync ref");
        assert!(quick(&s).reason.is_some());
        s["aheadCount"] = json!(0);
        assert_eq!(quick(&s).action, "pull");
        s["refName"] = Value::Null;
        assert!(quick(&s).reason.is_some());
    }
}
