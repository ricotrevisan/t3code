use gpui_kit::base::Selectable;
use gpui_kit::component::{
    ActiveTheme as _, Disableable, Sizable,
    button::{Button, ButtonVariants as _},
    input::{Input, Textarea},
};
use gpui_kit::{Context, FontWeight, div, prelude::*, px};
use serde_json::json;
use t3code_desktop_gpui::{connection, palette, project_actions};

use crate::{NativeApp, RightTab};
impl NativeApp {
    pub(crate) fn render_inspector(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let border = cx.theme().border;
        let group_box = cx.theme().group_box;
        div()
            .id("inspector")
            .when(!self.inspector_open, |view| view.hidden())
            .flex()
            .flex_col()
            .gap_2()
            .w(px(340.))
            .flex_shrink_0()
            .h_full()
            .p_3()
            .bg(group_box)
            .border_l_1()
            .border_color(border)
            .child(
                div().flex().gap_1().children(
                    [
                        ("right-tab-inspector", "Inspector", RightTab::Inspector),
                        ("right-tab-changes", "Changes", RightTab::Changes),
                        ("right-tab-agents", "Agents", RightTab::Agents),
                    ]
                    .into_iter()
                    .map(|(id, label, tab)| {
                        Button::new(id)
                            .debug_selector(move || id.into())
                            .label(label)
                            .ghost()
                            .small()
                            .selected(self.right_tab == tab)
                            .on_click(cx.listener(move |app, _, _, cx| {
                                app.right_tab = tab;
                                cx.notify();
                            }))
                    }),
                ),
            )
            .child(self.render_inspector_body(cx))
            .when(self.right_tab == RightTab::Changes, |view| {
                view.child(self.render_changes_panel(cx))
            })
            .when(self.right_tab == RightTab::Agents, |view| {
                view.child(self.render_agents_panel(cx))
            })
    }

    fn render_inspector_body(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let muted_foreground = cx.theme().muted_foreground;
        let border = cx.theme().border;
        let group_box = cx.theme().group_box;
        let danger = cx.theme().danger;
        div()
            .id("inspector-body")
            .when(self.right_tab != RightTab::Inspector, |view| view.hidden())
            .flex()
            .flex_col()
            .gap_2()
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .children(
                self.storage_error
                    .clone()
                    .map(|error| div().text_color(danger).child(error)),
            )
            .child(
                div()
                    .text_xs()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(muted_foreground)
                    .child("INSPECTOR"),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .p_2()
                    .rounded_md()
                    .bg(group_box)
                    .border_1()
                    .border_color(border)
                    .child(
                        div()
                            .text_xs()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(muted_foreground)
                            .child("PROJECT"),
                    )
                    .child(
                        div()
                            .flex()
                            .gap_2()
                            .child(Input::new(&self.title))
                            .child(Input::new(&self.workspace)),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .gap_2()
                            .child(
                                Button::new("add-project")
                                    .label("Add project")
                                    .primary()
                                    .on_click(cx.listener(|app, _, window, cx| {
                                        app.execute_command(
                                            palette::Command::AddProject,
                                            window,
                                            cx,
                                        )
                                    })),
                            )
                            .child(
                                Button::new("rename-project")
                                    .debug_selector(|| "rename-project".into())
                                    .label("Rename project")
                                    .ghost()
                                    .on_click(
                                        cx.listener(|app, _, _, cx| app.update_project(false, cx)),
                                    ),
                            )
                            .child(
                                Button::new("remove-project")
                                    .debug_selector(|| "remove-project".into())
                                    .label("Remove project")
                                    .danger()
                                    .outline()
                                    .on_click(
                                        cx.listener(|app, _, _, cx| app.update_project(true, cx)),
                                    ),
                            )
                            .child(
                                Button::new("new-thread")
                                    .label("New thread")
                                    .secondary()
                                    .on_click(cx.listener(|app, _, window, cx| {
                                        app.execute_command(palette::Command::NewThread, window, cx)
                                    })),
                            ),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .p_2()
                    .rounded_md()
                    .bg(group_box)
                    .border_1()
                    .border_color(border)
                    .child(
                        div()
                            .text_xs()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(muted_foreground)
                            .child("SNOOZE"),
                    )
                    .child(
                        div()
                            .flex()
                            .gap_2()
                            .child(Input::new(&self.custom_snooze))
                            .child(
                                Button::new("custom-snooze")
                                    .label("Custom snooze")
                                    .secondary()
                                    .on_click(cx.listener(|app, _, _, cx| app.snooze_custom(cx))),
                            ),
                    ),
            )
            .children(self.render_setup_card(cx))
            .child(
                Button::new("project-actions")
                    .label("Project actions")
                    .secondary()
                    .on_click(cx.listener(|app, _, _, cx| {
                        app.project_actions_open = true;
                        app.project_actions(None, cx);
                    })),
            )
            .when(self.project_actions_open, |view| {
                view.child(self.render_project_actions(cx))
            })
            .child(
                Button::new("worktrees")
                    .label("Branches and worktrees")
                    .secondary()
                    .on_click(cx.listener(|app, _, window, cx| {
                        app.execute_command(palette::Command::Worktrees, window, cx)
                    })),
            )
            .when(self.worktrees_open, |view| {
                view.child(self.render_worktrees(cx))
            })
            .child(
                Button::new("clone-repository")
                    .label("Clone repository")
                    .secondary()
                    .on_click(cx.listener(|app, _, window, cx| {
                        app.execute_command(palette::Command::CloneProject, window, cx)
                    })),
            )
            .when(self.clone_open, |view| view.child(self.render_clone(cx)))
            .child(
                Button::new("files")
                    .debug_selector(|| "files".into())
                    .label("Workspace file editor")
                    .secondary()
                    .on_click(cx.listener(|app, _, _, cx| {
                        app.files_open = !app.files_open;
                        cx.notify();
                    })),
            )
            .when(self.files_open, |view| view.child(self.render_files(cx)))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .p_2()
                    .rounded_md()
                    .bg(group_box)
                    .border_1()
                    .border_color(border)
                    .child(
                        div()
                            .text_xs()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(muted_foreground)
                            .child("MODEL"),
                    )
                    .child(
                        div()
                            .flex()
                            .gap_2()
                            .child(Input::new(&self.provider))
                            .child(Input::new(&self.model)),
                    ),
            )
    }

