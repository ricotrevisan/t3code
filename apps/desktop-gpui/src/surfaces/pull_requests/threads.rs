//! Review conversations keep drafts on the captured PR; host receipts own all status changes.
use super::{NativeApp, ReplyDraft, Submitted, model::text};
use gpui_kit::base::Disableable as _;
use gpui_kit::component::{
    ActiveTheme as _, Sizable,
    button::{Button, ButtonVariants as _},
    input::{Textarea, TextareaState},
};
use gpui_kit::{AnyElement, AppContext as _, Context, IntoElement, Window, div, prelude::*};
use serde_json::{Value, json};
use t3code_desktop_gpui::connection;

impl NativeApp {
    fn pr_thread(&self, id: &str) -> Option<&Value> {
        self.pull_requests.activity["reviewThreads"]
            .as_array()?
            .iter()
            .find(|t| text(t, "id") == id)
    }
    fn pr_thread_ready(&self, id: &str) -> bool {
        self.pr_can_mutate()
            && !self.pull_requests.loading.contains("activity")
            && !self.pull_requests.errors.contains_key("activity")
            && self.pr_thread(id).is_some()
    }
    pub(crate) fn pr_begin_reply(
        &mut self,
        id: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.pr_thread_ready(&id)
            || self.pull_requests.detail["capabilities"]["review"]["reply"] != true
            || self.pull_requests.detail["viewerPermissions"]["comment"] != true
        {
            return;
        }
        let Some(key) = self.pull_requests.key() else {
            return;
        };
        let Some(draft) = self.pull_requests.drafts.get_mut(&key) else {
            return;
        };
        if !draft.replies.contains_key(&id) {
            // Keep empty editors bounded too; never evict unsent replies.
            draft
                .replies
                .retain(|_, reply| !reply.input.read(cx).value().is_empty());
            if draft.replies.len() >= 64 {
                self.pull_requests.errors.insert("mutation", "64 unsent thread replies are retained. Send or clear one before starting another.".into());
                cx.notify();
                return;
            }
            let input = cx
                .new(|cx| TextareaState::new(window, cx).placeholder("Reply to this conversation"));
            let subscription = cx.observe(&input, |_, _, cx| cx.notify());
            draft.replies.insert(
                id.clone(),
                ReplyDraft {
                    input,
                    _subscription: subscription,
                },
            );
        }
        draft.replies[&id]
            .input
            .update(cx, |input, cx| input.focus(window, cx));
        self.pull_requests.replying = Some(id);
        cx.notify();
    }
    pub(crate) fn pr_reply(&mut self, id: String, window: &mut Window, cx: &mut Context<Self>) {
        if !self.pr_thread_ready(&id)
            || self.pull_requests.detail["capabilities"]["review"]["reply"] != true
            || self.pull_requests.detail["viewerPermissions"]["comment"] != true
        {
            return;
        }
        let Some(input) = self.pull_requests.draft().and_then(|d| d.replies.get(&id)) else {
            return;
        };
        let body = input.input.read(cx).value().to_string();
        if body.trim().is_empty() || body.encode_utf16().count() > 65536 {
            return;
        }
        self.pr_mutate(
            "pullRequests.replyToThread",
            json!({"threadId":id,"body":body}),
            Some(Submitted {
                key: self.pull_requests.key().unwrap_or_default(),
                body,
                comments: vec![],
                review: false,
                reply_thread: Some(id),
                window: window.window_handle(),
            }),
            cx,
        );
    }
    pub(crate) fn pr_resolve(&mut self, id: String, resolved: bool, cx: &mut Context<Self>) {
        if !self.pr_thread_ready(&id)
            || self.pull_requests.detail["capabilities"]["review"]["resolve"] != true
            || self.pull_requests.detail["viewerPermissions"]["resolve"] != true
            || self
                .pr_thread(&id)
                .is_none_or(|t| t["isResolved"] == resolved)
        {
            return;
        }
        self.pr_mutate(
            "pullRequests.setThreadResolution",
            json!({"threadId":id,"resolved":resolved}),
            None,
            cx,
        );
    }
    pub(crate) fn pr_thread_more(&mut self, id: String, cx: &mut Context<Self>) {
        if !self.pr_thread_ready(&id) || self.pull_requests.loading.contains("thread-comments") {
            return;
        }
        let Some(cursor) = self
            .pr_thread(&id)
            .and_then(|t| t["nextCommentsCursor"].as_str())
            .map(str::to_owned)
        else {
            return;
        };
        let Some(mut payload) = self.pull_requests.reference.clone() else {
            return;
        };
        let Some(sender) = self.commands.clone() else {
            return;
        };
        let Some(owner) = self.pr_owner("thread-comments", cx) else {
            return;
        };
        // Refresh invalidates pagination even when the PR identity did not change.
        let activity_serial = self.pull_requests.requests.get("activity").copied();
        payload["threadId"] = json!(id);
        payload["cursor"] = json!(cursor);
        cx.spawn(async move |entity, cx| {
            let result = connection::call(&sender, "pullRequests.threadComments", payload).await;
            let _ = entity.update(cx, |app, cx| {
                if !owner.current(app, "thread-comments") { return; }
                app.pull_requests.loading.remove("thread-comments");
                if app.pull_requests.requests.get("activity").copied() != activity_serial { cx.notify(); return; }
                let result = result.and_then(|page| {
                    let comments = page["comments"].as_array().ok_or("Invalid thread comments response")?;
                    if !(page["nextCursor"].is_null() || page["nextCursor"].as_str().is_some_and(|s| !s.trim().is_empty())) {
                        return Err("Invalid thread comment cursor".into());
                    }
                    if page["nextCursor"].as_str() == Some(&cursor) {
                        return Err("The host repeated the comment cursor; refresh or read on the host.".into());
                    }
                    let Some(thread) = app.pull_requests.activity["reviewThreads"].as_array_mut().and_then(|ts| ts.iter_mut().find(|t| text(t,"id") == id)) else { return Ok(()); };
                    if thread["nextCommentsCursor"].as_str() != Some(&cursor) { return Ok(()); }
                    let existing = thread["comments"].as_array_mut().ok_or("Invalid review conversation")?;
                    if existing.len().saturating_add(comments.len()) > 2000 {
                        return Err("Native conversation display is limited to 2,000 comments. Read the rest on the host.".into());
                    }
                    let mut bytes = existing.iter().map(|c| text(c, "body").len()).sum::<usize>();
                    for comment in comments {
                        if comment["id"].as_str().is_none_or(|s| s.trim().is_empty()) || !comment["body"].is_string() { return Err("Invalid thread comment".into()); }
                        bytes = bytes.saturating_add(text(comment, "body").len());
                    }
                    if bytes > 4 * 1024 * 1024 {
                        return Err("Native conversation display is limited to 4 MiB. Read the rest on the host.".into());
                    }
                    for comment in comments {
                        if !existing.iter().any(|c| c["id"] == comment["id"]) { existing.push(comment.clone()); }
                    }
                    thread["nextCommentsCursor"] = page["nextCursor"].clone();
                    Ok(())
                });
                if let Err(error) = result { app.pull_requests.errors.insert("thread-comments", error); }
                cx.notify();
            });
        }).detach();
    }
    pub(super) fn render_pr_thread(&self, thread: &Value, cx: &mut Context<Self>) -> AnyElement {
        let id = text(thread, "id").to_owned();
        let ready = self.pr_thread_ready(&id);
        let resolved = thread["isResolved"] == true;
        let mut view = div()
            .flex()
            .flex_col()
            .gap_1()
            .child(div().text_xs().child(format!(
                "{}:{}{} · {}",
                text(thread, "path"),
                thread["line"],
                if thread["isOutdated"] == true {
                    " (outdated location)"
                } else {
                    ""
                },
                if resolved { "resolved" } else { "unresolved" }
            )));
        for comment in thread["comments"].as_array().into_iter().flatten() {
            view = view
                .child(div().text_sm().child(format!(
                    "{}: {}",
                    text(&comment["author"], "login"),
                    text(comment, "body")
                )))
                .children(
                    comment["id"]
                        .as_str()
                        .map(|id| self.render_pr_reactions(Some(id), cx)),
                );
        }
        let mut controls = div().flex().flex_wrap().gap_1();
        if thread["nextCommentsCursor"].is_string() {
            let target = id.clone();
            let selector = format!("pr-thread-more-{id}");
            controls = controls.child(
                Button::new(gpui_kit::SharedString::from(format!("pr-thread-more-{id}")))
                    .debug_selector(move || selector.clone())
                    .label("Load more comments")
                    .ghost()
                    .small()
                    .disabled(!ready || self.pull_requests.loading.contains("thread-comments"))
                    .on_click(
                        cx.listener(move |app, _, _, cx| app.pr_thread_more(target.clone(), cx)),
                    ),
            );
        }
        if self.pull_requests.detail["capabilities"]["review"]["resolve"] == true {
            let target = id.clone();
            let selector = format!("pr-thread-resolve-{id}");
            controls = controls.child(
                Button::new(gpui_kit::SharedString::from(format!(
                    "pr-thread-resolve-{id}"
                )))
                .debug_selector(move || selector.clone())
                .label(if resolved { "Unresolve" } else { "Resolve" })
                .ghost()
                .small()
                .disabled(
                    !ready || self.pull_requests.detail["viewerPermissions"]["resolve"] != true,
                )
                .on_click(
                    cx.listener(move |app, _, _, cx| app.pr_resolve(target.clone(), !resolved, cx)),
                ),
            );
        }
        if self.pull_requests.detail["capabilities"]["review"]["reply"] == true {
            let target = id.clone();
            let selector = format!("pr-thread-reply-{id}");
            controls = controls.child(
                Button::new(gpui_kit::SharedString::from(format!(
                    "pr-thread-reply-{id}"
                )))
                .debug_selector(move || selector.clone())
                .label("Reply")
                .ghost()
                .small()
                .disabled(
                    !ready || self.pull_requests.detail["viewerPermissions"]["comment"] != true,
                )
                .on_click(cx.listener(move |app, _, window, cx| {
                    app.pr_begin_reply(target.clone(), window, cx)
                })),
            );
        }
        view = view.child(controls);
        if self.pull_requests.replying.as_deref() == Some(&id)
            && let Some(input) = self.pull_requests.draft().and_then(|d| d.replies.get(&id))
        {
            let target = id.clone();
            let selector = format!("pr-thread-send-{id}");
            let body = input.input.read(cx).value();
            view = view.child(Textarea::new(&input.input).small()).child(
                div()
                    .flex()
                    .gap_1()
                    .child(
                        Button::new(gpui_kit::SharedString::from(format!("pr-thread-send-{id}")))
                            .debug_selector(move || selector.clone())
                            .label("Send reply")
                            .ghost()
                            .small()
                            .disabled(
                                !ready
                                    || self.pull_requests.detail["viewerPermissions"]["comment"]
                                        != true
                                    || body.trim().is_empty()
                                    || body.encode_utf16().count() > 65536,
                            )
                            .on_click(cx.listener(move |app, _, window, cx| {
                                app.pr_reply(target.clone(), window, cx)
                            })),
                    )
                    .child(
                        Button::new(gpui_kit::SharedString::from(format!(
                            "pr-thread-cancel-{id}"
                        )))
                        .debug_selector(move || format!("pr-thread-cancel-{id}"))
                        .label("Cancel")
                        .tooltip("Close editor; reply draft is retained")
                        .ghost()
                        .small()
                        .on_click(cx.listener(|app, _, _, cx| {
                            app.pull_requests.replying = None;
                            cx.notify();
                        })),
                    ),
            );
        }
        view.text_color(cx.theme().foreground).into_any_element()
    }
}
