//! Restrained Kit controls; the patch's visible rows are virtualized, parsed once at receipt.
use super::model::{contains, reference, text};
use crate::NativeApp;
use gpui_kit::base::Disableable as _;
use gpui_kit::component::{
    ActiveTheme as _, Selectable as _, Sizable as _,
    button::{Button, ButtonVariants as _},
    input::{Input, Textarea},
    popover::Popover,
};
use gpui_kit::{AnyElement, Context, SharedString, div, prelude::*, px, uniform_list};
use serde_json::json;

impl NativeApp {
    pub(crate) fn render_pull_requests(&self, cx: &mut Context<Self>) -> AnyElement {
        let state = &self.pull_requests;
        let ready = self.connection_ready() && state.current(self);
        let mut view = div()
            .id("pull-requests-panel")
            .debug_selector(|| "pull-requests-panel".into())
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .min_w_0()
            .p_3()
            .bg(cx.theme().group_box)
            .border_l_1()
            .border_color(cx.theme().border)
            .gap_2();
        if !state.current(self) {
            return view
                .child(
                    Button::new("pr-open")
                        .ghost()
                        .small()
                        .label("Open pull requests")
                        .on_click(
                            cx.listener(|app, _, window, cx| app.open_pull_requests(window, cx)),
                        ),
                )
                .into_any_element();
        }
        view = view.child(
            div()
                .flex()
                .gap_1()
                .child(
                    Button::new("pr-list-back")
                        .ghost()
                        .small()
                        .label("Pull requests")
                        .on_click(cx.listener(|app, _, _, cx| {
                            app.pull_requests.epoch += 1;
                            app.pull_requests.reference = None;
                            app.pull_requests.loading.clear();
                            app.pull_requests.busy = false;
                            app.pull_requests.merge_confirm = false;
                            app.pull_requests.anchor = None;
                            app.pr_list(false, cx);
                        })),
                )
                .child(
                    Button::new("pr-refresh")
                        .debug_selector(|| "pr-refresh".into())
                        .ghost()
                        .small()
                        .label("Refresh")
                        .disabled(!ready || state.busy)
                        .on_click(cx.listener(|app, _, _, cx| app.pr_refresh(cx))),
                ),
        );
        if !ready {
            view = view.child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child("Disconnected or synchronizing — actions unavailable"),
            );
        }
        for (slot, error) in &state.errors {
            view = view.child(
                div()
                    .text_sm()
                    .text_color(cx.theme().danger)
                    .child(format!("{slot}: {error}")),
            );
        }
        if !state.loading.is_empty() {
            view = view.child(div().text_xs().child(format!(
                "Loading {}…",
                state.loading.iter().copied().collect::<Vec<_>>().join(", ")
            )));
        }
        if state.reference.is_none() {
            view.child(self.render_pr_list(cx)).into_any_element()
        } else {
            view.child(self.render_pr_detail(cx)).into_any_element()
        }
    }
    fn render_pr_list(&self, cx: &mut Context<Self>) -> AnyElement {
        let state = &self.pull_requests;
        let ready = self.connection_ready() && state.current(self);
        let mut view = div().flex().flex_col().gap_2().flex_1().min_h_0().min_w_0();
        if let Some(input) = &state.query {
            view = view.child(Input::new(input)).child(
                Button::new("pr-search")
                    .debug_selector(|| "pr-search".into())
                    .ghost()
                    .small()
                    .label("Search")
                    .disabled(!ready || state.loading.contains("list"))
                    .on_click(cx.listener(|app, _, _, cx| app.pr_list(false, cx))),
            );
        }
        view = view
            .child(
                div().flex().flex_wrap().gap_1().children(
                    ["open", "closed", "merged", "all"]
                        .into_iter()
                        .map(|filter| {
                            Button::new(SharedString::from(format!("pr-state-{filter}")))
                                .ghost()
                                .small()
                                .label(filter)
                                .selected(state.state_filter == filter)
                                .disabled(!ready)
                                .on_click(cx.listener(move |app, _, _, cx| {
                                    app.pull_requests.state_filter = filter;
                                    app.pr_list(false, cx);
                                }))
                        }),
                ),
            )
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap_1()
                    .children(
                        [
                            ("all", "Everyone"),
                            ("authored", "Authored"),
                            ("reviewing", "Reviewing"),
                        ]
                        .into_iter()
                        .map(|(filter, label)| {
                            Button::new(SharedString::from(format!("pr-involvement-{filter}")))
                                .ghost()
                                .small()
                                .label(label)
                                .selected(state.involvement == filter)
                                .disabled(!ready)
                                .on_click(cx.listener(move |app, _, _, cx| {
                                    app.pull_requests.involvement = filter;
                                    app.pr_list(false, cx);
                                }))
                        }),
                    )
                    .child(
                        Button::new("pr-project-filter")
                            .ghost()
                            .small()
                            .label("This project")
                            .selected(state.project_only)
                            .disabled(!ready || self.selected_project.is_none())
                            .on_click(cx.listener(|app, _, _, cx| {
                                app.pull_requests.project_only = !app.pull_requests.project_only;
                                app.pr_list(false, cx);
                            })),
                    ),
            );
        // Provider and project failures are visible even when another repository returned rows.
        for error in state.list["errors"].as_array().into_iter().flatten() {
            view = view.child(div().text_xs().child(format!(
                "{}: {}",
                text(error, "projectId"),
                text(error, "message")
            )));
        }
        for provider in state.list["providers"].as_array().into_iter().flatten() {
            if provider["configured"] == false {
                view = view.child(div().text_xs().child(format!(
                    "{}: {}",
                    text(provider, "host"),
                    text(provider, "detail")
                )));
            }
            if provider["searchesOnHost"] == false {
                view = view.child(div().text_xs().child(format!(
                    "{} search covers only the loaded rows",
                    text(provider, "host")
                )));
            }
        }
        for (index, link) in self
            .detail
            .thread
            .as_ref()
            .filter(|d| d["id"].as_str() == self.selected_thread.as_deref())
            .and_then(|d| d["pullRequests"].as_array())
            .into_iter()
            .flatten()
            .filter(|l| l["source"] != "stack-dismissed")
            .enumerate()
        {
            let url = text(link, "url").to_owned();
            view = view.child(
                Button::new(SharedString::from(format!("pr-linked-{index}")))
                    .ghost()
                    .small()
                    .label(format!("Linked PR #{}", link["number"]))
                    .disabled(!ready)
                    .on_click(cx.listener(move |app, _, window, cx| {
                        app.open_pull_request_url(&url, window, cx)
                    })),
            );
        }
        let query = state
            .query
            .as_ref()
            .map(|q| q.read(cx).value().trim().to_lowercase())
            .unwrap_or_default();
        if state.project_only && state.list_project != self.selected_project {
            return view
                .child(
                    div()
                        .text_sm()
                        .child("Project changed — refresh to load this project's pull requests"),
                )
                .into_any_element();
        }
        let entries = state.list_rows.clone();
        let mut cache = state.matches.borrow_mut();
        if cache.revision != state.list_revision || cache.query != query {
            cache.indices = std::sync::Arc::new(
                entries
                    .iter()
                    .enumerate()
                    .filter_map(|(index, row)| {
                        let local_search = state.list["providers"]
                            .as_array()
                            .into_iter()
                            .flatten()
                            .any(|p| p["host"] == row["host"] && p["searchesOnHost"] == false);
                        (!local_search
                            || query.is_empty()
                            || format!(
                                "{} {} {} {} {}",
                                row["number"],
                                text(row, "title"),
                                text(row, "repository"),
                                text(row, "headBranch"),
                                text(&row["author"], "login")
                            )
                            .to_lowercase()
                            .contains(&query))
                        .then_some(index)
                    })
                    .collect(),
            );
            cache.revision = state.list_revision;
            cache.query = query;
        }
        let indices = cache.indices.clone();
        drop(cache);
        if indices.is_empty() && !state.loading.contains("list") && !state.list.is_null() {
            view = view.child(
                div()
                    .text_sm()
                    .child("No pull requests match these filters"),
            );
        }
        view = view.child(
            uniform_list(
                "pr-list-rows",
                indices.len(),
                cx.processor(move |_, range: std::ops::Range<usize>, _, cx| {
                    range
                        .map(|index| {
                            let row = &entries[indices[index]];
                            let r = reference(row);
                            Button::new(SharedString::from(format!("pr-row-{index}")))
                                .debug_selector(move || format!("pr-row-{index}"))
                                .ghost()
                                .small()
                                .justify_start()
                                .label(format!(
                                    "#{} {} · {} · {}",
                                    row["number"],
                                    text(row, "title"),
                                    text(row, "state"),
                                    text(row, "repository")
                                ))
                                .disabled(r.is_none() || !ready)
                                .on_click(cx.listener(move |app, _, window, cx| {
                                    if let Some(r) = r.clone() {
                                        app.pr_open(r, window, cx);
                                    }
                                }))
                                .into_any_element()
                        })
                        .collect()
                }),
            )
            .flex_1()
            .min_h_0(),
        );
        if state.list["truncated"] == true
            && state.list["nextCursors"]
                .as_object()
                .is_none_or(|c| c.is_empty())
        {
            view = view.child(div().text_xs().child("This host capped the listing and has no continuation. Narrow the search or read the rest on the host."));
        }
        if state.list["nextCursors"]
            .as_object()
            .is_some_and(|c| !c.is_empty())
        {
            view = view.child(
                Button::new("pr-list-more")
                    .ghost()
                    .small()
                    .label("Load more pull requests")
                    .disabled(!ready || state.loading.contains("list-more"))
                    .on_click(cx.listener(|app, _, _, cx| app.pr_list(true, cx))),
            );
        }
        view.into_any_element()
    }
    fn render_pr_detail(&self, cx: &mut Context<Self>) -> AnyElement {
        let state = &self.pull_requests;
        let d = &state.detail;
        let ready = self.pr_can_mutate();
        if d.is_null() {
            return div()
                .text_sm()
                .child("Waiting for pull-request detail. Refresh to retry a failed read.")
                .into_any_element();
        }
        let linked = self
            .detail
            .thread
            .as_ref()
            .filter(|d| d["id"].as_str() == self.selected_thread.as_deref())
            .and_then(|d| d["pullRequests"].as_array())
            .into_iter()
            .flatten()
            .any(|l| {
                l["source"] != "stack-dismissed"
                    && state.reference.as_ref().is_some_and(|r| {
                        r["host"] == l["host"]
                            && r["repository"] == l["repository"]
                            && r["number"] == l["number"]
                    })
            });
        let can_link = ready
            && self.selected_thread.is_some()
            && state
                .reference
                .as_ref()
                .is_some_and(|r| r["projectId"].as_str() == self.selected_project.as_deref())
            && self.sidebar_state.capabilities["threadPullRequests"] == true;
        let mut view = div()
            .flex()
            .flex_col()
            .gap_2()
            .flex_1()
            .min_h_0()
            .min_w_0()
            .child(
                div()
                    .text_sm()
                    .child(format!("#{} {}", d["number"], text(d, "title"))),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(format!(
                        "{}{} · {} → {} · {}",
                        text(d, "state"),
                        if d["isDraft"] == true { " (draft)" } else { "" },
                        text(d, "headBranch"),
                        text(d, "baseBranch"),
                        text(d, "mergeability")
                    )),
            )
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap_1()
                    .child(
                        Button::new("pr-link")
                            .debug_selector(|| "pr-link".into())
                            .ghost()
                            .small()
                            .label(if linked {
                                "Unlink from thread"
                            } else {
                                "Link to thread"
                            })
                            .disabled(!can_link)
                            .on_click(cx.listener(move |app, _, _, cx| app.pr_link(linked, cx))),
                    )
                    .child(
                        Button::new("pr-open-host")
                            .ghost()
                            .small()
                            .label("Open on host")
                            .on_click({
                                let url = text(d, "url").to_owned();
                                move |_, _, cx| {
                                    if url::Url::parse(&url)
                                        .is_ok_and(|u| matches!(u.scheme(), "http" | "https"))
                                    {
                                        cx.open_url(&url);
                                    }
                                }
                            }),
                    )
                    .child(self.render_pr_checks(cx))
                    .child(
                        Button::new("pr-handoff")
                            .debug_selector(|| "pr-handoff".into())
                            .ghost()
                            .small()
                            .label("Add PR to message")
                            .disabled(self.selected_thread.is_none())
                            .on_click(cx.listener(|app, _, window, cx| app.pr_handoff(window, cx))),
                    ),
            );
        if !can_link {
            view = view.child(div().text_xs().child(
                "Linking requires a selected thread in this project and server link support",
            ));
        }
        view = view.child(
            div()
                .flex()
                .gap_1()
                .child(
                    Button::new("pr-summary-tab")
                        .ghost()
                        .small()
                        .label("Activity")
                        .selected(!state.code)
                        .on_click(cx.listener(|app, _, _, cx| {
                            app.pull_requests.code = false;
                            cx.notify();
                        })),
                )
                .child(
                    Button::new("pr-code-tab")
                        .debug_selector(|| "pr-code-tab".into())
                        .ghost()
                        .small()
                        .label("Code")
                        .selected(state.code)
                        .disabled(d["capabilities"]["diff"] != true)
                        .on_click(cx.listener(|app, _, _, cx| {
                            app.pull_requests.code = true;
                            if app.pull_requests.files.is_empty() {
                                app.pr_diff(false, cx);
                            }
                            cx.notify();
                        })),
                ),
        );
        if d["capabilities"]["diff"] != true {
            view = view.child(
                div()
                    .text_xs()
                    .child("This host does not provide PR diffs. Read the code on the host."),
            );
        }
        if state.code {
            view = view.child(self.render_pr_code(cx));
        } else {
            view = view.child(
                div()
                    .id("pr-activity-scroll")
                    .flex()
                    .flex_col()
                    .gap_2()
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .child(div().text_sm().child(text(d, "body").to_owned()))
                    .child(self.render_pr_activity(cx))
                    .child(self.render_pr_candidates(cx))
                    .child(self.render_pr_merge(cx)),
            );
        }
        view.child(self.render_pr_forms(cx)).into_any_element()
    }
    fn render_pr_checks(&self, cx: &mut Context<Self>) -> AnyElement {
        let checks = self.pull_requests.detail["checks"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        Popover::new("pr-checks-menu")
            .anchor(gpui_kit::Anchor::TopRight)
            .open(self.pull_requests.checks_open)
            .trigger(
                Button::new("pr-checks")
                    .debug_selector(|| "pr-checks".into())
                    .ghost()
                    .small()
                    .label(format!("Checks ({})", checks.len())),
            )
            .on_open_change(cx.listener(|app, open: &bool, _, cx| {
                app.pull_requests.checks_open = *open;
                cx.notify();
            }))
            .content(move |_, _, cx| {
                let mut body = div()
                    .id("pr-checks-popover")
                    .debug_selector(|| "pr-checks-popover".into())
                    .flex()
                    .flex_col()
                    .gap_1()
                    .p_2()
                    .max_h_64()
                    .overflow_y_scroll()
                    .border_1()
                    .border_color(cx.theme().border)
                    .rounded_md();
                if checks.is_empty() {
                    body = body.child(div().text_xs().child("No checks reported"));
                }
                for (index, check) in checks.iter().enumerate() {
                    let url = text(check, "url").to_owned();
                    body = body.child(
                        Button::new(SharedString::from(format!("pr-check-{index}")))
                            .ghost()
                            .small()
                            .justify_start()
                            .label(format!(
                                "{} · {} {}",
                                text(check, "name"),
                                text(check, "status"),
                                text(check, "description")
                            ))
                            .disabled(
                                !url::Url::parse(&url)
                                    .is_ok_and(|u| matches!(u.scheme(), "http" | "https")),
                            )
                            .on_click(move |_, _, cx| cx.open_url(&url)),
                    );
                }
                body.into_any_element()
            })
            .into_any_element()
    }
    fn render_pr_activity(&self, cx: &mut Context<Self>) -> AnyElement {
        let activity = &self.pull_requests.activity;
        let mut view = div()
            .flex()
            .flex_col()
            .gap_2()
            .child(self.render_pr_reactions(None, cx));
        let threaded_ids: std::collections::HashSet<_> = activity["reviewThreads"]
            .as_array()
            .into_iter()
            .flatten()
            .flat_map(|thread| thread["comments"].as_array().into_iter().flatten())
            .filter_map(|comment| comment["id"].as_str())
            .collect();
        for c in activity["comments"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|comment| {
                !comment["id"]
                    .as_str()
                    .is_some_and(|id| threaded_ids.contains(id))
            })
        {
            view = view.child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(div().text_xs().child(format!(
                        "{} · {} · {}",
                        text(&c["author"], "login"),
                        text(c, "kind"),
                        text(c, "createdAt")
                    )))
                    .child(div().text_sm().child(text(c, "body").to_owned()))
                    .children(
                        c["id"]
                            .as_str()
                            .map(|id| self.render_pr_reactions(Some(id), cx)),
                    ),
            );
        }
        for thread in activity["reviewThreads"].as_array().into_iter().flatten() {
            view = view.child(self.render_pr_thread(thread, cx));
        }
        for commit in activity["commits"].as_array().into_iter().flatten() {
            view = view.child(div().text_xs().child(format!(
                "{} · {}",
                text(commit, "oid"),
                text(commit, "messageHeadline")
            )));
        }
        if activity["commentsTruncated"] == true {
            view=view.child(div().text_xs().child(format!("{} comments total; only part of the activity was returned. Open on host for the full conversation.",activity["commentCount"])));
        }
        view.into_any_element()
    }
    fn render_pr_candidates(&self, cx: &mut Context<Self>) -> AnyElement {
        let s = &self.pull_requests;
        let d = &s.detail;
        let ready = self.pr_can_mutate();
        let labels = d["capabilities"]["labels"] == true;
        let reviewers = d["capabilities"]["reviewers"]["listCandidates"] == true;
        let mut view = div()
            .flex()
            .flex_col()
            .gap_1()
            .child(div().text_xs().child(format!(
                    "Labels: {}",
                    d["labels"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .map(|l| text(l, "name"))
                        .collect::<Vec<_>>()
                        .join(", ")
                )))
            .child(div().text_xs().child(format!(
                    "Reviewers: {}",
                    d["reviewers"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .map(|r| text(r, "login"))
                        .collect::<Vec<_>>()
                        .join(", ")
                )));
        if labels {
            view = view.child(
                Button::new("pr-labels")
                    .debug_selector(|| "pr-labels".into())
                    .ghost()
                    .small()
                    .label("Change labels")
                    .disabled(!ready || d["viewerPermissions"]["labels"] == false)
                    .on_click(cx.listener(|app, _, _, cx| app.pr_candidates("labels", cx))),
            );
            if d["viewerPermissions"]["labels"] == false {
                view = view.child(div().text_xs().child("Changing labels needs triage access"));
            }
        }
        if d["capabilities"]["reviewers"]["request"] == true && !reviewers {
            view = view.child(div().text_xs().child(
                "This host does not list reviewer candidates. Request reviews on the host.",
            ));
        }
        if reviewers {
            view = view.child(
                Button::new("pr-reviewers")
                    .debug_selector(|| "pr-reviewers".into())
                    .ghost()
                    .small()
                    .label("Request reviewers")
                    .disabled(
                        !ready
                            || d["viewerPermissions"]["requestReviewers"] != true
                            || d["capabilities"]["reviewers"]["request"] != true,
                    )
                    .on_click(cx.listener(|app, _, _, cx| app.pr_candidates("reviewers", cx))),
            );
            if d["viewerPermissions"]["requestReviewers"] != true {
                view = view.child(
                    div()
                        .text_xs()
                        .child("Requesting reviewers needs write access"),
                );
            }
        }
        if let Some(kind) = s.candidate_kind {
            let allowed = if kind == "labels" {
                d["capabilities"]["labels"] == true && d["viewerPermissions"]["labels"] != false
            } else {
                d["capabilities"]["reviewers"]["request"] == true
                    && d["viewerPermissions"]["requestReviewers"] == true
            };
            let mut matched = 0;
            view = view.child(
                Button::new("pr-candidates-close")
                    .ghost()
                    .small()
                    .label("Close picker")
                    .on_click(cx.listener(|app, _, _, cx| {
                        app.pull_requests.candidate_kind = None;
                        cx.notify();
                    })),
            );
            if let Some(input) = &s.candidate_query {
                view = view.child(Input::new(input));
            }
            let query = s
                .candidate_query
                .as_ref()
                .map(|q| q.read(cx).value().to_lowercase())
                .unwrap_or_default();
            for (index, candidate) in s.candidates["candidates"]
                .as_array()
                .into_iter()
                .flatten()
                .enumerate()
                .filter(|(_, c)| {
                    format!(
                        "{} {} {}",
                        text(c, "name"),
                        text(c, "login"),
                        text(c, "description")
                    )
                    .to_lowercase()
                    .contains(&query)
                })
            {
                matched += 1;
                let candidate = candidate.clone();
                let applied = candidate[if kind == "labels" {
                    "isApplied"
                } else {
                    "isRequested"
                }] == true;
                let label = if kind == "labels" {
                    text(&candidate, "name")
                } else {
                    text(&candidate, "login")
                };
                view = view.child(
                    Button::new(SharedString::from(format!("pr-candidate-{index}")))
                        .debug_selector(move || format!("pr-candidate-{index}"))
                        .ghost()
                        .small()
                        .justify_start()
                        .label(format!("{}{}", if applied { "✓ " } else { "" }, label))
                        .disabled(!ready || !allowed)
                        .on_click(cx.listener(move |app, _, _, cx| {
                            app.pr_toggle_candidate(candidate.clone(), cx)
                        })),
                );
            }
            if matched == 0 && !s.candidates.is_null() && !query.is_empty() {
                view = view.child(div().text_xs().child("No candidates match this search"));
            }
            if s.candidates["truncated"] == true {
                view = view.child(div().text_xs().child(
                    "Only part of the candidate list was returned; use the host for the rest",
                ));
            }
            if s.candidates["candidates"]
                .as_array()
                .is_some_and(Vec::is_empty)
            {
                view = view.child(div().text_xs().child("No candidates returned by this host"));
            }
        }
        view.into_any_element()
    }
    fn render_pr_merge(&self, cx: &mut Context<Self>) -> AnyElement {
        let s = &self.pull_requests;
        let d = &s.detail;
        let reason = self.pr_merge_reason();
        let ready = self.pr_can_mutate();
        let mut view = div()
            .flex()
            .flex_col()
            .gap_1()
            .child(
                div().flex().flex_wrap().gap_1().children(
                    ["merge", "squash", "rebase"]
                        .into_iter()
                        .filter(|m| {
                            contains(&d["capabilities"], "mergeMethods", m)
                                && d["mergeCapabilities"][*m] == true
                        })
                        .map(|method| {
                            Button::new(SharedString::from(format!("pr-merge-method-{method}")))
                                .ghost()
                                .small()
                                .label(method)
                                .selected(s.merge_method == method)
                                .disabled(!ready)
                                .on_click(cx.listener(move |app, _, _, cx| {
                                    app.pull_requests.merge_method = method;
                                    app.pull_requests.merge_confirm = false;
                                    cx.notify();
                                }))
                        }),
                ),
            )
            .child(
                Button::new("pr-merge")
                    .debug_selector(|| "pr-merge".into())
                    .ghost()
                    .small()
                    .label("Merge pull request")
                    .disabled(!ready || reason.is_some())
                    .on_click(cx.listener(|app, _, _, cx| app.pr_merge(false, cx))),
            );
        if let Some(reason) = reason {
            view = view.child(div().text_xs().child(reason));
        }
        for (action, label, applicable) in [
            (
                "ready",
                "Mark ready",
                d["state"] == "open" && d["isDraft"] == true,
            ),
            (
                "draft",
                "Convert to draft",
                d["state"] == "open" && d["isDraft"] == false,
            ),
            ("close", "Close PR", d["state"] == "open"),
            ("reopen", "Reopen PR", d["state"] == "closed"),
            (
                "enable-auto-merge",
                "Enable auto-merge",
                d["state"] == "open" && d["isDraft"] == false && d["autoMergeEnabled"] == false,
            ),
            (
                "disable-auto-merge",
                "Disable auto-merge",
                d["autoMergeEnabled"] == true,
            ),
        ] {
            if applicable && contains(&d["capabilities"], "actions", action) {
                view = view.child(
                    Button::new(SharedString::from(format!("pr-action-{action}")))
                        .ghost()
                        .small()
                        .label(label)
                        .disabled(
                            !ready
                                || !contains(&d["viewerPermissions"], "actions", action)
                                || (action == "enable-auto-merge"
                                    && !self.pr_auto_merge_available()),
                        )
                        .on_click(cx.listener(move |app, _, _, cx| app.pr_action(action, cx))),
                );
                if !contains(&d["viewerPermissions"], "actions", action) {
                    view = view.child(
                        div()
                            .text_xs()
                            .child(format!("You do not have permission to {label}")),
                    );
                }
            }
        }
        if s.merge_confirm {
            view = view
                .child(
                    div()
                        .text_xs()
                        .child(format!("Merge #{} using {}?", d["number"], s.merge_method)),
                )
                .child(
                    Button::new("pr-confirm-merge")
                        .debug_selector(|| "pr-confirm-merge".into())
                        .danger()
                        .small()
                        .label("Confirm merge")
                        .disabled(!ready || reason.is_some())
                        .on_click(cx.listener(|app, _, _, cx| app.pr_merge(true, cx))),
                )
                .child(
                    Button::new("pr-cancel-merge")
                        .ghost()
                        .small()
                        .label("Cancel")
                        .on_click(cx.listener(|app, _, _, cx| {
                            app.pull_requests.merge_confirm = false;
                            cx.notify();
                        })),
                );
        }
        view.into_any_element()
    }
    fn render_pr_code(&self, cx: &mut Context<Self>) -> AnyElement {
        let s = &self.pull_requests;
        let ready = self.pr_can_mutate();
        let paths = s
            .files
            .iter()
            .map(|file| file.path.clone())
            .collect::<Vec<_>>();
        let selected = s.file_index;
        let mut view = div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .min_w_0()
            .gap_1()
            .child(
                uniform_list(
                    "pr-files",
                    paths.len(),
                    cx.processor(move |_, range: std::ops::Range<usize>, _, cx| {
                        range
                            .map(|index| {
                                Button::new(SharedString::from(format!("pr-file-{index}")))
                                    .debug_selector(move || format!("pr-file-{index}"))
                                    .ghost()
                                    .small()
                                    .justify_start()
                                    .label(paths[index].clone())
                                    .selected(selected == index)
                                    .on_click(cx.listener(move |app, _, _, cx| {
                                        app.pull_requests.file_index = index;
                                        app.pull_requests.anchor = None;
                                        cx.notify();
                                    }))
                                    .into_any_element()
                            })
                            .collect()
                    }),
                )
                .h(px(96.))
                .flex_shrink_0(),
            );
        if s.diff_truncated {
            view = view.child(
                div()
                    .text_xs()
                    .child("The host withheld some diff content (binary or oversized changes)"),
            );
        }
        if s.diff_cursor.is_some() {
            view = view.child(
                Button::new("pr-diff-more")
                    .ghost()
                    .small()
                    .label("Load more changed files")
                    .disabled(!ready || s.loading.contains("diff") || s.diff_limit)
                    .on_click(cx.listener(|app, _, _, cx| app.pr_diff(true, cx))),
            );
        }
        if s.errors.contains_key("diff") {
            view = view.child(
                Button::new("pr-diff-retry")
                    .ghost()
                    .small()
                    .label("Retry diff")
                    .disabled(!ready || s.loading.contains("diff"))
                    .on_click(cx.listener(|app, _, _, cx| app.pr_diff(false, cx))),
            );
        }
        if let Some(file) = s.files.get(s.file_index) {
            if s.detail["capabilities"]["viewedFiles"].is_string() {
                let viewed = s.viewed["files"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .any(|f| f["path"] == file.path && f["state"] == "viewed");
                let path = file.path.clone();
                view = view.child(
                    Button::new("pr-file-viewed")
                        .debug_selector(|| "pr-file-viewed".into())
                        .ghost()
                        .small()
                        .label(if viewed {
                            "✓ Viewed (mark unviewed)"
                        } else {
                            "Mark viewed"
                        })
                        .disabled(!ready || s.viewed.is_null())
                        .on_click(cx.listener(move |app, _, _, cx| {
                            app.pr_viewed(path.clone(), !viewed, cx)
                        })),
                );
                if s.detail["capabilities"]["viewedFiles"] == "environment" {
                    view = view.child(
                        div()
                            .text_xs()
                            .child("Viewed marks are stored in this environment, not on the host"),
                    );
                }
                if s.viewed["truncated"] == true {
                    view = view.child(
                        div()
                            .text_xs()
                            .child("The host returned only part of your viewed-file marks"),
                    );
                }
            }
            let notes = s.activity["reviewThreads"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|t| t["path"] == file.path)
                .collect::<Vec<_>>();
            if !notes.is_empty() {
                view = view.child(
                    div()
                        .id("pr-file-comments")
                        .flex()
                        .flex_col()
                        .gap_1()
                        .max_h_64()
                        .overflow_y_scroll()
                        .children(notes.into_iter().map(|t| self.render_pr_thread(t, cx))),
                );
            }
            let lines = file.lines.clone();
            let path = file.path.clone();
            let old_path = file.old_path.clone();
            let inline = ready
                && s.detail["capabilities"]["review"]["inlineComment"] == true
                && s.detail["viewerPermissions"]["comment"] == true;
            let mono = cx.theme().mono_font_family.clone();
            view = view.child(
                uniform_list(
                    "pr-patch-lines",
                    lines.len(),
                    cx.processor(move |_, range: std::ops::Range<usize>, _, cx| {
                        range
                            .map(|index| {
                                let line = &lines[index];
                                let mut anchor = json!({"path":path,"position":line.position.as_ref().map(super::model::Position::value)});
                                if !old_path.is_empty() && old_path != path {
                                    anchor["oldPath"] = json!(old_path);
                                }
                                Button::new(SharedString::from(format!("pr-line-{index}")))
                                    .debug_selector(move || format!("pr-line-{index}"))
                                    .ghost()
                                    .small()
                                    .justify_start()
                                    .label(line.text.clone())
                                    .disabled(!inline || line.position.is_none())
                                    .on_click(cx.listener(move |app, _, window, cx| {
                                        app.pull_requests.anchor = Some(anchor.clone());
                                        if let Some(input) = &app.pull_requests.inline_body {
                                            input.update(cx, |input, cx| input.focus(window, cx));
                                        }
                                        cx.notify();
                                    }))
                                    .into_any_element()
                            })
                            .collect()
                    }),
                )
                .font_family(mono)
                .text_xs()
                .flex_1()
                .min_h_0()
                .min_w_0(),
            );
        }
        if let Some(anchor) = &s.anchor {
            view = view
                .child(div().text_xs().child(format!(
                    "Pending comment on {} · {}",
                    text(anchor, "path"),
                    anchor["position"]
                )))
                .children(
                    s.inline_body
                        .as_ref()
                        .map(|input| Textarea::new(input).disabled(!ready)),
                )
                .child(
                    Button::new("pr-inline-add")
                        .debug_selector(|| "pr-inline-add".into())
                        .ghost()
                        .small()
                        .label("Add to pending review")
                        .disabled(
                            !ready
                                || s.inline_body.as_ref().is_none_or(|input| {
                                    let body = input.read(cx).value();
                                    body.trim().is_empty() || body.encode_utf16().count() > 65536
                                }),
                        )
                        .on_click(cx.listener(|app, _, window, cx| app.pr_add_inline(window, cx))),
                )
                .child(
                    Button::new("pr-inline-cancel")
                        .ghost()
                        .small()
                        .label("Cancel line comment")
                        .on_click(cx.listener(|app, _, _, cx| {
                            app.pull_requests.anchor = None;
                            cx.notify();
                        })),
                );
        }
        view.into_any_element()
    }
    fn render_pr_forms(&self, cx: &mut Context<Self>) -> AnyElement {
        let s = &self.pull_requests;
        let d = &s.detail;
        let ready = self.pr_can_mutate();
        let Some(draft) = s.draft() else {
            return div().into_any_element();
        };
        let mut view = div()
            .flex()
            .flex_col()
            .gap_1()
            .flex_shrink_0()
            .max_h(px(340.))
            .id("pr-forms")
            .overflow_y_scroll();
        if d["capabilities"]["comment"] == true && d["viewerPermissions"]["comment"] == true {
            let body = draft.comment.read(cx).value();
            view = view
                .child(Textarea::new(&draft.comment).disabled(!ready))
                .child(
                    Button::new("pr-comment-submit")
                        .debug_selector(|| "pr-comment-submit".into())
                        .ghost()
                        .small()
                        .label("Post comment")
                        .disabled(
                            !ready || body.trim().is_empty() || body.encode_utf16().count() > 65536,
                        )
                        .on_click(cx.listener(|app, _, window, cx| app.pr_comment(window, cx))),
                );
        }
        let verdicts = [
            ("comment", "Comment"),
            ("approve", "Approve"),
            ("request-changes", "Request changes"),
        ]
        .into_iter()
        .filter(|(v, _)| {
            contains(&d["capabilities"]["review"], "verdicts", v)
                && contains(&d["viewerPermissions"], "verdicts", v)
        })
        .collect::<Vec<_>>();
        if !verdicts.is_empty() {
            view = view
                .child(Textarea::new(&draft.review).disabled(!ready))
                .child(
                    div()
                        .flex()
                        .flex_wrap()
                        .gap_1()
                        .children(verdicts.into_iter().map(|(verdict, label)| {
                            Button::new(SharedString::from(format!("pr-verdict-{verdict}")))
                                .debug_selector(move || format!("pr-verdict-{verdict}"))
                                .ghost()
                                .small()
                                .label(label)
                                .selected(draft.verdict == verdict)
                                .disabled(!ready)
                                .on_click(cx.listener(move |app, _, _, cx| {
                                    if let Some(key) = app.pull_requests.key()
                                        && let Some(draft) = app.pull_requests.drafts.get_mut(&key)
                                    {
                                        draft.verdict = verdict;
                                    }
                                    cx.notify();
                                }))
                        })),
                );
            for (index, comment) in draft.comments.iter().enumerate() {
                view = view.child(
                    Button::new(SharedString::from(format!("pr-remove-inline-{index}")))
                        .ghost()
                        .small()
                        .justify_start()
                        .label(format!(
                            "Remove {}: {}",
                            text(comment, "path"),
                            text(comment, "body")
                        ))
                        .disabled(!ready)
                        .on_click(cx.listener(move |app, _, _, cx| {
                            if let Some(key) = app.pull_requests.key()
                                && let Some(draft) = app.pull_requests.drafts.get_mut(&key)
                                && index < draft.comments.len()
                            {
                                draft.comments.remove(index);
                            }
                            cx.notify();
                        })),
                );
            }
            view = view.child(
                Button::new("pr-review-submit")
                    .debug_selector(|| "pr-review-submit".into())
                    .primary()
                    .small()
                    .label(format!(
                        "Submit review ({} line comments)",
                        draft.comments.len()
                    ))
                    .disabled(
                        !ready
                            || !self.pr_review_allowed(
                                draft.verdict,
                                draft.review.read(cx).value().as_str(),
                                &draft.comments,
                            ),
                    )
                    .on_click(cx.listener(|app, _, window, cx| app.pr_review(window, cx))),
            );
        }
        view = view.child(
            Button::new("pr-discard-draft")
                .ghost()
                .small()
                .label("Discard PR drafts…")
                .disabled(!ready)
                .on_click(cx.listener(|app, _, _, cx| app.pr_discard(false, cx))),
        );
        if s.discard_confirm {
            view = view
                .child(
                    Button::new("pr-confirm-discard")
                        .danger()
                        .small()
                        .label("Discard unsent comment and review")
                        .disabled(!ready)
                        .on_click(cx.listener(|app, _, _, cx| app.pr_discard(true, cx))),
                )
                .child(
                    Button::new("pr-cancel-discard")
                        .ghost()
                        .small()
                        .label("Keep drafts")
                        .on_click(cx.listener(|app, _, _, cx| {
                            app.pull_requests.discard_confirm = false;
                            cx.notify();
                        })),
                );
        }
        view.into_any_element()
    }
}
