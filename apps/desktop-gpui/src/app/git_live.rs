//! Checkout-owned local/remote status projection. No remote host editor execution.
use crate::NativeApp;
use gpui_kit::Context;
use serde_json::{Value, json};
use t3code_desktop_gpui::connection;

type Checkout = (Option<String>, Option<String>, Option<String>);
#[derive(Default)]
pub(crate) struct LiveGit {
    owner: Option<(u64, Checkout, String)>,
    pub(crate) status: Option<Value>,
    pub(crate) error: Option<String>,
}
impl NativeApp {
    /// Call after synchronization and checkout selection changes; the old stream is
    /// explicitly cancelled and late events cannot repopulate the new checkout.
    pub(crate) fn sync_git_status(&mut self, cx: &mut Context<Self>) {
        let context = self.checkout_context();
        let ready = self.connection_ready() && self.selected_workspace().is_some();
        if ready
            && self
                .git_live
                .owner
                .as_ref()
                .is_some_and(|(g, c, _)| *g == self.generation && *c == context)
        {
            return;
        }
        if let Some((generation, _, key)) = self.git_live.owner.take() {
            // A disconnected/retired transport owns no stream in the new session.
            // Cancel its pending enqueue, without reporting a new-session error.
            self.subscription_enqueues.remove(&key);
            if generation == self.generation && self.commands.is_some() {
                self.enqueue_subscription(connection::Command::Unsubscribe(key), cx);
            }
        }
        self.git_live.status = None;
        self.git_live.error = None;
        if ready && let Some(cwd) = self.selected_workspace() {
            let key = format!("header-vcs:{}", uuid::Uuid::new_v4());
            self.git_live.owner = Some((self.generation, context, key.clone()));
            self.enqueue_subscription(
                connection::Command::Subscribe {
                    key,
                    tag: "subscribeVcsStatus".into(),
                    payload: json!({"cwd":cwd}),
                },
                cx,
            );
        }
    }
    pub(crate) fn live_git_error(&self) -> Option<&str> {
        self.git_live
            .owner
            .as_ref()
            .filter(|(g, c, _)| *g == self.generation && *c == self.checkout_context())?;
        self.git_live.error.as_deref()
    }
    pub(crate) fn live_git_status(&self) -> Option<&Value> {
        self.git_live
            .owner
            .as_ref()
            .filter(|(g, c, _)| *g == self.generation && *c == self.checkout_context())?;
        self.git_live.status.as_ref()
    }
    pub(crate) fn apply_git_status(
        &mut self,
        key: &str,
        item: Result<Value, String>,
        cx: &mut Context<Self>,
    ) {
        if !self.git_live.owner.as_ref().is_some_and(|(g, c, k)| {
            *g == self.generation && *c == self.checkout_context() && k == key
        }) {
            return;
        }
        match item {
            Err(error) => {
                self.git_live.error = Some(error);
                self.git_live.status = None;
                self.git_header.status = None;
            }
            Ok(event) => {
                let kind = event["_tag"].as_str();
                if !matches!(kind, Some("snapshot" | "localUpdated" | "remoteUpdated")) {
                    return;
                }
                let status = self.git_live.status.get_or_insert_with(|| json!({}));
                if kind == Some("snapshot") {
                    *status = json!({});
                }
                if let Some(local) = event["local"].as_object() {
                    for (k, v) in local {
                        status[k] = v.clone();
                    }
                }
                if matches!(kind, Some("snapshot" | "remoteUpdated")) {
                    for k in [
                        "hasUpstream",
                        "aheadCount",
                        "behindCount",
                        "aheadOfDefaultCount",
                        "pr",
                    ] {
                        status.as_object_mut().unwrap().remove(k);
                    }
                    if let Some(remote) = event["remote"].as_object() {
                        for (k, v) in remote {
                            status[k] = v.clone();
                        }
                    }
                }
                self.git_live.error = None;
                // A commit form keeps the user's file choices; only status/applicability updates.
                self.git_header.status = Some(status.clone());
                self.git_header.context = Some(self.checkout_context());
            }
        }
        cx.notify();
    }
}
