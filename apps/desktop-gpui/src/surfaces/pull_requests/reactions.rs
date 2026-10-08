//! Host-owned reaction counts; no optimistic status survives a denied write.
use super::{NativeApp, model::text};
use gpui_kit::base::{Disableable as _, Selectable as _};
use gpui_kit::component::{
    Sizable,
    button::{Button, ButtonVariants as _},
};
use gpui_kit::{AnyElement, Context, IntoElement, SharedString, div, prelude::*};
use serde_json::{Value, json};

const REACTIONS: [(&str, &str); 8] = [
    ("thumbs-up", "👍"),
    ("thumbs-down", "👎"),
    ("laugh", "😄"),
    ("hooray", "🎉"),
    ("confused", "😕"),
    ("heart", "❤️"),
    ("rocket", "🚀"),
    ("eyes", "👀"),
];
impl NativeApp {
    fn pr_reactions(&self, subject: Option<&str>) -> Option<&Value> {
        let activity = &self.pull_requests.activity;
        let Some(id) = subject else {
            return (!activity.is_null()).then_some(&activity["reactions"]);
        };
        activity["comments"]
            .as_array()
            .into_iter()
            .flatten()
            .chain(
                activity["reviewThreads"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .flat_map(|t| t["comments"].as_array().into_iter().flatten()),
            )
            .find(|c| text(c, "id") == id)
            .map(|c| &c["reactions"])
    }
    fn pr_reaction_ready(&self, subject: Option<&str>) -> bool {
        self.pr_can_mutate()
            && !self.pull_requests.loading.contains("activity")
            && !self.pull_requests.errors.contains_key("activity")
            && self.pull_requests.detail["capabilities"]["reactions"] == true
            && self.pull_requests.detail["viewerPermissions"]["comment"] == true
            && self.pr_reactions(subject).is_some()
    }
    pub(crate) fn pr_react(
        &mut self,
        subject: Option<String>,
        content: &'static str,
        cx: &mut Context<Self>,
    ) {
        if !self.pr_reaction_ready(subject.as_deref())
            || !REACTIONS.iter().any(|(c, _)| *c == content)
        {
            return;
        }
        let reacted = !self
            .pr_reactions(subject.as_deref())
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .any(|r| r["content"] == content && r["viewerHasReacted"] == true);
        let mut payload = json!({"content":content,"reacted":reacted});
        if let Some(subject) = subject {
            payload["subjectId"] = json!(subject);
        }
        self.pull_requests.reacting = None;
        self.pr_mutate("pullRequests.setReaction", payload, None, cx);
    }
    pub(super) fn render_pr_reactions(
        &self,
        subject: Option<&str>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let ready = self.pr_reaction_ready(subject);
        let subject = subject.map(str::to_owned);
        let key = subject
            .as_deref()
            .map_or_else(|| "description".to_owned(), |id| format!("comment-{id}"));
        let reactions = self
            .pr_reactions(subject.as_deref())
            .and_then(Value::as_array);
        let expanded = self.pull_requests.reacting.as_ref() == Some(&subject);
        let mut view = div().flex().flex_wrap().gap_1();
        for (content, emoji) in REACTIONS {
            let reaction = reactions.and_then(|rs| rs.iter().find(|r| r["content"] == content));
            if reaction.is_none() && !expanded {
                continue;
            }
            let count = reaction.and_then(|r| r["count"].as_u64()).unwrap_or(0);
            let selected = reaction.is_some_and(|r| r["viewerHasReacted"] == true);
            let actors = reaction
                .and_then(|r| r["actors"].as_array())
                .map(|xs| {
                    xs.iter()
                        .filter_map(Value::as_str)
                        .collect::<Vec<_>>()
                        .join(", ")
                })
                .unwrap_or_default();
            let target = subject.clone();
            let selector = format!("pr-reaction-{key}-{content}");
            view = view.child(
                Button::new(SharedString::from(selector.clone()))
                    .debug_selector(move || selector.clone())
                    .label(if count == 0 {
                        emoji.to_owned()
                    } else {
                        format!("{emoji} {count}")
                    })
                    .tooltip(format!(
                        "{} {content} · {actors}",
                        if selected {
                            "You reacted · remove"
                        } else {
                            "React"
                        }
                    ))
                    .ghost()
                    .small()
                    .selected(selected)
                    .disabled(!ready)
                    .on_click(
                        cx.listener(move |app, _, _, cx| app.pr_react(target.clone(), content, cx)),
                    ),
            );
        }
        if ready || expanded {
            let selector = format!("pr-reactions-{key}");
            view = view.child(
                Button::new(SharedString::from(selector.clone()))
                    .debug_selector(move || selector.clone())
                    .label(if expanded { "Close reactions" } else { "React" })
                    .ghost()
                    .small()
                    .on_click(cx.listener(move |app, _, _, cx| {
                        app.pull_requests.reacting = if expanded {
                            None
                        } else {
                            Some(subject.clone())
                        };
                        cx.notify();
                    })),
            );
        }
        view.into_any_element()
    }
}
