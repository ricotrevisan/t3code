//! Searchable comparison refs belong to this page and captured checkout, not the branch-switch picker.
use crate::NativeApp;
use gpui_kit::component::{
    Disableable, Selectable, Sizable,
    button::{Button, ButtonVariants},
    input::{Input, InputEvent, InputState},
};
use gpui_kit::{
    AppContext as _, Context, Entity, FocusHandle, Subscription, Task, Window, div, prelude::*,
    rems, uniform_list,
};
use serde::Deserialize;
use serde_json::json;
use std::rc::Rc;
use t3code_desktop_gpui::connection;

#[derive(Default)]
pub(crate) struct Comparison {
    open: bool,
    input: Option<Entity<InputState>>,
    subscription: Option<Subscription>,
    previous_focus: Option<FocusHandle>,
    rows: Rc<Vec<Choice>>,
    loading: bool,
    error: Option<String>,
    next: Option<u64>,
    epoch: u64,
    task: Option<Task<()>>,
}
#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Choice {
    name: String,
    #[serde(default)]
    is_remote: Option<bool>,
}
impl NativeApp {
    pub(crate) fn show_diff_comparison(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.diffs.comparison.open {
            self.close_diff_comparison(window, cx);
            return;
        }
        let input =
            cx.new(|cx| InputState::new(window, cx).placeholder("Search local and remote refs…"));
        let subscription = cx.subscribe(&input, |app, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                app.query_diff_comparison(false, cx);
            }
        });
        let picker = &mut self.diffs.comparison;
        picker.previous_focus = window.focused(cx);
        picker.open = true;
        picker.input = Some(input.clone());
        picker.subscription = Some(subscription);
        input.update(cx, |input, cx| input.focus(window, cx));
        self.query_diff_comparison(false, cx);
    }
    fn close_diff_comparison(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let picker = &mut self.diffs.comparison;
        picker.open = false;
        picker.epoch = picker.epoch.wrapping_add(1);
        picker.task = None;
        picker.subscription = None;
        if let Some(focus) = picker.previous_focus.take() {
            window.focus(&focus, cx);
        }
        cx.notify();
    }
    pub(crate) fn query_diff_comparison(&mut self, more: bool, cx: &mut Context<Self>) {
        let (Some(cwd), Some(sender)) = (self.selected_workspace(), self.commands.clone()) else {
            return;
        };
        if !self.connection_ready() || !self.diffs.comparison.open {
            return;
        }
        let picker = &mut self.diffs.comparison;
        if more && (picker.loading || picker.next.is_none()) {
            return;
        }
        let query = picker
            .input
            .as_ref()
            .map(|input| input.read(cx).value().trim().to_owned())
            .unwrap_or_default();
        let cursor = more.then_some(picker.next).flatten();
        picker.epoch = picker.epoch.wrapping_add(1);
        let epoch = picker.epoch;
        picker.loading = true;
        picker.error = None;
        if !more {
            picker.rows = Rc::new(Vec::new());
            picker.next = None;
        }
        let generation = self.generation;
        let diff_epoch = self.diffs.epoch;
        let context = self.checkout_context();
        let mut payload =
            json!({"cwd":cwd,"refKind":"all","includeMatchingRemoteRefs":true,"limit":100});
        if !query.is_empty() {
            payload["query"] = query.into();
        }
        if let Some(cursor) = cursor {
            payload["cursor"] = cursor.into();
        }
        self.diffs.comparison.task = Some(cx.spawn(async move |entity, cx| {
            let result = connection::call(&sender, "vcs.listRefs", payload).await;
            let _ = entity.update(cx, |app, cx| {
                if app.generation != generation
                    || app.checkout_context() != context
                    || app.diffs.epoch != diff_epoch
                    || app.diffs.comparison.epoch != epoch
                    || !app.diffs.comparison.open
                {
                    return;
                }
                let head = app.diffs.source_head.clone();
                let picker = &mut app.diffs.comparison;
                picker.loading = false;
                match result.and_then(|value| {
                    let rows: Vec<Choice> = serde_json::from_value(value["refs"].clone())
                        .map_err(|_| "Invalid comparison refs response".to_owned())?;
                    Ok((rows, value["nextCursor"].as_u64()))
                }) {
                    Ok((rows, next)) => {
                        let mut combined = if more {
                            picker.rows.as_ref().clone()
                        } else {
                            Vec::new()
                        };
                        for row in rows {
                            if Some(&row.name) != head.as_ref()
                                && !combined.iter().any(|old| {
                                    old.name == row.name && old.is_remote == row.is_remote
                                })
                            {
                                combined.push(row);
                            }
                        }
                        picker.rows = Rc::new(combined);
                        picker.next = next;
                    }
                    Err(error) => picker.error = Some(error),
                }
                cx.notify();
            });
        }));
        cx.notify();
    }
    pub(crate) fn render_diff_comparison(
        &self,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let picker = &self.diffs.comparison;
        let rows = picker.rows.clone();
        let entity = cx.entity().downgrade();
        let selected = self.diffs.base_ref.clone();
        div()
            .flex()
            .flex_col()
            .min_w_0()
            .gap_1()
            .capture_key_down(
                cx.listener(|app, event: &gpui_kit::KeyDownEvent, window, cx| {
                    if app.diffs.comparison.open && event.keystroke.key == "escape" {
                        app.close_diff_comparison(window, cx);
                        cx.stop_propagation();
                    }
                }),
            )
            .when(picker.open, |view| {
                view.when_some(picker.input.as_ref(), |view, input| {
                    view.child(
                        div()
                            .debug_selector(|| "diff-ref-search".into())
                            .child(Input::new(input).small()),
                    )
                })
                .child(
                    Button::new("diff-ref-close")
                        .ghost()
                        .small()
                        .label("Close comparison picker")
                        .on_click(
                            cx.listener(|app, _, window, cx| app.close_diff_comparison(window, cx)),
                        ),
                )
                .child(
                    Button::new("diff-ref-automatic")
                        .debug_selector(|| "diff-ref-automatic".into())
                        .ghost()
                        .small()
                        .label("Automatic base")
                        .selected(selected.is_none())
                        .on_click(cx.listener(|app, _, window, cx| {
                            app.close_diff_comparison(window, cx);
                            app.diffs.base_ref = None;
                            app.view_git_diff("branch-range", cx);
                        })),
                )
                .when(picker.loading, |view| view.child("Loading refs…"))
                .when_some(picker.error.clone(), |view, error| {
                    view.child(error).child(
                        Button::new("diff-ref-retry")
                            .ghost()
                            .small()
                            .label("Retry refs")
                            .on_click(
                                cx.listener(|app, _, _, cx| app.query_diff_comparison(false, cx)),
                            ),
                    )
                })
                .when(
                    !picker.loading && picker.error.is_none() && rows.is_empty(),
                    |view| view.child("No matching refs"),
                )
                .child(
                    uniform_list("diff-ref-rows", rows.len(), move |range, _, _| {
                        range
                            .map(|index| {
                                let row = rows[index].clone();
                                let entity = entity.clone();
                                Button::new(("diff-ref-row", index))
                                    .debug_selector(move || format!("diff-ref-row-{index}"))
                                    .ghost()
                                    .small()
                                    .justify_start()
                                    .selected(selected.as_ref() == Some(&row.name))
                                    .label(format!(
                                        "{} · {}",
                                        row.name,
                                        match row.is_remote {
                                            Some(true) => "Remote",
                                            Some(false) => "Local",
                                            None => "Ref",
                                        }
                                    ))
                                    .tooltip(row.name.clone())
                                    .on_click(move |_, window, cx| {
                                        let _ = entity.update(cx, |app, cx| {
                                            app.close_diff_comparison(window, cx);
                                            app.diffs.base_ref = Some(row.name.clone());
                                            app.view_git_diff("branch-range", cx);
                                        });
                                    })
                            })
                            .collect()
                    })
                    .h(rems(6.))
                    .w_full(),
                )
                .when(picker.next.is_some(), |view| {
                    view.child(
                        Button::new("diff-ref-more")
                            .ghost()
                            .small()
                            .label("Load more refs")
                            .disabled(picker.loading)
                            .on_click(
                                cx.listener(|app, _, _, cx| app.query_diff_comparison(true, cx)),
                            ),
                    )
                })
            })
    }
}
