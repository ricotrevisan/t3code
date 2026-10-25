use gpui_kit::assets::IconName;
use gpui_kit::base::{Disableable, Selectable};
use gpui_kit::component::{
    ActiveTheme as _, Icon, Sizable,
    button::{Button, ButtonVariants as _},
    input::Input,
    list::ListItem,
};
use gpui_kit::{
    Context, FontWeight, MouseButton, MouseDownEvent, SharedString, div, prelude::*, px,
};
use serde_json::json;
use t3code_desktop_gpui::{connection, palette, project_actions, sidebar, thread};

use super::list_row;
use crate::{NativeApp, THREAD_ACTIONS};

struct SidebarDragPreview(String);
impl gpui_kit::Render for SidebarDragPreview {
    fn render(&mut self, _: &mut gpui_kit::Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .p_2()
            .rounded_md()
            .bg(cx.theme().popover)
            .border_1()
            .border_color(cx.theme().border)
            .child(self.0.clone())
    }
}

impl NativeApp {
    /// Shared row/header menu. Dynamic snooze timestamps are captured with the
    /// row target, so a click never reads a different selected conversation.
    pub(crate) fn render_sidebar_thread_menu(
        &self,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let target = self.thread_menu_target();
        let thread = target.as_ref().and_then(|id| {
            self.shell
                .threads
                .iter()
                .chain(&self.archived_shell.threads)
                .find(|thread| &thread.id == id)
        });
        let now = chrono::Utc::now();
        // Older clients already supported the baseline commands before capability
        // discovery. Once config is received, its flags are authoritative.
        let legacy = json!({"threadSettlement":true,"threadPinning":true,"threadSnooze":true});
        let capabilities = if self.sidebar_state.capabilities.is_null() {
            &legacy
        } else {
            &self.sidebar_state.capabilities
        };
        let mut rows: Vec<(String, String, String)> = THREAD_ACTIONS
            .iter()
            .filter(|(id, _, _)| {
                sidebar::thread_action_policy(thread, id, now, capabilities)
                    && !(matches!(*id, "settle" | "snooze")
                        && target
                            .as_ref()
                            .is_some_and(|target| self.queues.has_queued(target)))
            })
            .map(|(id, label, kind)| ((*id).into(), (*label).into(), (*kind).into()))
            .collect();
        if thread.is_none() && target.is_some() {
            rows.push((
                "discard".into(),
                "Discard unsent thread".into(),
                "discard".into(),
            ));
        }
        if let Some(thread) = thread {
            if capabilities["threadCleanupReview"].as_bool() == Some(true)
                && thread.settled_override.as_deref() == Some("settled")
            {
                rows.push((
                    "cleanup-review".into(),
                    "Review cleanup…".into(),
                    "cleanup-review".into(),
                ));
            }
            if capabilities["threadAutoSettleOptOut"].as_bool() == Some(true) {
                let disabled = thread.auto_settle_disabled_at.is_some();
                rows.push((
                    if disabled {
                        "auto-settle-enable"
                    } else {
                        "auto-settle-disable"
                    }
                    .into(),
                    if disabled {
                        "Enable auto-settle"
                    } else {
                        "Disable auto-settle"
                    }
                    .into(),
                    if disabled {
                        "auto-settle-enable"
                    } else {
                        "auto-settle-disable"
                    }
                    .into(),
                ));
            }
            let pinned = thread.pinned_at.is_some();
            let reorder_cap = if pinned {
                "threadPinReorder"
            } else {
                "threadActiveReorder"
            };
            if sidebar::belongs(thread, sidebar::View::Active, now)
                && capabilities[reorder_cap].as_bool() == Some(true)
                && !(self.sidebar_state.order_busy
                    && self.sidebar_state.order_generation == self.generation)
            {
                let groups = self.sidebar_state.grouped(
                    &self.shell.threads,
                    Some(self.shell.revision),
                    if self.sidebar_state.priority {
                        self.sidebar_state.project_scope.as_deref().unwrap_or("")
                    } else {
                        &thread.project_id
                    },
                    sidebar::View::Active,
                    now,
                    &self.environment_scope,
                );
                let order = &groups[if pinned { 0 } else { 1 }].2;
                for (id, label, forward) in [
                    ("move-up", "Move up", false),
                    ("move-down", "Move down", true),
                ] {
                    if sidebar::move_assignments(
                        order,
                        &self.shell.threads,
                        &thread.id,
                        forward,
                        pinned,
                    )
                    .is_some()
                    {
                        rows.insert(0, (id.into(), label.into(), id.into()));
                    }
                }
            }
            for (id, label) in [
                ("new-thread-on-branch", "New thread on this branch"),
                ("regenerate-title", "Regenerate title"),
                ("mark-unread", "Mark unread"),
                ("project-settings", "Project settings"),
                ("copy-path", "Copy path"),
                ("copy-branch", "Copy branch"),
                ("copy-thread-id", "Copy thread ID"),
            ] {
                if sidebar::thread_action_policy(Some(thread), id, now, capabilities) {
                    rows.push((id.into(), label.into(), id.into()));
                }
            }
            if self.thread_menu_row.is_some() {
                rows.push((
                    "filter-project".into(),
                    if self.sidebar_state.project_scope.as_ref() == Some(&thread.project_id) {
                        "Show all projects"
                    } else {
                        "Filter by this project"
                    }
                    .into(),
                    "filter-project".into(),
                ));
            }
            if sidebar::thread_action_policy(Some(thread), "snooze", now, capabilities)
                && !self.queues.has_queued(&thread.id)
            {
                for (index, (label, until)) in sidebar::snooze_presets(chrono::Local::now())
                    .into_iter()
                    .enumerate()
                {
                    rows.push((
                        format!("snooze-preset-{index}"),
                        format!(
                            "{label} ({})",
                            until.with_timezone(&chrono::Local).format("%a %H:%M")
                        ),
                        format!(
                            "snooze-until:{}",
                            until.to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
                        ),
                    ));
                }
                rows.push((
                    "snooze-custom".into(),
                    "Custom snooze…".into(),
                    "snooze-custom".into(),
                ));
            }
        }
        if self.sidebar_state.selection_environment == self.environment_scope
            && self.sidebar_state.selection.len() > 1
            && target
                .as_ref()
                .is_some_and(|target| self.sidebar_state.selection.contains(target))
        {
            rows.clear();
            if !(self.sidebar_state.bulk_busy
                && self.sidebar_state.bulk_generation == self.generation)
            {
                for (action, label, kind) in [
                    (
                        "settle",
                        "Settle applicable selected threads",
                        "thread.settle",
                    ),
                    (
                        "unsettle",
                        "Unsettle selected settled threads",
                        "thread.unsettle",
                    ),
                    (
                        "unsnooze",
                        "Wake selected snoozed threads",
                        "thread.unsnooze",
                    ),
                    ("pin", "Pin selected unpinned threads", "thread.pin"),
                    ("unpin", "Unpin selected pinned threads", "thread.unpin"),
                    (
                        "archive",
                        "Archive applicable selected threads",
                        "thread.archive",
                    ),
                    (
                        "unarchive",
                        "Restore selected archived threads",
                        "thread.unarchive",
                    ),
                    (
                        "regenerate-title",
                        "Regenerate selected titles",
                        "thread.meta.update",
                    ),
                    (
                        "delete",
                        "Delete selected threads permanently (confirm)",
                        "thread.delete",
                    ),
                ] {
                    let applicable_count = self
                        .sidebar_state
                        .selection
                        .iter()
                        .filter(|id| {
                            self.shell
                                .threads
                                .iter()
                                .chain(&self.archived_shell.threads)
                                .find(|thread| &thread.id == *id)
                                .is_some_and(|thread| {
                                    sidebar::thread_action_policy(
                                        Some(thread),
                                        action,
                                        now,
                                        capabilities,
                                    ) && !(matches!(action, "settle" | "snooze")
                                        && self.queues.has_queued(id))
                                })
                        })
                        .count();
                    if applicable_count > 0 {
                        rows.push((
                            format!("bulk-{action}"),
                            format!(
                                "{label} ({applicable_count}/{})",
                                self.sidebar_state.selection.len()
                            ),
                            format!("bulk:{action}:{kind}"),
                        ));
                    }
                }
                let can_snooze_all = self.sidebar_state.selection.iter().all(|id| {
                    self.shell
                        .threads
                        .iter()
                        .find(|thread| &thread.id == id)
                        .is_some_and(|thread| {
                            sidebar::thread_action_policy(Some(thread), "snooze", now, capabilities)
                                && !self.queues.has_queued(id)
                        })
                });
                if can_snooze_all {
                    for (index, (label, until)) in sidebar::snooze_presets(chrono::Local::now())
                        .into_iter()
                        .enumerate()
                    {
                        rows.push((
                            format!("bulk-snooze-preset-{index}"),
                            format!("Snooze selected: {label}"),
                            format!(
                                "bulk-snooze-until:{}",
                                until.to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
                            ),
                        ));
                    }
                    rows.push((
                        "bulk-snooze-custom".into(),
                        "Custom snooze selected…".into(),
                        "bulk-snooze-custom".into(),
                    ));
                }
                rows.push((
                    "bulk-mark-unread".into(),
                    "Mark selected unread".into(),
                    "bulk-mark-unread".into(),
                ));
            }
        }
        let single_review = self.sidebar_state.single_delete_review.as_ref().filter(
            |(generation, environment, ids)| {
                *generation == self.generation
                    && environment == &self.environment_scope
                    && ids.first() == self.confirm_delete.as_ref()
                    && ids.first() == target.as_ref()
            },
        );
        let single = single_review.is_some();
        let deletion_review = single_review.or_else(|| {
            self.sidebar_state
                .bulk_delete_review
                .as_ref()
                .filter(|(generation, environment, _)| {
                    *generation == self.generation && environment == &self.environment_scope
                })
        });
        if deletion_review.is_some() {
            rows.clear();
        }
        div()
            .id("thread-menu-popover")
            .occlude()
            .mt_1()
            .w(px(260.))
            .max_h(px(620.))
            .flex()
            .flex_col()
            .overflow_y_scroll()
            .p_2()
            .rounded_md()
            .bg(cx.theme().popover)
            .border_1()
            .border_color(cx.theme().border)
            .shadow_lg()
            .when_some(deletion_review, |menu, (_, _, ids)| {
                menu.child(
                    div()
                        .id("bulk-delete-review")
                        .debug_selector(|| "bulk-delete-review".into())
                        .flex()
                        .flex_col()
                        .gap_2()
                        .child(if single {
                            "Delete this thread?".into()
                        } else {
                            format!("Delete {} threads?", ids.len())
                        })
                        .child(
                            "This permanently clears conversation history. This cannot be undone.",
                        )
                        .children(ids.iter().map(|id| {
                            let title = self
                                .shell
                                .threads
                                .iter()
                                .chain(&self.archived_shell.threads)
                                .find(|thread| &thread.id == id)
                                .map(|thread| thread.title.as_str())
                                .unwrap_or("Unavailable thread");
                            div().child(format!("{title} · {id}"))
                        }))
                        .child(
                            Button::new("bulk-delete")
                                .debug_selector(|| "bulk-delete".into())
                                .danger()
                                .label(format!("Delete {} threads permanently", ids.len()))
                                .disabled(
                                    !self.connection_ready()
                                        || (self.sidebar_state.bulk_busy
                                            && self.sidebar_state.bulk_generation
                                                == self.generation),
                                )
                                .on_click(cx.listener(move |app, _, window, cx| {
                                    app.sidebar_menu_action(
                                        "",
                                        if single {
                                            "single-delete-confirm"
                                        } else {
                                            "bulk-delete-confirm"
                                        },
                                        window,
                                        cx,
                                    );
                                })),
                        )
                        .child(
                            Button::new("bulk-delete-cancel")
                                .debug_selector(|| "bulk-delete-cancel".into())
                                .ghost()
                                .label("Cancel")
                                .on_click(cx.listener(|app, _, window, cx| {
                                    app.sidebar_menu_action("", "bulk-delete-cancel", window, cx);
                                })),
                        ),
                )
            })
            .children(
                rows.into_iter()
                    .filter(|(_, _, kind)| {
                        self.connection_ready()
                            || (matches!(kind.as_str(), "rename" | "discard")
                                && target.as_ref().is_some_and(|id| {
                                    self.pending_threads
                                        .iter()
                                        .any(|draft| &draft.thread.id == id)
                                }))
                    })
                    .map(|(id, label, kind)| {
                        let label = if self.confirm_delete == target
                            && target.is_some()
                            && matches!(id.as_str(), "delete" | "discard")
                        {
                            "Confirm removal".to_owned()
                        } else {
                            label
                        };
                        let target = target.clone();
                        list_row(SharedString::from(id), None, label, false).on_click(cx.listener(
                            move |app, _, window, cx| {
                                if let Some(target) = target.as_deref() {
                                    app.sidebar_menu_action(target, &kind, window, cx);
                                }
                            },
                        ))
                    }),
            )
    }

