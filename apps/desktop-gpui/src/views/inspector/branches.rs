use gpui_kit::base::Disableable;
use gpui_kit::base::Selectable;
use gpui_kit::component::{
    ActiveTheme as _, Sizable as _,
    button::{Button, ButtonVariants as _},
    input::Input,
};
use gpui_kit::{Context, div, prelude::*};
use serde_json::json;

use super::field;
use crate::NativeApp;

impl NativeApp {
    pub(super) fn render_setup_card(
        &self,
        cx: &mut Context<Self>,
    ) -> Option<impl IntoElement + use<>> {
        let muted_foreground = cx.theme().muted_foreground;
        let border = cx.theme().border;
        let group_box = cx.theme().group_box;
        let setup = self.selected_setup()?;
        let id = setup.thread_id.clone();
        let show = self.setup_expanded;
        Some(
            div()
                .w_full()
                .min_w_0()
                .whitespace_normal()
                .flex()
                .flex_col()
                .gap_1()
                .p_2()
                .rounded_md()
                .bg(group_box)
                .border_1()
                .border_color(border)
                .child(format!(
                    "Worktree setup · {}{}",
                    setup.phase,
                    setup
                        .branch
                        .as_ref()
                        .map_or(String::new(), |branch| format!(" · {branch}"))
                ))
                .child(
                    Button::new("setup-details")
                        .ghost()
                        .label(if show {
                            "Hide setup details"
                        } else {
                            "Show setup details"
                        })
                        .on_click(cx.listener(|app, _, _, cx| {
                            app.setup_expanded = !app.setup_expanded;
                            cx.notify();
                        })),
                )
                .when_some(setup.worktree_path.clone(), |view, path| {
                    view.child(
                        Button::new("copy-worktree-path")
                            .ghost()
                            .label("Copy worktree path")
                            .tooltip(path.clone())
                            .on_click(cx.listener(move |_, _, _, cx| {
                                cx.write_to_clipboard(gpui_kit::ClipboardItem::new_string(
                                    path.clone(),
                                ))
                            })),
                    )
                })
                .children(setup.error.clone())
                .when(show, |view| {
                    view.children(setup.stages.iter().map(|stage| {
                        div()
                            .w_full()
                            .min_w_0()
                            .whitespace_normal()
                            .flex()
                            .flex_col()
                            .text_color(muted_foreground)
                            .child(format!(
                                "{} · {}{}",
                                stage.id,
                                stage.status,
                                stage
                                    .percent
                                    .map_or(String::new(), |percent| format!(" {percent}%"))
                            ))
                            .children(stage.detail.clone())
                            .children(stage.tail.clone())
                    }))
                })
                .when(setup.phase == "running", |view| {
                    view.child(
                        Button::new("setup-cancel").disabled(!self.connection_ready())
                            .danger()
                            .outline()
                            .label("Cancel worktree setup")
                            .on_click(cx.listener(move |app, _, _, cx| {
                                app.perform(
                                    "worktreeSetup.cancel",
                                    json!({"threadId":id}),
                                    None,
                                    cx,
                                )
                            })),
                    )
                }),
        )
    }

