//! The Changes and Agents tab bodies of the right panel.
use gpui_kit::component::{
    ActiveTheme as _, Selectable as _, Sizable as _,
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
            .when(checkpoints.is_none_or(|list| list.is_empty()), |view| {
                view.child(
                    div()
                        .text_sm()
                        .text_color(muted_foreground)
                        .child("No checkpointed changes yet"),
                )
            })
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
                selected.is_none() && checkpoints.is_some_and(|list| !list.is_empty()),
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
                        .child("Loading checkpoint patch…"),
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
                            if let Some(turn) = app.diffs.turn_count {
                                app.view_checkpoint_diff(turn, cx);
                            }
                        })),
                )
            })
            .when(
                selected.is_some()
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
                view.child(
                    div()
                        .id("diff-files")
                        .max_h_24()
                        .flex_shrink_0()
                        .overflow_y_scroll()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .children(self.diffs.files.iter().enumerate().map(|(index, file)| {
                            Button::new(gpui_kit::SharedString::from(format!("diff-file-{index}")))
                                .debug_selector(move || format!("diff-file-{index}"))
                                .label(file.path.clone())
                                .ghost()
                                .small()
                                .selected(self.diffs.selected_file == index)
                                .on_click(cx.listener(move |app, _, _, cx| {
                                    app.diffs.selected_file = index;
                                    cx.notify();
                                }))
                        })),
                )
                .child(
                    Button::new("copy-diff")
                        .debug_selector(|| "copy-diff".into())
                        .icon(gpui_kit::assets::IconName::Copy)
                        .label("Copy patch")
                        .ghost()
                        .small()
                        .on_click(cx.listener(|app, _, _, cx| {
                            if let Some(file) = app.diffs.files.get(app.diffs.selected_file) {
                                cx.write_to_clipboard(gpui_kit::ClipboardItem::new_string(file.patch.clone()));
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
                        .whitespace_nowrap()
                        .text_xs()
                        .child(
                            self.diffs
                                .files
                                .get(self.diffs.selected_file)
                                .map(|file| file.patch.clone())
                                .unwrap_or_default(),
                        ),
                )
            })
    }

    pub(super) fn render_agents_panel(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let muted_foreground = cx.theme().muted_foreground;
        div()
            .id("agents-panel")
            .debug_selector(|| "agents-panel".into())
            .flex()
            .flex_col()
            .gap_2()
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .when(self.provider_catalog.is_empty(), |view| {
                view.child(
                    div()
                        .text_sm()
                        .text_color(muted_foreground)
                        .child("No provider catalog loaded"),
                )
            })
            .children(self.provider_catalog.iter().map(|provider| {
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(div().text_sm().child(provider.label().to_owned()))
                    .child(div().text_xs().text_color(muted_foreground).child(format!(
                        "{} · {}",
                        provider.instance_id,
                        provider.explanation()
                    )))
            }))
    }
}