    fn render_setup_card(&self, cx: &mut Context<Self>) -> Option<impl IntoElement + use<>> {
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
                        Button::new("setup-cancel")
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

    fn render_project_actions(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let muted_foreground = cx.theme().muted_foreground;
        let border = cx.theme().border;
        let group_box = cx.theme().group_box;
        let scripts = self
            .selected_project
            .as_ref()
            .and_then(|id| {
                self.project_settings.as_ref().map(|settings| {
                    project_actions::resolve(
                        settings,
                        id,
                        self.shell
                            .projects
                            .iter()
                            .find(|p| &p.id == id)
                            .map(|p| p.scripts.as_slice())
                            .unwrap_or_default(),
                    )
                })
            })
            .unwrap_or_default();
        div()
            .flex()
            .flex_col()
            .gap_2()
            .min_w_0()
            .p_2()
            .rounded_md()
            .bg(group_box)
            .border_1()
            .border_color(border)
            .child(
                div()
                    .text_xs()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(muted_foreground)
                    .child("PROJECT ACTIONS"),
            )
            .child("Actions for the selected project on this environment")
            .child(Input::new(&self.action_name))
            .child(Input::new(&self.action_command))
            .child(
                Button::new("action-setup")
                    .ghost()
                    .selected(self.action_setup)
                    .label(if self.action_setup {
                        "Runs on worktree creation"
                    } else {
                        "Run manually"
                    })
                    .on_click(cx.listener(|app, _, _, cx| {
                        app.action_setup = !app.action_setup;
                        cx.notify();
                    })),
            )
            .child(
                Button::new("action-wait")
                    .ghost()
                    .selected(self.action_wait)
                    .label(if self.action_wait {
                        "Wait before starting agent"
                    } else {
                        "Start agent while setup runs"
                    })
                    .on_click(cx.listener(|app, _, _, cx| {
                        app.action_wait = !app.action_wait;
                        cx.notify();
                    })),
            )
            .child(
                Button::new("action-save")
                    .debug_selector(|| "action-save".into())
                    .label("Save action")
                    .primary()
                    .disabled(self.action_busy)
                    .on_click(cx.listener(|app, _, _, cx| app.save_project_action(cx))),
            )
            .child(
                Button::new("action-new")
                    .label("New action")
                    .secondary()
                    .on_click(cx.listener(|app, _, window, cx| {
                        app.action_edit_id = None;
                        app.confirm_action_delete = None;
                        app.action_setup = false;
                        app.action_wait = false;
                        app.action_name
                            .update(cx, |input, cx| input.set_value("", window, cx));
                        app.action_command
                            .update(cx, |input, cx| input.set_value("", window, cx));
                        cx.notify();
                    })),
            )
            .children(scripts.into_iter().enumerate().map(|(index, script)| {
                let edit = script.clone();
                let id = script["id"].as_str().unwrap_or_default().to_owned();
                let confirm = self.confirm_action_delete.as_ref() == Some(&id);
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .min_w_0()
                    .whitespace_normal()
                    .child(format!(
                        "{} · {}{}",
                        script["name"].as_str().unwrap_or_default(),
                        script["command"].as_str().unwrap_or_default(),
                        if script["runOnWorktreeCreate"] == true {
                            " · worktree setup"
                        } else {
                            ""
                        }
                    ))
                    .child(
                        Button::new(gpui_kit::SharedString::from(format!("action-edit-{index}")))
                            .label("Edit action")
                            .ghost()
                            .on_click(cx.listener(move |app, _, window, cx| {
                                app.action_edit_id = edit["id"].as_str().map(str::to_owned);
                                app.action_setup = edit["runOnWorktreeCreate"] == true;
                                app.action_wait = edit["async"] == false;
                                app.action_name.update(cx, |input, cx| {
                                    input.set_value(
                                        edit["name"].as_str().unwrap_or_default().to_owned(),
                                        window,
                                        cx,
                                    )
                                });
                                app.action_command.update(cx, |input, cx| {
                                    input.set_value(
                                        edit["command"].as_str().unwrap_or_default().to_owned(),
                                        window,
                                        cx,
                                    )
                                });
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new(gpui_kit::SharedString::from(format!(
                            "action-delete-{index}"
                        )))
                        .label(if confirm {
                            "Confirm delete action"
                        } else {
                            "Delete action"
                        })
                        .danger()
                        .outline()
                        .disabled(self.action_busy)
                        .on_click(cx.listener(move |app, _, _, cx| {
                            if app.confirm_action_delete.as_ref() == Some(&id) {
                                app.project_actions(
                                    Some(json!({"operation":"delete","id":id})),
                                    cx,
                                );
                            } else {
                                app.confirm_action_delete = Some(id.clone());
                                cx.notify();
                            }
                        })),
                    )
            }))
            .child(
                Button::new("project-default-save")
                    .label("Use selected provider and model for new threads")
                    .secondary()
                    .on_click(cx.listener(|app, _, _, cx| app.update_project_default(false, cx))),
            )
            .child(
                Button::new("project-default-inherit")
                    .label("Use environment model default")
                    .ghost()
                    .on_click(cx.listener(|app, _, _, cx| {
                        app.project_actions(Some(json!({"operation":"model-inherit"})), cx)
                    })),
            )
            .child(
                Button::new("project-default-clear")
                    .label("Clear project model default")
                    .ghost()
                    .on_click(cx.listener(|app, _, _, cx| app.update_project_default(true, cx))),
            )
            .child(
                Button::new("actions-refresh")
                    .label("Refresh actions")
                    .secondary()
                    .disabled(self.action_busy)
                    .on_click(cx.listener(|app, _, _, cx| app.project_actions(None, cx))),
            )
            .child(
                Button::new("actions-inherit")
                    .label("Use environment action defaults")
                    .secondary()
                    .disabled(self.action_busy)
                    .on_click(cx.listener(|app, _, _, cx| {
                        app.project_actions(Some(json!({"operation":"inherit"})), cx)
                    })),
            )
            .child(
                Button::new("actions-close")
                    .label("Close actions")
                    .ghost()
                    .on_click(cx.listener(|app, _, _, cx| {
                        app.project_actions_open = false;
                        cx.notify();
                    })),
            )
    }

    fn render_worktrees(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let muted_foreground = cx.theme().muted_foreground;
        let border = cx.theme().border;
        let group_box = cx.theme().group_box;
        div()
            .flex()
            .flex_col()
            .gap_2()
            .p_2()
            .rounded_md()
            .bg(group_box)
            .border_1()
            .border_color(border)
            .child(
                div()
                    .text_xs()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(muted_foreground)
                    .child("BRANCHES"),
            )
            .child(
                Button::new("prepare-worktree")
                    .ghost()
                    .selected(self.prepare_new_worktree)
                    .label(if self.prepare_new_worktree {
                        "Next new thread: new worktree"
                    } else {
                        "Next new thread: selected checkout"
                    })
                    .on_click(cx.listener(|app, _, _, cx| {
                        app.prepare_new_worktree = !app.prepare_new_worktree;
                        cx.notify();
                    })),
            )
            .child(
                Button::new("worktree-origin")
                    .ghost()
                    .selected(self.start_from_origin)
                    .label(if self.start_from_origin {
                        "Base: fetch from origin"
                    } else {
                        "Base: local branch"
                    })
                    .on_click(cx.listener(|app, _, _, cx| {
                        app.start_from_origin = !app.start_from_origin;
                        cx.notify();
                    })),
            )
            .child(
                Button::new("worktree-setup-script")
                    .ghost()
                    .selected(self.run_setup_script)
                    .label(if self.run_setup_script {
                        "Run setup script"
                    } else {
                        "Skip setup script"
                    })
                    .on_click(cx.listener(|app, _, _, cx| {
                        app.run_setup_script = !app.run_setup_script;
                        cx.notify();
                    })),
            )
            .child(Input::new(&self.worktree_ref))
            .child(Input::new(&self.worktree_branch))
            .child(Input::new(&self.worktree_path))
            .child(
                Button::new("draft-checkout-update")
                    .debug_selector(|| "draft-checkout-update".into())
                    .label("Update selected draft checkout")
                    .secondary()
                    .on_click(cx.listener(|app, _, _, cx| app.update_draft_checkout(cx))),
            )
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap_2()
                    .child(
                        Button::new("refs-refresh")
                            .label("Refresh branches")
                            .secondary()
                            .on_click(cx.listener(|app, _, _, cx| {
                                app.worktree_operation("vcs.listRefs", false, cx)
                            })),
                    )
                    .child(
                        Button::new("worktree-create")
                            .label("Create worktree")
                            .primary()
                            .on_click(cx.listener(|app, _, _, cx| {
                                app.worktree_operation("vcs.createWorktree", false, cx)
                            })),
                    )
                    .child(
                        Button::new("ref-switch")
                            .label("Switch branch")
                            .secondary()
                            .on_click(cx.listener(|app, _, _, cx| {
                                app.worktree_operation("vcs.switchRef", false, cx)
                            })),
                    )
                    .child(
                        Button::new("worktree-remove")
                            .label("Remove worktree")
                            .danger()
                            .outline()
                            .on_click(cx.listener(|app, _, _, cx| {
                                app.worktree_operation("vcs.removeWorktree", false, cx)
                            })),
                    )
                    .child(
                        Button::new("worktrees-close")
                            .label("Close")
                            .ghost()
                            .on_click(cx.listener(|app, _, _, cx| {
                                app.worktrees_open = false;
                                cx.notify();
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
                .selected(row.current)
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
                    Button::new("refs-more")
                        .ghost()
                        .label("More branches")
                        .on_click(cx.listener(|app, _, _, cx| {
                            app.worktree_operation("vcs.listRefs", true, cx)
                        })),
                )
            })
    }

    fn render_clone(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let muted_foreground = cx.theme().muted_foreground;
        let border = cx.theme().border;
        let group_box = cx.theme().group_box;
        div()
            .flex()
            .flex_col()
            .gap_2()
            .p_2()
            .rounded_md()
            .bg(group_box)
            .border_1()
            .border_color(border)
            .child(
                div()
                    .text_xs()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(muted_foreground)
                    .child("CLONE"),
            )
            .child(
                Button::new("clone-close")
                    .label("Close clone panel")
                    .ghost()
                    .on_click(cx.listener(|app, _, _, cx| {
                        app.clone_open = false;
                        if let Some(sender) = &app.commands {
                            let _ = sender.try_send(connection::Command::Unsubscribe(
                                "project-clones".into(),
                            ));
                        }
                        cx.notify();
                    })),
            )
            .child(Input::new(&self.clone_remote))
            .child(
                Button::new("clone-start")
                    .debug_selector(|| "clone-start".into())
                    .label("Start clone to project directory")
                    .primary()
                    .on_click(cx.listener(|app, _, _, cx| app.clone_project(cx))),
            )
            .children(self.clone_progress.iter().map(|row| {
                let cancel_id = row.project_id.clone();
                let retry_id = cancel_id.clone();
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(format!(
                        "{} · {} · {}{}",
                        row.destination_path,
                        row.phase,
                        row.stage,
                        row.percent
                            .map_or(String::new(), |percent| format!(" {percent}%"))
                    ))
                    .children(row.detail.clone())
                    .children(row.error.clone())
                    .when(row.phase == "running", |view| {
                        view.child(
                            Button::new(gpui_kit::SharedString::from(format!(
                                "clone-cancel-{cancel_id}"
                            )))
                            .label("Cancel clone")
                            .danger()
                            .outline()
                            .on_click(cx.listener(
                                move |app, _, _, cx| {
                                    app.perform(
                                        "projectClone.cancel",
                                        json!({"projectId":cancel_id}),
                                        None,
                                        cx,
                                    )
                                },
                            )),
                        )
                    })
                    .when(row.phase == "failed" || row.phase == "cancelled", |view| {
                        view.child(
                            Button::new(gpui_kit::SharedString::from(format!(
                                "clone-retry-{retry_id}"
                            )))
                            .label("Retry clone")
                            .secondary()
                            .on_click(cx.listener(
                                move |app, _, _, cx| {
                                    app.perform(
                                        "projectClone.retry",
                                        json!({"projectId":retry_id}),
                                        None,
                                        cx,
                                    )
                                },
                            )),
                        )
                    })
            }))
    }

    fn render_files(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let muted_foreground = cx.theme().muted_foreground;
        let border = cx.theme().border;
        let group_box = cx.theme().group_box;
        let selection = cx.theme().selection;
        div()
            .flex()
            .flex_col()
            .gap_2()
            .p_2()
            .rounded_md()
            .bg(group_box)
            .border_1()
            .border_color(border)
            .child(
                div()
                    .text_xs()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(muted_foreground)
                    .child("FILES"),
            )
            .child(
                div()
                    .key_context("NativeContentSearch")
                    .child(Input::new(&self.content_query)),
            )
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap_2()
                    .child(
                        Button::new("content-case")
                            .ghost()
                            .selected(self.content_options.case_sensitive)
                            .label(if self.content_options.case_sensitive {
                                "Case sensitive ✓"
                            } else {
                                "Case sensitive"
                            })
                            .on_click(cx.listener(|app, _, _, cx| {
                                app.content_options.case_sensitive =
                                    !app.content_options.case_sensitive;
                                app.search_contents(cx);
                            })),
                    )
                    .child(
                        Button::new("content-word")
                            .ghost()
                            .selected(self.content_options.whole_word)
                            .label(if self.content_options.whole_word {
                                "Whole word ✓"
                            } else {
                                "Whole word"
                            })
                            .on_click(cx.listener(|app, _, _, cx| {
                                app.content_options.whole_word = !app.content_options.whole_word;
                                app.search_contents(cx);
                            })),
                    )
                    .child(
                        Button::new("content-regex")
                            .ghost()
                            .selected(self.content_options.use_regex)
                            .label(if self.content_options.use_regex {
                                "Regex ✓"
                            } else {
                                "Regex"
                            })
                            .on_click(cx.listener(|app, _, _, cx| {
                                app.content_options.use_regex = !app.content_options.use_regex;
                                app.search_contents(cx);
                            })),
                    )
                    .child(
                        Button::new("search-contents")
                            .debug_selector(|| "search-contents".into())
                            .label("Search contents")
                            .primary()
                            .on_click(cx.listener(|app, _, _, cx| app.search_contents(cx))),
                    ),
            )
            .children(self.content_results.as_ref().map(|results| {
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .when(results.truncated, |view| {
                        view.child("Content results truncated; refine the search")
                    })
                    .when(results.matches.is_empty(), |view| {
                        view.child("No content matches")
                    })
                    .child(
                        div()
                            .id("content-result-list")
                            .track_scroll(&self.content_scroll)
                            .max_h(px(160.))
                            .overflow_y_scroll()
                            .flex()
                            .flex_col()
                            .children(results.matches.iter().enumerate().map(|(index, hit)| {
                                Button::new(gpui_kit::SharedString::from(format!(
                                    "content-hit-{index}"
                                )))
                                .debug_selector(move || format!("content-hit-{index}"))
                                .ghost()
                                .selected(index == self.content_index)
                                .label(format!(
                                    "{}{}:{}  {}",
                                    if index == self.content_index {
                                        "› "
                                    } else {
                                        ""
                                    },
                                    hit.path,
                                    hit.line_number,
                                    ""
                                ))
                                .child(
                                    gpui_kit::StyledText::new(hit.line_content.clone())
                                        .with_highlights(hit.byte_ranges().into_iter().map(
                                            |range| {
                                                (
                                                    range,
                                                    gpui_kit::HighlightStyle {
                                                        background_color: Some(selection),
                                                        ..Default::default()
                                                    },
                                                )
                                            },
                                        )),
                                )
                                .on_click(cx.listener(
                                    move |app, _, window, cx| {
                                        app.open_content_hit(index, window, cx)
                                    },
                                ))
                            })),
                    )
            }))
            .child(Input::new(&self.file_path))
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap_2()
                    .child(
                        Button::new("list-files")
                            .debug_selector(|| "list-files".into())
                            .label("Browse directory")
                            .secondary()
                            .on_click(cx.listener(|app, _, _, cx| app.list_files(false, cx))),
                    )
                    .child(
                        Button::new("search-files")
                            .debug_selector(|| "search-files".into())
                            .label("Search file paths")
                            .secondary()
                            .on_click(cx.listener(|app, _, _, cx| app.list_files(true, cx))),
                    ),
            )
            .when(self.file_listing_truncated, |view| {
                view.child("Results truncated; refine the path or search")
            })
            .child(
                div()
                    .id("file-entry-list")
                    .max_h(px(160.))
                    .overflow_y_scroll()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .children(self.file_entries.iter().enumerate().map(|(index, entry)| {
                        let path = entry.path.clone();
                        let directory = entry.kind == "directory";
                        Button::new(gpui_kit::SharedString::from(format!("file-entry-{index}")))
                            .debug_selector(move || format!("file-entry-{index}"))
                            .ghost()
                            .label(format!(
                                "{}{}",
                                entry.path,
                                if directory { "/" } else { "" }
                            ))
                            .on_click(cx.listener(move |app, _, window, cx| {
                                app.file_path.update(cx, |input, cx| {
                                    input.set_value(path.clone(), window, cx)
                                });
                                if directory {
                                    app.list_files(false, cx);
                                } else {
                                    app.file_operation(false, window, cx);
                                }
                            }))
                    })),
            )
            .child(
                div()
                    .flex()
                    .gap_2()
                    .child(
                        Button::new("read-file")
                            .debug_selector(|| "read-file".into())
                            .label("Read file")
                            .secondary()
                            .on_click(cx.listener(|app, _, window, cx| {
                                app.file_operation(false, window, cx)
                            })),
                    )
                    .child(
                        Button::new("save-file")
                            .debug_selector(|| "save-file".into())
                            .label("Save file")
                            .primary()
                            .on_click(cx.listener(|app, _, window, cx| {
                                app.file_operation(true, window, cx)
                            })),
                    ),
            )
            .children(self.file_editor.target.as_ref().map(|target| {
                format!(
                    "Loaded: {} / {}{}",
                    target.cwd,
                    target.path,
                    if self.file_editor.writable() {
                        ""
                    } else {
                        " (read-only)"
                    }
                )
            }))
            .child(
                Button::new("discard-file-edits")
                    .debug_selector(|| "discard-file-edits".into())
                    .label("Discard unsaved file edits")
                    .ghost()
                    .on_click(cx.listener(|app, _, window, cx| {
                        if app.file_busy {
                            app.status =
                                "Wait for the pending file operation before discarding".into();
                            cx.notify();
                            return;
                        }
                        let text = app.file_editor.saved.clone();
                        app.file_contents
                            .update(cx, |input, cx| input.set_value(text, window, cx));
                        cx.notify();
                    })),
            )
            .child(Textarea::new(&self.file_contents))
    }

    fn render_changes_panel(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let muted_foreground = cx.theme().muted_foreground;
        let files = self
            .detail
            .thread
            .as_ref()
            .and_then(|detail| {
                detail["checkpoints"]
                    .as_array()
                    .and_then(|list| list.last())
            })
            .and_then(|checkpoint| checkpoint["files"].as_array())
            .cloned()
            .unwrap_or_default();
        div()
            .id("changes-panel")
            .debug_selector(|| "changes-panel".into())
            .flex()
            .flex_col()
            .gap_2()
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .when(files.is_empty(), |view| {
                view.child(
                    div()
                        .text_sm()
                        .text_color(muted_foreground)
                        .child("No checkpointed changes yet"),
                )
            })
            .children(files.iter().map(|file| {
                div()
                    .flex()
                    .justify_between()
                    .gap_2()
                    .text_sm()
                    .child(
                        div()
                            .min_w_0()
                            .truncate()
                            .child(file["path"].as_str().unwrap_or("").to_owned()),
                    )
                    .child(div().text_xs().child(format!(
                        "+{} -{}",
                        file["additions"].as_u64().unwrap_or(0),
                        file["deletions"].as_u64().unwrap_or(0)
                    )))
            }))
    }

    fn render_agents_panel(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
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