    pub(super) fn render_worktrees(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        div()
            .flex()
            .flex_col()
            .gap_2()
            .child(div().text_xs().child(self.next_checkout_label()))
            .child(
                gpui_kit::component::switch::Switch::new("prepare-worktree")
                    .small()
                    .label("New worktree")
                    .checked(self.prepare_new_worktree)
                    .on_click(cx.listener(|app, value: &bool, _, cx| {
                        app.prepare_new_worktree = *value;
                        cx.notify();
                    })),
            )
            .child(
                gpui_kit::component::switch::Switch::new("worktree-origin")
                    .small()
                    .label("Start from origin")
                    .checked(self.start_from_origin)
                    .on_click(cx.listener(|app, value: &bool, _, cx| {
                        app.start_from_origin = *value;
                        cx.notify();
                    })),
            )
            .child(
                gpui_kit::component::switch::Switch::new("worktree-setup-script")
                    .small()
                    .label("Run setup script")
                    .checked(self.run_setup_script)
                    .on_click(cx.listener(|app, value: &bool, _, cx| {
                        app.run_setup_script = *value;
                        cx.notify();
                    })),
            )
            .child(field("Base branch", Input::new(&self.worktree_ref), cx))
            .child(field("New branch", Input::new(&self.worktree_branch), cx))
            .child(field("Worktree path", Input::new(&self.worktree_path), cx))
            .child(
                Button::new("draft-checkout-update")
                    .debug_selector(|| "draft-checkout-update".into())
                    .label("Update selected draft checkout")
                    .secondary()
                    .on_click(cx.listener(|app, _, window, cx| {
                        app.update_draft_checkout(cx);
                        app.reset_checkout_context(window, cx);
                    })),
            )
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap_2()
                    .child(
                        Button::new("refs-refresh").disabled(!self.connection_ready())
                            .label("Refresh branches")
                            .ghost()
                            .on_click(cx.listener(|app, _, _, cx| {
                                app.worktree_operation("vcs.listRefs", false, cx)
                            })),
                    )
                    .child(
                        Button::new("worktree-create").disabled(!self.connection_ready())
                            .label("Create worktree")
                            .primary()
                            .on_click(cx.listener(|app, _, _, cx| {
                                app.worktree_operation("vcs.createWorktree", false, cx)
                            })),
                    )
                    .child(
                        Button::new("ref-switch").disabled(!self.connection_ready())
                            .label("Switch branch")
                            .secondary()
                            .on_click(cx.listener(|app, _, window, cx| {
                                let branch = app.worktree_ref.read(cx).value().trim().to_owned();
                                if !branch.is_empty() {
                                    app.pick_checkout(branch, None, window, cx);
                                }
                            })),
                    )
                    .child(
                        Button::new("worktree-remove").disabled(!self.connection_ready())
                            .label("Remove worktree")
                            .danger()
                            .outline()
                            .on_click(cx.listener(|app, _, _, cx| {
                                app.worktree_operation("vcs.removeWorktree", false, cx)
                            })),
                    ),
            )
            .children(self.refs.iter().map(|row| {
                let branch = row.name.clone();
                let path = row.worktree_path.clone();
                Button::new(gpui_kit::SharedString::from(format!(
                    "choose-ref-{}",
                    row.name
                )))
                .ghost()
                .justify_start()
                .selected(self.is_next_checkout(row))
                .label(format!(
                    "{}{}",
                    if row.current { "● " } else { "" },
                    row.name
                ))
                .tooltip(
                    row.worktree_path
                        .clone()
                        .unwrap_or_else(|| "No worktree checkout".into()),
                )
                .on_click(cx.listener(move |app, _, window, cx| {
                    app.choose_checkout(branch.clone(), path.clone(), window, cx)
                }))
            }))
            .when_some(self.worktree_outcome.as_ref(), |view, result| {
                view.child(
                    div()
                        .w_full()
                        .min_w_0()
                        .whitespace_normal()
                        .child(format!(
                            "Checkout updated: {}",
                            result["worktree"]["refName"]
                                .as_str()
                                .or_else(|| result["refName"].as_str())
                                .unwrap_or("removed")
                        ))
                        .when_some(result["worktree"]["path"].as_str(), |view, path| {
                            let path = path.to_owned();
                            view.child(
                                Button::new("copy-created-worktree")
                                    .ghost()
                                    .label("Copy worktree path")
                                    .tooltip(path.clone())
                                    .on_click(cx.listener(move |_, _, _, cx| {
                                        cx.write_to_clipboard(gpui_kit::ClipboardItem::new_string(
                                            path.clone(),
                                        ))
                                    })),
                            )
                        }),
                )
            })
            .when(self.refs_cursor.is_some(), |view| {
                view.child(
                    Button::new("refs-more").disabled(!self.connection_ready())
                        .ghost()
                        .label("More branches")
                        .on_click(cx.listener(|app, _, _, cx| {
                            app.worktree_operation("vcs.listRefs", true, cx)
                        })),
                )
            })
    }
}