    pub(crate) fn render_sidebar_row(
        &self,
        thread: &t3code_desktop_gpui::shell::Thread,
        now: chrono::DateTime<chrono::Utc>,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let muted_foreground = cx.theme().muted_foreground;
        let warning = cx.theme().warning;
        let info = cx.theme().info;
        let blue = cx.theme().blue;
        let magenta = cx.theme().magenta;
        let green = cx.theme().green;
        let id = thread.id.clone();
        let prepared =
            self.sidebar_state
                .prepared_row(thread, self.queues.has_queued(&thread.id), now);
        let offline = matches!(
            self.connection_phase,
            connection::Phase::Offline
                | connection::Phase::Reconnecting { .. }
                | connection::Phase::Error(_)
        );
        let status = if offline { None } else { prepared.status };
        let has_pull_request = thread.linked_pull_request.is_some();
        let age = prepared.age.clone();
        let woke = self
            .sidebar_state
            .woke(&self.environment_scope, thread, now);
        let unread = self.sidebar_state.unread(&self.environment_scope, thread);
        let meta = if self.sidebar_state.priority {
            let project = self
                .shell
                .projects
                .iter()
                .find(|project| project.id == thread.project_id)
                .map(|project| project.title.as_str())
                .unwrap_or("Project");
            Some(match &prepared.meta {
                Some(meta) => format!("{project} · {meta}"),
                None => project.to_owned(),
            })
        } else {
            prepared.meta.clone()
        };
        let row_section = if sidebar::snoozed(thread, now) {
            "snoozed"
        } else if thread.settled_override.as_deref() == Some("settled") {
            "settled"
        } else if thread.pinned_at.is_some() {
            "pinned"
        } else {
            "active"
        };
        div()
            .id(SharedString::from(format!("sidebar-drag-{id}")))
            .relative()
            .w_full()
            .min_w_0()
            .flex_shrink_0()
            .when(self.selected_thread.as_ref() == Some(&id), |row| {
                let selected_bounds = self.sidebar_state.selected_bounds.clone();
                row.child(
                    gpui_kit::canvas(
                        move |bounds, _, _| *selected_bounds.borrow_mut() = Some(bounds),
                        |_, _, _, _| {},
                    )
                    .absolute()
                    .size_full(),
                )
            })
            .pl_4()
            .when(
                self.sidebar_state.priority && self.connection_ready(),
                |row| {
                    row.on_drag(
                        sidebar::Drag {
                            id: id.clone(),
                            title: thread.title.clone(),
                            environment: self.environment_scope.clone(),
                            generation: self.generation,
                        },
                        |drag, _, _, cx| cx.new(|_| SidebarDragPreview(drag.title.clone())),
                    )
                },
            )
            .when(
                self.sidebar_state.priority && matches!(row_section, "pinned" | "active"),
                |row| {
                    let target = id.clone();
                    let preview_target = target.clone();
                    let entity = cx.entity().downgrade();
                    row.drag_over::<sidebar::Drag>(move |style, drag, _, cx| {
                        if entity.upgrade().is_some_and(|entity| {
                            let app = entity.read(cx);
                            app.environment_scope == drag.environment
                                && app.generation == drag.generation
                                && app
                                    .sidebar_drop_plan(&drag.id, row_section, Some(&preview_target))
                                    .is_some()
                        }) {
                            style.border_t_2().border_color(cx.theme().blue)
                        } else {
                            style
                        }
                    })
                    .on_drop(cx.listener(
                        move |app, drag: &sidebar::Drag, _, cx| {
                            app.drop_sidebar_thread(drag, row_section, Some(&target), cx);
                        },
                    ))
                },
            )
            .child(
                Button::new(gpui_kit::SharedString::from(format!("thread-{id}")))
                    .debug_selector({
                        let id = id.clone();
                        move || format!("thread-{id}")
                    })
                    .ghost()
                    // This domain row has two lines; the standard one-line
                    // Button height cannot contain its scaled typography.
                    .h_auto()
                    .w_full()
                    .selected(
                        self.selected_thread.as_ref() == Some(&id)
                            || (self.sidebar_state.selection_environment == self.environment_scope
                                && self.sidebar_state.selection.contains(&id)),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .w_full()
                            .py_1()
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
                                            .debug_selector({
                                                let id = id.clone();
                                                move || format!("sidebar-title-{id}")
                                            })
                                            .when(unread, |view| view.child("● "))
                                            .child(thread.title.clone()),
                                    )
                                    .when_some(meta, |view, meta| {
                                        view.child(
                                            div()
                                                .w_full()
                                                .truncate()
                                                .text_xs()
                                                .text_color(muted_foreground)
                                                .debug_selector({
                                                    let id = id.clone();
                                                    move || format!("sidebar-meta-{id}")
                                                })
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
                                    .when(offline, |view| {
                                        view.child(div().text_color(warning).child("Offline"))
                                    })
                                    .when_some(status, |view, status| {
                                        let color = match status {
                                            sidebar::Status::Approval => warning,
                                            sidebar::Status::Input => info,
                                            sidebar::Status::Working => blue,
                                            sidebar::Status::Plan => magenta,
                                            sidebar::Status::Error => warning,
                                        };
                                        view.child(
                                            div().w(px(6.)).h(px(6.)).rounded_full().bg(color),
                                        )
                                        .child(div().text_color(color).child(status.label()))
                                    })
                                    .when(woke, |view| {
                                        let wake_id = thread.id.clone();
                                        let selector = format!("dismiss-woke-{}", thread.id);
                                        view.child(Button::new(SharedString::from(selector.clone()))
                                            .debug_selector(move || selector.clone())
                                            .label("Woke")
                                            .tooltip("Dismiss Woke notification without marking unread history as read")
                                            .ghost().xsmall()
                                            .on_click(cx.listener(move |app, _, _, cx| {
                                                cx.stop_propagation();
                                                if let Some(thread) = app.shell.threads.iter().find(|thread| thread.id == wake_id)
                                                    && let Some(at) = sidebar::woke_at(thread, chrono::Utc::now()) {
                                                    app.sidebar_state.woke_acknowledged.insert((app.environment_scope.clone(), wake_id.clone()), at);
                                                    if let Err(error) = app.sidebar_state.save_local() { app.report_error(error, cx); }
                                                }
                                                cx.notify();
                                            })))
                                    })
                                    .when(has_pull_request, |view| {
                                        view.child(
                                            Icon::new(IconName::GitPullRequest)
                                                .xsmall()
                                                .text_color(green),
                                        )
                                    })
                                    .when_some(age, |view, age| {
                                        view.child(div().text_color(muted_foreground).child(age))
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
                                app.thread_menu_row = Some((id.clone(), event.position));
                                cx.notify();
                            }
                        }),
                    )
                    .on_click(
                        cx.listener(move |app, event: &gpui_kit::ClickEvent, window, cx| {
                            app.sidebar_row_click(&id, event.modifiers(), window, cx)
                        }),
                    ),
            )
    }

    pub(crate) fn render_sidebar(&mut self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        self.reconcile_sidebar_selection();
        if self.sidebar_state.cleanup.busy
            && self.sidebar_state.cleanup.generation != self.generation
        {
            self.sidebar_state.cleanup.busy = false;
            self.sidebar_state.cleanup.error =
                Some("Connection changed; review again before running cleanup".into());
            self.sidebar_state.cleanup.review = None;
            self.sidebar_state.cleanup.selected.clear();
        }
        self.sidebar_rows.retain(
            &self.environment_scope,
            if self.sidebar_view == sidebar::View::Archived {
                &self.archived_shell.threads
            } else {
                &self.shell.threads
            },
        );
        self.sidebar_state.retain_rows(
            if self.sidebar_view == sidebar::View::Archived {
                &self.archived_shell.threads
            } else {
                &self.shell.threads
            },
            &self.environment_scope,
        );
        let muted_foreground = cx.theme().muted_foreground;
        let muted = cx.theme().muted;
        let warning = cx.theme().warning;
        let sidebar_color = cx.theme().sidebar;
        let sidebar_border = cx.theme().sidebar_border;
        let sidebar_foreground = cx.theme().sidebar_foreground;
        let mut projects = Vec::new();
        let now = chrono::Utc::now();
        for project in self.shell.projects.iter().filter(|_| {
            !self.sidebar_state.priority || self.sidebar_view == sidebar::View::Archived
        }) {
            if self
                .sidebar_state
                .project_scope
                .as_ref()
                .is_some_and(|scope| scope != &project.id)
            {
                continue;
            }
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
                        .accessibility_label(name.clone())
                        .tooltip(workspace.clone())
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
                                        .debug_selector({
                                            let id = project_id.clone();
                                            move || format!("project-{id}-name")
                                        })
                                        .min_w_0()
                                        .flex_1()
                                        .truncate()
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .child(name.clone()),
                                ),
                        )
                        .on_click(cx.listener(
                            move |app, _, window, cx| {
                                if app.selected_project.as_ref() == Some(&project_id)
                                    || !app.can_change_checkout(cx)
                                {
                                    return;
                                }
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
                                app.reset_checkout_context(window, cx);
                                app.new_thread_checkout = None;
                                app.title.update(cx, |input, cx| {
                                    input.set_value(title.clone(), window, cx)
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
                                .debug_selector({
                                    let id = id.clone();
                                    move || format!("draft-{id}")
                                })
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
                let sequence = if self.sidebar_view == sidebar::View::Archived {
                    Some(self.archived_shell.revision)
                } else {
                    Some(self.shell.revision)
                };
                let group_defs = self.sidebar_state.grouped(
                    threads,
                    sequence,
                    &project.id,
                    self.sidebar_view,
                    now,
                    &self.environment_scope,
                );
                for (group_key, group_label, group_threads) in group_defs {
                    if group_threads.is_empty() {
                        continue;
                    }
                    // Collapse state is per project: folding one project's group leaves the
                    // rest alone.
                    let group_key = sidebar::group_key(&project.id, group_key);
                    let group_collapsed = self.collapsed_groups.contains(&group_key);
                    // The chevron hangs in the gutter so the label starts at the
                    // thread titles' inset.
                    group = group.child(
                        ListItem::new(SharedString::from(format!("group-toggle-{group_key}")))
                            .debug_selector({
                                let key = group_key.clone();
                                move || format!("group-toggle-{key}")
                            })
                            .aria_expanded(!group_collapsed)
                            .px_2()
                            .rounded_md()
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_2()
                                    .min_w_0()
                                    .text_color(muted_foreground)
                                    .child(
                                        Icon::new(if group_collapsed {
                                            IconName::ChevronRight
                                        } else {
                                            IconName::ChevronDown
                                        })
                                        .xsmall(),
                                    )
                                    .child(
                                        div()
                                            .min_w_0()
                                            .truncate()
                                            .text_xs()
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .child(format!(
                                                "{} ({})",
                                                group_label.to_uppercase(),
                                                group_threads.len()
                                            )),
                                    ),
                            )
                            .on_click(cx.listener(move |app, _, _, cx| {
                                let key = group_key.clone();
                                if let Some(index) =
                                    app.collapsed_groups.iter().position(|entry| entry == &key)
                                {
                                    app.collapsed_groups.remove(index);
                                } else {
                                    app.collapsed_groups.push(key);
                                }
                                cx.notify();
                            })),
                    );
                    if group_collapsed {
                        continue;
                    }
                    for thread in group_threads {
                        group = group.child(self.cached_sidebar_row(thread, now, cx));
                    }
                }
            }
            projects.push(group.into_any_element());
        }
        if self.sidebar_state.priority && self.sidebar_view != sidebar::View::Archived {
            for (key, label, rows) in self.sidebar_state.priority_groups(
                &self.shell.threads,
                self.shell.revision,
                now,
                &self.environment_scope,
            ) {
                let collapse_key = sidebar::group_key("", key);
                let collapsed = self.collapsed_groups.contains(&collapse_key);
                let mut shelf = div()
                    .id(SharedString::from(format!("drop-shelf-{key}")))
                    .flex_shrink_0()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .when(matches!(key, "pinned" | "active" | "settled"), |shelf| {
                        let entity = cx.entity().downgrade();
                        shelf
                            .drag_over::<sidebar::Drag>(move |style, drag, _, cx| {
                                if entity.upgrade().is_some_and(|entity| {
                                    let app = entity.read(cx);
                                    app.environment_scope == drag.environment
                                        && app.generation == drag.generation
                                        && app.sidebar_drop_plan(&drag.id, key, None).is_some()
                                }) {
                                    style
                                        .bg(cx.theme().muted)
                                        .border_1()
                                        .border_color(cx.theme().blue)
                                } else {
                                    style
                                }
                            })
                            .on_drop(cx.listener(move |app, drag: &sidebar::Drag, _, cx| {
                                app.drop_sidebar_thread(drag, key, None, cx);
                            }))
                    })
                    .child(
                        super::list_row(
                            format!("priority-{key}"),
                            Some(if collapsed {
                                IconName::ChevronRight
                            } else {
                                IconName::ChevronDown
                            }),
                            format!("{} ({})", label.to_uppercase(), rows.len()),
                            false,
                        )
                        .text_xs()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(muted_foreground)
                        .on_click(cx.listener(move |app, _, _, cx| {
                            if app.collapsed_groups.contains(&collapse_key) {
                                app.collapsed_groups.retain(|key| key != &collapse_key);
                            } else {
                                app.collapsed_groups.push(collapse_key.clone());
                            }
                            app.reconcile_sidebar_selection();
                            cx.notify();
                        })),
                    );
                if !collapsed {
                    for thread in rows {
                        shelf = shelf.child(self.cached_sidebar_row(thread, now, cx));
                    }
                }
                projects.push(shelf.into_any_element());
            }
        }
        let mut search_results = div().flex().flex_col().gap_1();
        let current_search = self.sidebar_state.search_environment == self.environment_scope;
        if current_search && self.sidebar_state.search_loading {
            search_results = search_results.child("Searching…");
        }
        if current_search && let Some(error) = &self.sidebar_state.search_error {
            search_results = search_results.child(error.clone());
        }
        if current_search
            && !self.sidebar_state.search_query.is_empty()
            && !self.sidebar_state.search_loading
            && self.sidebar_state.search_error.is_none()
            && self.search_results.is_empty()
        {
            search_results = search_results.child("No matching messages");
        }
        for (index, result) in self
            .search_results
            .iter()
            .enumerate()
            .filter(|_| current_search)
        {
            if let Some(id) = result["threadId"].as_str() {
                let thread = self.shell.threads.iter().find(|thread| thread.id == id);
                let project = self
                    .shell
                    .projects
                    .iter()
                    .find(|project| result["projectId"].as_str() == Some(project.id.as_str()));
                let title = thread.map(|thread| thread.title.as_str()).unwrap_or(id);
                let project_title = project
                    .map(|project| project.title.as_str())
                    .unwrap_or("Project");
                search_results =
                    search_results.child(
                        Button::new(SharedString::from(format!("search-match-{index}")))
                            .debug_selector(move || format!("search-match-{index}"))
                            .ghost()
                            .selected(self.sidebar_state.search_index == index)
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .min_w_0()
                                    .child(format!("{title} · {project_title}"))
                                    .child(
                                        div().text_xs().child(
                                            gpui_kit::StyledText::new(
                                                result["snippet"]
                                                    .as_str()
                                                    .unwrap_or("Match")
                                                    .to_owned(),
                                            )
                                            .with_highlights(
                                                sidebar::match_ranges(
                                                    result["snippet"].as_str().unwrap_or(""),
                                                    &self.sidebar_state.search_query,
                                                )
                                                .into_iter()
                                                .map(|range| {
                                                    (
                                                        range,
                                                        gpui_kit::HighlightStyle {
                                                            background_color: Some(
                                                                cx.theme().selection,
                                                            ),
                                                            ..Default::default()
                                                        },
                                                    )
                                                }),
                                            ),
                                        ),
                                    ),
                            )
                            .on_click(cx.listener(move |app, _, window, cx| {
                                app.select_search_match(index, window, cx)
                            })),
                    );
            }
        }
        div()
            .id("sidebar")
            .debug_selector(|| "sidebar".into())
            .flex()
            .flex_col()
            .size_full()
            .bg(sidebar_color)
            .text_color(sidebar_foreground)
            .border_r_1()
            .border_color(sidebar_border)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_1()
                    .p_2()
                    .flex_shrink_0()
                    .child(
                        div()
                            .debug_selector(|| "sidebar-search-field".into())
                            .key_context("NativeThreadSearch")
                            .flex_1()
                            .min_w_0()
                            .on_action(cx.listener(|app, _: &crate::ThreadSearchNext, _, cx| {
                                app.step_search_match(true, cx);
                            }))
                            .on_action(cx.listener(
                                |app, _: &crate::ThreadSearchPrevious, _, cx| {
                                    app.step_search_match(false, cx);
                                },
                            ))
                            .on_action(cx.listener(
                                |app, _: &crate::ThreadSearchCancel, window, cx| {
                                    app.clear_thread_search(window, cx);
                                },
                            ))
                            .on_action(cx.listener(
                                |app, _: &crate::ThreadSearchSubmit, window, cx| {
                                    if !app.search_results.is_empty()
                                        && app.search.read(cx).value().trim()
                                            == app.sidebar_state.search_query
                                    {
                                        app.select_search_match(
                                            app.sidebar_state.search_index,
                                            window,
                                            cx,
                                        );
                                    } else {
                                        app.search_threads(cx);
                                    }
                                },
                            ))
                            .child(Input::new(&self.search)),
                    )
                    .when(!self.search.read(cx).value().is_empty(), |view| {
                        view.child(
                            Button::new("clear-search")
                                .debug_selector(|| "clear-search".into())
                                .icon(IconName::X)
                                .tooltip("Clear thread search")
                                .ghost()
                                .small()
                                .on_click(cx.listener(|app, _, window, cx| {
                                    app.clear_thread_search(window, cx)
                                })),
                        )
                    })
                    .child(
                        Button::new("search")
                            .icon(IconName::Search)
                            .tooltip("Search threads · Enter")
                            .ghost()
                            .small()
                            .on_click(cx.listener(|app, _, _, cx| app.search_threads(cx))),
                    )
                    .child(
                        Button::new("new-thread")
                            .debug_selector(|| "sidebar-new-thread".into())
                            .label("New")
                            .primary()
                            .small()
                            .on_click(cx.listener(|app, _, window, cx| {
                                app.execute_command(palette::Command::NewThread, window, cx)
                            })),
                    ),
            )
            .when_some(
                self.thread_undo.record.as_ref().filter(|record| {
                    record.environment == self.environment_scope
                        && record.generation == self.generation
                }),
                |view, record| {
                    view.child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .p_2()
                            .child(div().text_xs().child(format!("Updated {}", record.label())))
                            .child(
                                Button::new("undo-thread-navigation")
                                    .debug_selector(|| "undo-thread-navigation".into())
                                    .label(if self.thread_undo.busy {
                                        "Updating…"
                                    } else {
                                        "Undo · ⌘Z"
                                    })
                                    .disabled((self.thread_undo.busy) || !self.connection_ready())
                                    .ghost()
                                    .small()
                                    .on_click(
                                        cx.listener(|app, _, _, cx| app.undo_thread_navigation(cx)),
                                    ),
                            ),
                    )
                },
            )
            .when(
                self.sidebar_state.selection.len() > 1
                    && self.sidebar_state.selection_environment == self.environment_scope,
                |view| {
                    view.child(div().text_xs().px_2().child(format!(
                        "{} selected{}",
                        self.sidebar_state.selection.len(),
                        if self.sidebar_state.bulk_busy {
                            " · Updating…"
                        } else {
                            ""
                        }
                    )))
                },
            )
            .child(
                div()
                    .flex()
                    .flex_shrink_0()
                    .items_center()
                    .gap_1()
                    .px_2()
                    .children(
                        [(true, "My priority"), (false, "By project")]
                            .into_iter()
                            .map(|(priority, label)| {
                                Button::new(if priority {
                                    "priority-view"
                                } else {
                                    "project-view"
                                })
                                .debug_selector(move || {
                                    if priority {
                                        "priority-view".into()
                                    } else {
                                        "project-view".into()
                                    }
                                })
                                .label(if self.sidebar_width < 260. {
                                    if priority { "Priority" } else { "Projects" }
                                } else {
                                    label
                                })
                                .tooltip(label)
                                .ghost()
                                .small()
                                .selected(self.sidebar_state.priority == priority)
                                .on_click(cx.listener(
                                    move |app, _, window, cx| {
                                        app.sidebar_state.priority = priority;
                                        if let Err(error) = app.sidebar_state.save_local() {
                                            app.report_error(error, cx);
                                        }
                                        app.sidebar_state.scroll.set_offset(Default::default());
                                        app.reveal_sidebar_selection();
                                        app.sidebar_state.scroll_selected(window, cx);
                                        app.reconcile_sidebar_selection();
                                        cx.notify();
                                    },
                                ))
                            }),
                    )
                    .child(
                        Button::new("sidebar-scope")
                            .debug_selector(|| "sidebar-scope".into())
                            .icon(IconName::Folder)
                            .tooltip(
                                self.sidebar_state
                                    .project_scope
                                    .as_ref()
                                    .and_then(|id| {
                                        self.shell.projects.iter().find(|project| &project.id == id)
                                    })
                                    .map(|project| format!("Project scope: {}", project.title))
                                    .unwrap_or_else(|| "Project scope: All projects".into()),
                            )
                            .selected(
                                self.sidebar_state.project_scope.is_some()
                                    || self.sidebar_state.scope_picker,
                            )
                            .ghost()
                            .small()
                            .on_click(cx.listener(|app, _, _, cx| {
                                app.sidebar_state.scope_picker = !app.sidebar_state.scope_picker;
                                cx.notify();
                            })),
                    ),
            )
            .when(self.sidebar_state.scope_picker, |view| {
                view.child(
                    div()
                        .flex()
                        .flex_col()
                        .children(self.shell.projects.iter().map(|project| {
                            let id = project.id.clone();
                            Button::new(SharedString::from(format!("scope-{id}")))
                                .debug_selector({
                                    let id = id.clone();
                                    move || format!("scope-{id}")
                                })
                                .label(project.title.clone())
                                .ghost()
                                .small()
                                .selected(self.sidebar_state.project_scope.as_ref() == Some(&id))
                                .on_click(cx.listener(move |app, _, _, cx| {
                                    app.sidebar_state.project_scope = Some(id.clone());
                                    app.sidebar_state.scope_picker = false;
                                    app.reconcile_sidebar_selection();
                                    cx.notify();
                                }))
                        })),
                )
            })
            .when(self.sidebar_state.project_scope.is_some(), |view| {
                view.child(
                    Button::new("reset-project-scope")
                        .debug_selector(|| "reset-project-scope".into())
                        .label("Show all projects")
                        .ghost()
                        .small()
                        .on_click(cx.listener(|app, _, _, cx| {
                            app.sidebar_state.project_scope = None;
                            app.reconcile_sidebar_selection();
                            cx.notify();
                        })),
                )
            })
            .when(
                self.sidebar_state
                    .rename_target
                    .as_ref()
                    .is_some_and(|(environment, _)| environment == &self.environment_scope),
                |view| {
                    view.child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .p_2()
                            .child("Rename thread")
                            .child(Input::new(&self.sidebar_rename))
                            .child(
                                Button::new("row-save-rename")
                                    .debug_selector(|| "row-save-rename".into())
                                    .label(
                                        if self.sidebar_state.rename_busy == Some(self.generation) {
                                            "Renaming…"
                                        } else {
                                            "Rename"
                                        },
                                    )
                                    .disabled(
                                        self.sidebar_state.rename_busy == Some(self.generation),
                                    )
                                    .ghost()
                                    .small()
                                    .on_click(
                                        cx.listener(|app, _, _, cx| app.rename_sidebar_target(cx)),
                                    ),
                            )
                            .child(
                                Button::new("row-cancel-rename")
                                    .label("Cancel")
                                    .ghost()
                                    .small()
                                    .on_click(cx.listener(|app, _, _, cx| {
                                        app.sidebar_state.rename_target = None;
                                        cx.notify();
                                    })),
                            ),
                    )
                },
            )
            .when(self.sidebar_state.custom_snooze_target.is_some(), |view| {
                view.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .p_2()
                        .child("Wake at (ISO date/time with timezone)")
                        .child(Input::new(&self.custom_snooze))
                        .child(
                            Button::new("row-custom-snooze")
                                .debug_selector(|| "row-custom-snooze".into())
                                .disabled(!self.connection_ready())
                                .label("Snooze")
                                .ghost()
                                .small()
                                .on_click(cx.listener(|app, _, _, cx| app.snooze_custom(cx))),
                        )
                        .child(
                            Button::new("row-cancel-snooze")
                                .debug_selector(|| "row-cancel-snooze".into())
                                .label("Cancel")
                                .ghost()
                                .small()
                                .on_click(cx.listener(|app, _, _, cx| {
                                    app.sidebar_state.custom_snooze_target = None;
                                    app.sidebar_state.bulk_snooze_review = None;
                                    cx.notify();
                                })),
                        ),
                )
            })
            .child(
                div().flex().flex_wrap().gap_1().px_2().pb_2().children(
                    [
                        ("active-view", "Active", sidebar::View::Active),
                        ("settled-view", "Settled", sidebar::View::Settled),
                        ("snoozed-view", "Snoozed", sidebar::View::Snoozed),
                        ("archived-view", "Archived", sidebar::View::Archived),
                    ]
                    .into_iter()
                    .filter(|(_, _, view)| {
                        !self.sidebar_state.priority
                            || matches!(view, sidebar::View::Active | sidebar::View::Archived)
                    })
                    .map(|(id, label, view)| {
                        Button::new(id)
                            .debug_selector(move || id.into())
                            .label(
                                if self.sidebar_state.priority && view == sidebar::View::Active {
                                    "All shelves"
                                } else {
                                    label
                                },
                            )
                            .ghost()
                            .small()
                            .selected(self.sidebar_view == view)
                            .on_click(cx.listener(move |app, _, _, cx| {
                                app.sidebar_view = view;
                                app.reconcile_sidebar_selection();
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
            .when_some(
                self.sidebar_state
                    .project_settings_target
                    .as_ref()
                    .filter(|(environment, _)| environment == &self.environment_scope)
                    .and_then(|(_, id)| {
                        self.shell.projects.iter().find(|project| &project.id == id)
                    }),
                |view, project| {
                    let id = project.id.clone();
                    let reset_id = id.clone();
                    let environment = self.environment_scope.clone();
                    let reset_environment = environment.clone();
                    view.child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .p_2()
                            .child("Project settings")
                            .child(project.workspace_root.clone())
                            .child(Input::new(&self.sidebar_rename))
                            .child(
                                Button::new("sidebar-project-rename")
                                    .debug_selector(|| "sidebar-project-rename".into())
                                    .label("Save project name")
                                    .ghost()
                                    .small()
                                    .disabled(!self.connection_ready() || self.pending_actions > 0)
                                    .on_click(cx.listener(move |app, _, _, cx| {
                                        if app.environment_scope != environment {
                                            return;
                                        }
                                        match t3code_desktop_gpui::commands::rename_project(
                                            &id,
                                            &app.sidebar_rename.read(cx).value(),
                                        ) {
                                            Ok(command) => app.dispatch(command, cx),
                                            Err(error) => app.report_error(error, cx),
                                        }
                                    })),
                            )
                            .when(project.default_model_selection.is_some(), |view| {
                                view.child(
                                    Button::new("sidebar-project-reset-model")
                                        .label("Use environment model default")
                                        .ghost()
                                        .small()
                                        .disabled(
                                            !self.connection_ready() || self.pending_actions > 0,
                                        )
                                        .on_click(cx.listener(move |app, _, _, cx| {
                                            if app.environment_scope != reset_environment {
                                                return;
                                            }
                                            let command = json!({
                                                "type": "project.meta.update",
                                                "commandId": t3code_desktop_gpui::commands::id(),
                                                "projectId": reset_id,
                                                "defaultModelSelection": null
                                            });
                                            app.dispatch(command, cx);
                                        })),
                                )
                            })
                            .child(
                                Button::new("sidebar-project-settings-close")
                                    .label("Close project settings")
                                    .ghost()
                                    .small()
                                    .on_click(cx.listener(|app, _, _, cx| {
                                        app.sidebar_state.project_settings_target = None;
                                        cx.notify();
                                    })),
                            ),
                    )
                },
            )
            .children(self.render_sidebar_cleanup(cx))
            .child(
                div()
                    .id("sidebar-list")
                    .debug_selector(|| "sidebar-list".into())
                    .track_scroll(&self.sidebar_state.scroll)
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
                    .items_center()
                    .p_2()
                    .border_t_1()
                    .border_color(sidebar_border)
                    .child(
                        Button::new("sidebar-settings")
                            .debug_selector(|| "sidebar-settings".into())
                            .icon(IconName::Settings)
                            .label("Settings")
                            .ghost()
                            .small()
                            .on_click(
                                cx.listener(|app, _, window, cx| app.open_settings("", window, cx)),
                            ),
                    ),
            )
    }
}
