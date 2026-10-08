use crate::NativeApp;
use gpui_kit::component::{
    Disableable, Sizable,
    button::{Button, ButtonVariants},
};
use gpui_kit::{Context, div, prelude::*};
impl NativeApp {
    pub(crate) fn render_full_diff_file(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        div()
            .flex()
            .flex_col()
            .min_w_0()
            .gap_1()
            .when(self.diffs.full_file_loading, |view| {
                view.child("Loading full file contents…")
            })
            .when_some(self.diffs.full_file_error.clone(), |view, error| {
                view.child(error)
            })
            .when_some(
                self.diffs.full_file_view.as_ref().filter(|_| {
                    self.diffs
                        .full_file
                        .as_ref()
                        .is_some_and(|(index, _, _)| *index == self.diffs.selected_file)
                }),
                |view, entity| {
                    view.child(
                        Button::new("diff-close-full")
                            .debug_selector(|| "diff-close-full".into())
                            .ghost()
                            .small()
                            .label("Close full contents")
                            .on_click(cx.listener(|app, _, _, cx| {
                                app.diffs.full_file = None;
                                app.diffs.full_file_view = None;
                                cx.notify();
                            })),
                    )
                    .child(
                        entity.clone().cached(
                            gpui_kit::StyleRefinement::default()
                                .w_full()
                                .h(gpui_kit::rems(14.)),
                        ),
                    )
                },
            )
    }
    pub(crate) fn render_diff_options(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        div()
            .flex()
            .flex_wrap()
            .gap_1()
            .child(
                Button::new("diff-split")
                    .debug_selector(|| "diff-split".into())
                    .ghost()
                    .small()
                    .label(if self.diffs.split { "Unified" } else { "Split" })
                    .on_click(cx.listener(|app, _, _, cx| {
                        app.diffs.split = !app.diffs.split;
                        app.save_diff_preferences(cx);
                    })),
            )
            .when(self.diffs.git_scope.is_some(), |view| {
                view.child(
                    Button::new("diff-whitespace")
                        .debug_selector(|| "diff-whitespace".into())
                        .ghost()
                        .small()
                        .label(if self.diffs.ignore_whitespace {
                            "Show whitespace changes"
                        } else {
                            "Ignore whitespace changes"
                        })
                        .on_click(cx.listener(|app, _, _, cx| {
                            app.diffs.ignore_whitespace = !app.diffs.ignore_whitespace;
                            app.save_diff_preferences(cx);
                            if let Some(scope) = app.diffs.git_scope.clone() {
                                app.view_git_diff(&scope, cx);
                            }
                        })),
                )
            })
            .when(!self.diffs.files.is_empty(), |view| {
                view.child(
                    Button::new("diff-open-editor")
                        .debug_selector(|| "diff-open-editor".into())
                        .ghost()
                        .small()
                        .label("Open file in editor")
                        .disabled(
                            !self.connection_ready() || self.editor_unavailable_reason().is_some(),
                        )
                        .on_click(cx.listener(|app, _, _, cx| app.open_diff_editor(cx))),
                )
            })
            .when(
                self.diffs.git_scope.is_some() && !self.diffs.files.is_empty(),
                |view| {
                    view.child(
                        Button::new("diff-full-file")
                            .debug_selector(|| "diff-full-file".into())
                            .ghost()
                            .small()
                            .label("Load full before / after")
                            .disabled(
                                self.diffs.full_file_loading
                                    || !self.connection_ready()
                                    || self
                                        .diffs
                                        .files
                                        .get(self.diffs.selected_file)
                                        .is_none_or(|file| !file.loaded),
                            )
                            .on_click(cx.listener(|app, _, _, cx| app.load_diff_file_contents(cx))),
                    )
                },
            )
            .when(
                self.diffs.git_scope.as_deref() == Some("branch-range"),
                |view| {
                    view.child(
                        Button::new("diff-base-ref")
                            .debug_selector(|| "diff-base-ref".into())
                            .ghost()
                            .small()
                            .label(format!(
                                "Base: {}",
                                self.diffs.base_ref.as_deref().unwrap_or("Automatic")
                            ))
                            .disabled(!self.connection_ready())
                            .on_click(cx.listener(|app, _, window, cx| {
                                app.show_diff_comparison(window, cx)
                            })),
                    )
                },
            )
    }
}
