use crate::NativeApp;
use gpui_kit::component::{
    Disableable, Sizable,
    button::{Button, ButtonVariants},
    menu::{DropdownMenu, PopupMenuItem},
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
                self.diffs
                    .full_file
                    .as_ref()
                    .filter(|(index, _, _)| *index == self.diffs.selected_file),
                |view, (_, old, new)| {
                    view.child(
                        Button::new("diff-close-full")
                            .ghost()
                            .small()
                            .label("Close full contents")
                            .on_click(cx.listener(|app, _, _, cx| {
                                app.diffs.full_file = None;
                                cx.notify();
                            })),
                    )
                    .child(
                        div().flex().min_w_0().gap_2().children(
                            [("Before (complete)", old), ("After (complete)", new)]
                                .into_iter()
                                .map(|(label, body)| {
                                    let body = body.clone();
                                    let copy = body.clone();
                                    div()
                                        .flex()
                                        .flex_col()
                                        .flex_1()
                                        .min_w_0()
                                        .gap_1()
                                        .child(
                                            Button::new(gpui_kit::SharedString::from(format!(
                                                "copy-full-{label}"
                                            )))
                                            .ghost()
                                            .small()
                                            .label(format!("Copy {label}"))
                                            .on_click(move |_, _, cx| {
                                                cx.write_to_clipboard(
                                                    gpui_kit::ClipboardItem::new_string(
                                                        copy.clone(),
                                                    ),
                                                )
                                            }),
                                        )
                                        .child(
                                            div()
                                                .id(gpui_kit::SharedString::from(label))
                                                .max_h_64()
                                                .overflow_scroll()
                                                .whitespace_nowrap()
                                                .child(body),
                                        )
                                }),
                        ),
                    )
                },
            )
    }
    pub(crate) fn render_diff_options(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let refs = self
            .refs
            .iter()
            .map(|row| row.name.clone())
            .collect::<Vec<_>>();
        let entity = cx.entity().downgrade();
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
                        cx.notify();
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
                            .disabled(self.diffs.full_file_loading || !self.connection_ready())
                            .on_click(cx.listener(|app, _, _, cx| app.load_diff_file_contents(cx))),
                    )
                },
            )
            .when(
                self.diffs.git_scope.as_deref() == Some("branch-range"),
                |view| {
                    view.child(
                        Button::new("diff-base-ref")
                            .ghost()
                            .small()
                            .label(format!(
                                "Base: {}",
                                self.diffs.base_ref.as_deref().unwrap_or("Automatic")
                            ))
                            .dropdown_menu(move |mut menu, _, _| {
                                for base in std::iter::once(None)
                                    .chain(refs.iter().map(|r| Some(r.clone())))
                                {
                                    let entity = entity.clone();
                                    let label =
                                        base.clone().unwrap_or_else(|| "Automatic base".into());
                                    menu = menu.item(PopupMenuItem::new(label).on_click(
                                        move |_, _, cx| {
                                            let _ = entity.update(cx, |app, cx| {
                                                app.diffs.base_ref = base.clone();
                                                app.view_git_diff("branch-range", cx);
                                            });
                                        },
                                    ));
                                }
                                menu
                            }),
                    )
                    .child(
                        Button::new("diff-refresh-refs")
                            .ghost()
                            .small()
                            .label("Refresh refs")
                            .on_click(cx.listener(|app, _, _, cx| {
                                app.worktree_operation("vcs.listRefs", false, cx)
                            })),
                    )
                },
            )
    }
}
