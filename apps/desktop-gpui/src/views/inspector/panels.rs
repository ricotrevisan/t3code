//! The Changes and Agents tab bodies of the right panel.
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Selectable as _, Sizable as _,
    button::{Button, ButtonVariants as _},
};
use gpui_kit::{Context, div, prelude::*};

use crate::NativeApp;

impl NativeApp {
    pub(super) fn render_changes_panel(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let muted_foreground = cx.theme().muted_foreground;
        let checkpoints = self
            .detail
            .thread
            .as_ref()
            .filter(|detail| detail["id"].as_str() == self.selected_thread.as_deref())
            .and_then(|detail| detail["checkpoints"].as_array());
        let current = self.selected_thread.is_some()
            && self.diffs.thread_id == self.selected_thread
            && self.diffs.generation == self.generation;
        let selected = current.then_some(self.diffs.turn_count).flatten();
        let mono_font = cx.theme().mono_font_family.clone();
        div()
            .id("changes-panel")
            .debug_selector(|| "changes-panel".into())
            .flex()
            .flex_col()
            .gap_2()
            .flex_1()
            .min_h_0()
            .min_w_0()
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap_1()
                    .child(
                        Button::new("diff-working-tree")
                            .debug_selector(|| "diff-working-tree".into())
                            .ghost()
                            .small()
                            .label("Working tree")
                            .selected(self.diffs.git_scope.as_deref() == Some("working-tree"))
                            .on_click(
                                cx.listener(|app, _, _, cx| app.view_git_diff("working-tree", cx)),
                            ),
                    )
                    .child(
                        Button::new("diff-branch-range")
                            .debug_selector(|| "diff-branch-range".into())
                            .ghost()
                            .small()
                            .label("Branch changes")
                            .selected(self.diffs.git_scope.as_deref() == Some("branch-range"))
                            .on_click(
                                cx.listener(|app, _, _, cx| app.view_git_diff("branch-range", cx)),
                            ),
                    )
                    .child(
                        Button::new("diff-refresh")
                            .debug_selector(|| "diff-refresh".into())
                            .ghost()
                            .small()
                            .label("Refresh")
                            .on_click(cx.listener(|app, _, _, cx| {
                                if let Some(scope) = app.diffs.git_scope.clone() {
                                    app.view_git_diff(&scope, cx);
                                } else if let Some(turn) = app.diffs.turn_count {
                                    app.view_checkpoint_diff(turn, cx);
                                }
                            })),
                    )
                    .child(
                        Button::new("diff-wrap")
                            .debug_selector(|| "diff-wrap".into())
                            .ghost()
                            .small()
                            .label("Wrap lines")
                            .selected(self.diffs.wrap)
                            .on_click(cx.listener(|app, _, _, cx| {
                                app.diffs.wrap = !app.diffs.wrap;
                                app.save_diff_preferences(cx);
                            })),
                    )
                    .child(
                        Button::new("diff-expand-all")
                            .debug_selector(|| "diff-expand-all".into())
                            .ghost()
                            .small()
                            .label("Expand all")
                            .on_click(cx.listener(|app, _, _, cx| {
                                app.diffs.expand_all = true;
                                app.diffs.collapsed.clear();
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("diff-collapse-all")
                            .debug_selector(|| "diff-collapse-all".into())
                            .ghost()
                            .small()
                            .label("Collapse all")
                            .on_click(cx.listener(|app, _, _, cx| {
                                app.diffs.expand_all = true;
                                app.diffs.collapsed = (0..app.diffs.files.len()).collect();
                                cx.notify();
                            })),
                    ),
            )
            .child(self.render_diff_options(cx))
            .child(self.render_diff_comparison(cx))
            .child(self.render_full_diff_file(cx))
            .when(self.diffs.truncated, |view| {
                view.child(
                    div()
                        .text_xs()
                        .child(if self.diffs.files.first().is_some_and(|file| file.stats.is_some()) {
                            "Preview truncated; select any manifest file to load its patch. Full contents are available separately."
                        } else {
                            "Preview truncated; this server supplied no omitted-file manifest. Only returned files are navigable."
                        }),
                )
            })
            .when(self.diffs.patch_loading, |view| view.child("Loading selected file patch…"))
            .when_some(self.diffs.patch_error.clone(), |view, error| {
                view.child(div().text_xs().child(error)).child(
                    Button::new("diff-file-retry").debug_selector(|| "diff-file-retry".into())
                        .ghost().small().label("Retry file patch")
                        .on_click(cx.listener(|app, _, _, cx| app.load_selected_diff_patch(cx)))
                )
            })
            .when(
                self.diffs.git_scope.is_none() && checkpoints.is_none_or(|list| list.is_empty()),
                |view| {
                    view.child(
                        div()
                            .text_sm()
                            .text_color(muted_foreground)
                            .child("No checkpointed changes yet"),
                    )
                },
            )
            .child(
                div()
                    .id("diff-checkpoints")
                    .max_h_24()
                    .flex_shrink_0()
                    .overflow_y_scroll()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .children(checkpoints.into_iter().flatten().filter_map(|checkpoint| {
                        let turn = checkpoint["checkpointTurnCount"].as_u64()?;
                        Some(
                            Button::new(gpui_kit::SharedString::from(format!(
                                "diff-checkpoint-{turn}"
                            )))
                            .debug_selector(move || format!("diff-checkpoint-{turn}"))
                            .label(format!("Turn {turn}"))
                            .ghost()
                            .small()
                            .justify_start()
                            .selected(selected == Some(turn))
                            .on_click(
                                cx.listener(move |app, _, _, cx| {
                                    app.view_checkpoint_diff(turn, cx)
                                }),
                            ),
                        )
                    })),
            )
            .when(
                selected.is_none()
                    && self.diffs.git_scope.is_none()
                    && checkpoints.is_some_and(|list| !list.is_empty()),
                |view| {
                    view.child(
                        div()
                            .text_sm()
                            .child("Select a checkpoint to read its patch"),
                    )
                },
            )
            .when(current && self.diffs.loading, |view| {
                view.child(
                    div()
                        .id("diff-loading")
                        .debug_selector(|| "diff-loading".into())
                        .text_sm()
                        .child("Loading patch…"),
                )
            })
            .when(current && self.diffs.error.is_some(), |view| {
                view.child(
                    div()
                        .text_sm()
                        .child(self.diffs.error.clone().unwrap_or_default()),
                )
                .child(
                    Button::new("diff-retry")
                        .debug_selector(|| "diff-retry".into())
                        .label("Retry")
                        .ghost()
                        .small()
                        .on_click(cx.listener(move |app, _, _, cx| {
                            if let Some(scope) = app.diffs.git_scope.clone() {
                                app.view_git_diff(&scope, cx);
                            } else if let Some(turn) = app.diffs.turn_count {
                                app.view_checkpoint_diff(turn, cx);
                            }
                        })),
                )
            })
            .when(
                (selected.is_some() || self.diffs.git_scope.is_some())
                    && !self.diffs.loading
                    && self.diffs.error.is_none()
                    && self.diffs.files.is_empty(),
                |view| {
                    view.child(
                        div()
                            .id("diff-empty")
                            .debug_selector(|| "diff-empty".into())
                            .text_sm()
                            .child("No changes in this checkpoint"),
                    )
                },
            )
            .when(current && !self.diffs.files.is_empty(), |view| {
                view.child(self.render_diff_file_list(cx))
                .child(
                    Button::new("copy-diff")
                        .debug_selector(|| "copy-diff".into())
                        .icon(gpui_kit::assets::IconName::Copy)
                        .label("Copy patch")
                        .disabled(self.diffs.files.get(self.diffs.selected_file).is_none_or(|file| !file.loaded))
                        .ghost()
                        .small()
                        .on_click(cx.listener(|app, _, _, cx| {
                            if let Some(file) = app.diffs.files.get(app.diffs.selected_file) {
                                cx.write_to_clipboard(gpui_kit::ClipboardItem::new_string(
                                    file.patch.clone(),
                                ));
                            }
                        })),
                )
                .child(
                    div()
                        .id("diff-patch")
                        .debug_selector(|| "diff-patch".into())
                        .flex_1()
                        .min_h_0()
                        .min_w_0()
                        .overflow_scroll()
                        .font_family(mono_font)
                        .text_xs()
                        .flex()
                        .flex_col()
                        .gap_2()
                        .children(
                            self.diffs
                                .files
                                .iter()
                                .enumerate()
                                .skip(if self.diffs.expand_all { 0 } else { self.diffs.selected_file })
                                .take(if self.diffs.expand_all { usize::MAX } else { 1 })
                                .map(|(index, file)| {
                                    div()
                                        .flex()
                                        .flex_col()
                                        .min_w_0()
                                        .gap_1()
                                        .child(
                                            Button::new(gpui_kit::SharedString::from(format!(
                                                "diff-disclosure-{index}"
                                            )))
                                            .ghost()
                                            .small()
                                            .justify_start()
                                            .label(file.path.clone())
                                            .on_click(cx.listener(move |app, _, _, cx| {
                                                if !app.diffs.collapsed.remove(&index) {
                                                    app.diffs.collapsed.insert(index);
                                                }
                                                cx.notify();
                                            })),
                                        )
                                        .when(file.truncated, |view| view.child("File patch truncated; load full before / after for complete contents"))
                                        .when(!file.loaded && !self.diffs.collapsed.contains(&index), |view| {
                                            view.child(
                                                Button::new(gpui_kit::SharedString::from(format!("diff-load-{index}")))
                                                    .ghost().small().label("Load file patch")
                                                    .on_click(cx.listener(move |app, _, _, cx| app.select_diff_file(index, cx)))
                                            )
                                        })
                                        .when(file.loaded && !self.diffs.collapsed.contains(&index), |view| {
                                            view.child(self.render_patch_lines(file, cx))
                                        })
                                }),
                        ),
                )
            })
    }

    pub(crate) fn render_pull_requests_panel(
        &self,
        cx: &mut Context<Self>,
    ) -> gpui_kit::AnyElement {
        self.render_pull_requests(cx)
    }

    pub(super) fn render_agents_panel(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        self.render_runtime_agents(cx)
    }
}
