use gpui_kit::assets::IconName;
use gpui_kit::base::Selectable;
use gpui_kit::component::{
    ActiveTheme as _, Icon, Sizable,
    button::{Button, ButtonVariants as _},
    input::Input,
};
use gpui_kit::{
    Context, FontWeight, MouseButton, MouseDownEvent, anchored, deferred, div, prelude::*, px,
};
use serde_json::json;
use t3code_desktop_gpui::{connection, credentials, palette, project_actions, sidebar, thread};

use crate::NativeApp;

impl NativeApp {
    pub(crate) fn render_sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let muted_foreground = cx.theme().muted_foreground;
        let muted = cx.theme().muted;
        let warning = cx.theme().warning;
        let blue = cx.theme().blue;
        let info = cx.theme().info;
        let magenta = cx.theme().magenta;
        let green = cx.theme().green;
        let sidebar_color = cx.theme().sidebar;
        let sidebar_border = cx.theme().sidebar_border;
        let sidebar_foreground = cx.theme().sidebar_foreground;
        let mut projects = Vec::new();
        let now = chrono::Utc::now();
        for project in &self.shell.projects {
            let project_id = project.id.clone();
            let title = project.title.clone();
            let workspace = project.workspace_root.clone();
            let default_selection = project_actions::model_default(
                self.project_settings.as_ref(),
                &project.id,
                project.default_model_selection.as_ref(),
            );
            let project_selected = self.selected_project.as_ref() == Some(&project_id);
            let name = sidebar::project_name(&title, &workspace).to_owned();
            let monogram = name
                .chars()
                .next()
                .map(|letter| letter.to_uppercase().to_string())
                .unwrap_or_default();
            let collapsed = self.collapsed_projects.contains(&project_id);
            let mut group = div().flex().flex_col().gap_1().child(
                div()
                    .flex()
                    .items_center()
                    .gap_1()
                    .child(
                        Button::new(gpui_kit::SharedString::from(format!(
                            "project-toggle-{project_id}"
                        )))
                        .debug_selector({
                            let id = project_id.clone();
                            move || format!("project-toggle-{id}")
                        })
                        .label(if collapsed { "▸" } else { "▾" })
                        .ghost()
                        .small()
                        .tooltip(if collapsed {
                            "Expand project"
                        } else {
                            "Collapse project"
                        })
                        .on_click(cx.listener({
                            let id = project_id.clone();
                            move |app, _, _, cx| {
                                if let Some(index) =
                                    app.collapsed_projects.iter().position(|p| p == &id)
                                {
                                    app.collapsed_projects.remove(index);
                                } else {
                                    app.collapsed_projects.push(id.clone());
                                }
                                cx.notify();
                            }
                        })),
                    )
                    .child(
                        Button::new(gpui_kit::SharedString::from(format!(
                            "project-{project_id}"
                        )))
                        .ghost()
                        .selected(project_selected)
                        .debug_selector({
                            let id = project_id.clone();
                            move || format!("project-{id}")
                        })
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap_2()
                                .w_full()
                                .child(
                                    div()
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .w(px(20.))
                                        .h(px(20.))
                                        .rounded_sm()
                                        .bg(muted)
                                        .text_xs()
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .child(monogram),
                                )
                                .child(
                                    div()
                                        .flex()
                                        .flex_col()
                                        .min_w_0()
                                        .flex_1()
                                        .child(
                                            div()
                                                .w_full()
                                                .truncate()
                                                .font_weight(FontWeight::SEMIBOLD)
                                                .child(name.clone()),
                                        )
                                        .child(
                                            div()
                                                .w_full()
                                                .truncate()
                                                .text_xs()
                                                .text_color(muted_foreground)
                                                .child(workspace.clone()),
                                        ),
                                ),
                        )
                        .on_click(cx.listener(
                            move |app, _, window, cx| {
                                if app.selected_project.as_ref() != Some(&project_id) {
                                    app.model_override = None;
                                    app.provider.update(cx, |input, cx| {
                                        input.set_value(
                                            default_selection["instanceId"]
                                                .as_str()
                                                .unwrap_or_default()
                                                .to_owned(),
                                            window,
                                            cx,
                                        )
                                    });
                                    app.model.update(cx, |input, cx| {
                                        input.set_value(
                                            default_selection["model"]
                                                .as_str()
                                                .unwrap_or_default()
                                                .to_owned(),
                                            window,
                                            cx,
                                        )
                                    });
                                    app.action_task = None;
                                    app.action_busy = false;
                                    app.action_edit_id = None;
                                    app.confirm_action_delete = None;
                                    if let Some(previous) = app.selected_thread.take() {
                                        app.drafts.insert(
                                            previous.clone(),
                                            app.composer.read(cx).value().to_string(),
                                        );
                                        if let Some(store) = &mut app.draft_store {
                                            store.set(
                                                &app.environment_scope,
                                                &previous,
                                                app.composer.read(cx).value().to_string(),
                                            );
                                        }
                                        app.schedule_draft_save(cx);
                                    }
                                    app.detail = thread::ThreadDetail::default();
                                    app.setup_progress = None;
                                    app.confirm_delete = None;
                                    app.composer
                                        .update(cx, |input, cx| input.set_value("", window, cx));
                                    if let Some(sender) = &app.commands {
                                        let _ = sender.try_send(connection::Command::Unsubscribe(
                                            "worktree-setup".into(),
                                        ));
                                        let _ = sender
                                            .try_send(connection::Command::SelectThread(None));
                                    }
                                }
                                app.selected_project = Some(project_id.clone());
                                app.worktree_task = None;
                                app.worktree_busy = false;
                                app.worktree_epoch += 1;
                                app.refs.clear();
                                app.worktree_outcome = None;
                                app.new_thread_checkout = None;
                                app.confirm_worktree_removal = None;
                                app.confirm_project_removal = None;
                                app.file_entries.clear();
                                app.file_listing_task = None;
                                app.file_listing_epoch += 1;
                                app.content_results = None;
                                app.title.update(cx, |input, cx| {
                                    input.set_value(title.clone(), window, cx)
                                });
                                app.workspace.update(cx, |input, cx| {
                                    input.set_value(workspace.clone(), window, cx)
                                });
                                cx.notify();
                            },
                        )),
                    ),
            );
            if !collapsed {
                for draft in self.pending_threads.iter().filter(|draft| {
                    draft.thread.project_id == project.id
                        && !self
                            .shell
                            .threads
                            .iter()
                            .any(|thread| thread.id == draft.thread.id)
                }) {
                    let id = draft.thread.id.clone();
                    group = group.child(
                        div().pl_4().child(
                            Button::new(gpui_kit::SharedString::from(format!("draft-{id}")))
                                .ghost()
                                .selected(self.selected_thread.as_ref() == Some(&id))
                                .child(
                                    div()
                                        .flex()
                                        .flex_col()
                                        .min_w_0()
                                        .w_full()
                                        .child(
                                            div()
                                                .w_full()
                                                .truncate()
                                                .child(draft.thread.title.clone()),
                                        )
                                        .child(div().text_xs().text_color(warning).child("unsent")),
                                )
                                .on_click(cx.listener(move |app, _, window, cx| {
                                    app.select_thread(&id, window, cx)
                                })),
                        ),
                    );
                }
                let threads = if self.sidebar_view == sidebar::View::Archived {
                    &self.archived_shell.threads
                } else {
                    &self.shell.threads
                };
                let group_defs = sidebar::grouped(threads, &project.id, self.sidebar_view, now);
                for (group_key, group_label, group_threads) in group_defs {
                    if group_threads.is_empty() {
                        continue;
                    }
                    // Collapse state is per project: folding one project's group leaves the
                    // rest alone.
                    let group_key = sidebar::group_key(&project.id, group_key);
                    let group_collapsed = self.collapsed_groups.contains(&group_key);
                    group = group.child(
                        div().pl_4().child(
                            Button::new(gpui_kit::SharedString::from(format!(
                                "group-toggle-{group_key}"
                            )))
                            .debug_selector({
                                let key = group_key.clone();
                                move || format!("group-toggle-{key}")
                            })
                            .label(format!(
                                "{} {} ({})",
                                if group_collapsed { "▸" } else { "▾" },
                                group_label,
                                group_threads.len()
                            ))
                            .ghost()
                            .small()
                            .on_click(cx.listener(
                                move |app, _, _, cx| {
                                    let key = group_key.clone();
                                    if let Some(index) =
                                        app.collapsed_groups.iter().position(|entry| entry == &key)
                                    {
                                        app.collapsed_groups.remove(index);
                                    } else {
                                        app.collapsed_groups.push(key);
                                    }
                                    cx.notify();
                                },
                            )),
                        ),
                    );
                    if group_collapsed {
                        continue;
                    }
                    for thread in group_threads {
                        let id = thread.id.clone();
                        let status = sidebar::primary_status(thread);
                        let has_pull_request = thread.linked_pull_request.is_some();
                        let age = sidebar::last_activity(thread)
                            .map(|at| sidebar::relative_time(at, now));
                        let meta = sidebar::thread_meta(thread);
                        group = group.child(
                            div().pl_4().child(
                                Button::new(gpui_kit::SharedString::from(format!("thread-{id}")))
                                    .debug_selector({
                                        let id = id.clone();
                                        move || format!("thread-{id}")
                                    })
                                    .ghost()
                                    .selected(self.selected_thread.as_ref() == Some(&id))
                                    .child(
                                        div()
                                            .flex()
                                            .items_center()
                                            .gap_2()
                                            .w_full()
                                            .child(
                                                div()
                                                    .flex()
                                                    .flex_col()
                                                    .min_w_0()
                                                    .flex_1()
                                                    .child(
                                                        div()
                                                            .w_full()
                                                            .truncate()
                                                            .child(thread.title.clone()),
                                                    )
                                                    .when_some(meta, |view, meta| {
                                                        view.child(
                                                            div()
                                                                .w_full()
                                                                .truncate()
                                                                .text_xs()
                                                                .text_color(muted_foreground)
                                                                .child(meta),
                                                        )
                                                    }),
                                            )
                                            .child(
                                                div()
                                                    .flex()
                                                    .flex_shrink_0()
                                                    .items_center()
                                                    .gap_1()
                                                    .text_xs()
                                                    .when_some(status, |view, status| {
                                                        let color = match status {
                                                            sidebar::Status::Approval => warning,
                                                            sidebar::Status::Input => info,
                                                            sidebar::Status::Working => blue,
                                                            sidebar::Status::Plan => magenta,
                                                        };
                                                        view.child(
                                                            div()
                                                                .w(px(6.))
                                                                .h(px(6.))
                                                                .rounded_full()
                                                                .bg(color),
                                                        )
                                                        .child(
                                                            div()
                                                                .text_color(color)
                                                                .child(status.label()),
                                                        )
                                                    })
                                                    .when(has_pull_request, |view| {
                                                        view.child(
                                                            Icon::new(IconName::GitPullRequest)
                                                                .xsmall()
                                                                .text_color(green),
                                                        )
                                                    })
                                                    .when_some(age, |view, age| {
                                                        view.child(
                                                            div()
                                                                .text_color(muted_foreground)
                                                                .child(age),
                                                        )
                                                    }),
                                            ),
                                    )
                                    .on_mouse_down(
                                        MouseButton::Right,
                                        cx.listener({
                                            let id = id.clone();
                                            move |app, event: &MouseDownEvent, _, cx| {
                                                app.close_thread_menu();
                                                app.thread_menu_open = true;
                                                app.thread_menu_row =
                                                    Some((id.clone(), event.position));
                                                cx.notify();
                                            }
                                        }),
                                    )
                                    .on_click(cx.listener(move |app, _, window, cx| {
                                        app.select_thread(&id, window, cx)
                                    })),
                            ),
                        );
                    }
                }
            }
            projects.push(group);
        }
        let mut search_results = div().flex().flex_col().gap_1();
        for result in &self.search_results {
            if let Some(id) = result["threadId"].as_str() {
                let id = id.to_owned();
                search_results = search_results.child(
                    Button::new(gpui_kit::SharedString::from(format!("search-{id}")))
                        .label(result["snippet"].as_str().unwrap_or("Match").to_owned())
                        .ghost()
                        .on_click(cx.listener(move |app, _, window, cx| {
                            app.select_thread(&id, window, cx)
                        })),
                );
            }
        }
        let row_menu = self
            .thread_menu_row
            .as_ref()
            .filter(|_| self.thread_menu_open)
            .map(|(_, at)| {
                deferred(
                    anchored()
                        .position(*at)
                        .snap_to_window()
                        .child(self.render_thread_menu(cx)),
                )
            });
        div()
            .id("sidebar")
            .flex()
            .flex_col()
            .w(px(288.))
            .flex_shrink_0()
            .h_full()
            .bg(sidebar_color)
            .text_color(sidebar_foreground)
            .border_r_1()
            .border_color(sidebar_border)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .p_2()
                    .child(div().flex_1().min_w_0().child(Input::new(&self.search)))
                    .child(
                        Button::new("search")
                            .label("Search")
                            .ghost()
                            .small()
                            .on_click(cx.listener(|app, _, _, cx| app.search_threads(cx))),
                    )
                    .child(
                        Button::new("new-thread")
                            .label("New")
                            .primary()
                            .small()
                            .on_click(cx.listener(|app, _, window, cx| {
                                app.execute_command(palette::Command::NewThread, window, cx)
                            })),
                    ),
            )
            .child(
                div().flex().flex_wrap().gap_1().px_2().pb_2().children(
                    [
                        ("active-view", "Active", sidebar::View::Active),
                        ("settled-view", "Settled", sidebar::View::Settled),
                        ("snoozed-view", "Snoozed", sidebar::View::Snoozed),
                        ("archived-view", "Archived", sidebar::View::Archived),
                    ]
                    .into_iter()
                    .map(|(id, label, view)| {
                        Button::new(id)
                            .debug_selector(move || id.into())
                            .label(label)
                            .ghost()
                            .small()
                            .selected(self.sidebar_view == view)
                            .on_click(cx.listener(move |app, _, _, cx| {
                                app.sidebar_view = view;
                                if view == sidebar::View::Archived {
                                    app.perform(
                                        "orchestration.getArchivedShellSnapshot",
                                        json!({}),
                                        None,
                                        cx,
                                    );
                                }
                                cx.notify();
                            }))
                    }),
                ),
            )
            .child(
                div()
                    .id("sidebar-list")
                    .flex()
                    .flex_col()
                    .gap_1()
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .px_2()
                    .pb_2()
                    .child(search_results)
                    .when(self.shell.projects.is_empty(), |view| {
                        view.child(
                            div()
                                .p_3()
                                .text_sm()
                                .text_color(muted_foreground)
                                .child("No projects yet"),
                        )
                    })
                    .children(projects),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .p_2()
                    .border_t_1()
                    .border_color(sidebar_border)
                    .child(
                        Button::new("connection-toggle")
                            .ghost()
                            .selected(self.connection_expanded)
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_2()
                                    .w_full()
                                    .child(div().w(px(8.)).h(px(8.)).rounded_full().bg(
                                        if self.commands.is_some() {
                                            green
                                        } else {
                                            muted_foreground
                                        },
                                    ))
                                    .child(
                                        div()
                                            .flex_1()
                                            .min_w_0()
                                            .truncate()
                                            .child(self.status.clone()),
                                    )
                                    .child(div().text_color(muted_foreground).child(
                                        if self.connection_expanded {
                                            "▾"
                                        } else {
                                            "▸"
                                        },
                                    )),
                            )
                            .on_click(cx.listener(|app, _, _, cx| {
                                app.connection_expanded = !app.connection_expanded;
                                cx.notify();
                            })),
                    )
                    .when(self.connection_expanded, |view| {
                        view.child(Input::new(&self.server))
                            .child(Input::new(&self.pairing))
                            .child(
                                div()
                                    .flex()
                                    .flex_wrap()
                                    .gap_2()
                                    .child(
                                        Button::new("pair")
                                            .debug_selector(|| "pair".into())
                                            .label("Pair and connect")
                                            .primary()
                                            .on_click(cx.listener(|app, _, window, cx| {
                                                app.pair(window, cx)
                                            })),
                                    )
                                    .child(
                                        Button::new("remember-credential")
                                            .debug_selector(|| "remember-credential".into())
                                            .label("Remember in Keychain")
                                            .ghost()
                                            .on_click(cx.listener(|app, _, _, cx| {
                                                app.remember_credential(cx)
                                            })),
                                    )
                                    .child(
                                        Button::new("load-credential")
                                            .debug_selector(|| "load-credential".into())
                                            .label("Connect remembered")
                                            .ghost()
                                            .on_click(cx.listener(|app, _, _, cx| {
                                                app.credential_operation(
                                                    credentials::Operation::Load,
                                                    cx,
                                                )
                                            })),
                                    )
                                    .child(
                                        Button::new("forget-credential")
                                            .debug_selector(|| "forget-credential".into())
                                            .label("Forget credential")
                                            .ghost()
                                            .on_click(cx.listener(|app, _, _, cx| {
                                                app.forget_credential(cx)
                                            })),
                                    )
                                    .child(
                                        Button::new("connect")
                                            .label("Connect / retry")
                                            .secondary()
                                            .on_click(cx.listener(|app, _, _, cx| app.connect(cx))),
                                    )
                                    .child(
                                        Button::new("disconnect")
                                            .label("Disconnect")
                                            .ghost()
                                            .on_click(
                                                cx.listener(|app, _, _, cx| app.disconnect(cx)),
                                            ),
                                    ),
                            )
                    }),
            )
            .children(row_menu)
    }
}
