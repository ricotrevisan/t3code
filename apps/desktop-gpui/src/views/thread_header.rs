use gpui_kit::assets::IconName;
use gpui_kit::base::Selectable;
use gpui_kit::component::{
    ActiveTheme as _, Icon, Sizable,
    button::{Button, ButtonVariants as _},
};
use gpui_kit::{Context, FontWeight, anchored, deferred, div, prelude::*, px};
use t3code_desktop_gpui::{conversation, palette, sidebar};

use super::list_row;
use crate::{NativeApp, THREAD_ACTIONS};

impl NativeApp {
    pub(crate) fn render_thread_header(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let muted_foreground = cx.theme().muted_foreground;
        let border = cx.theme().border;
        let muted = cx.theme().muted;
        let green = cx.theme().green;
        let danger = cx.theme().danger;
        let has_thread = self.selected_thread.is_some();
        let header_title = self.selected_thread_title();
        let header_branch = self.selected_thread_branch();
        let header_worktree = self.selected_thread_worktree().is_some();
        let branch_picker = self.branch_picker == Some("branch-selector");
        let header_session = self.detail.thread.as_ref().and_then(|detail| {
            let status = detail["session"]["status"].as_str()?;
            let (label, danger) = match status {
                "running" | "starting" => ("Running", false),
                "error" => ("Error", true),
                "interrupted" | "stopped" => ("Interrupted", true),
                "idle" | "ready" => return None,
                other => (other, false),
            };
            Some((label.to_owned(), danger))
        });
        let header_diff = self
            .detail
            .thread
            .as_ref()
            .and_then(|detail| {
                detail["checkpoints"]
                    .as_array()
                    .and_then(|list| list.last())
            })
            .map(|checkpoint| {
                let files = checkpoint["files"].as_array().cloned().unwrap_or_default();
                let (additions, deletions) = conversation::diff_totals(&files);
                (files.len(), additions, deletions)
            })
            .filter(|(files, _, _)| *files > 0);
        let header_project = self
            .selected_project
            .as_ref()
            .and_then(|id| self.shell.projects.iter().find(|project| &project.id == id))
            .map(|project| {
                sidebar::project_name(&project.title, &project.workspace_root).to_owned()
            });

        let thread_menu = (self.thread_menu_open && self.thread_menu_row.is_none())
            .then(|| self.render_thread_menu(cx));
        div()
            .flex()
            .flex_col()
            .gap_2()
            .flex_shrink_0()
            .px_3()
            .py_2()
            .border_b_1()
            .border_color(border)
            .when(has_thread, |view| {
                view.child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .min_w_0()
                        .when_some(header_project, |view, project| {
                            view.child(
                                div()
                                    .text_sm()
                                    .text_color(muted_foreground)
                                    .child(format!("{project} /")),
                            )
                        })
                        .child(
                            div()
                                .min_w_0()
                                .truncate()
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(header_title),
                        )
                        .when_some(header_branch, |view, branch| {
                            view.child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .child(
                                        Button::new("branch-selector")
                                            .debug_selector(|| "branch-selector".into())
                                            .icon(IconName::GitBranch)
                                            .label(branch)
                                            .when(header_worktree, |button| {
                                                button
                                                    .child(Icon::new(IconName::FolderGit).xsmall())
                                            })
                                            .ghost()
                                            .small()
                                            .selected(branch_picker)
                                            .tooltip(
                                                "Switch the branch or worktree for this thread",
                                            )
                                            .on_click(cx.listener(|app, _, window, cx| {
                                                app.open_branch_picker(
                                                    "branch-selector",
                                                    window,
                                                    cx,
                                                )
                                            })),
                                    )
                                    // The wrapper puts the picker below the chip, not at the column's top.
                                    .when(branch_picker, |view| {
                                        view.child(
                                            div().child(deferred(
                                                anchored()
                                                    .snap_to_window()
                                                    .child(self.render_branch_picker(false, cx)),
                                            )),
                                        )
                                    }),
                            )
                        })
                        .when_some(header_session, |view, (label, is_danger)| {
                            view.child(
                                div()
                                    .px_2()
                                    .rounded_sm()
                                    .bg(muted)
                                    .text_xs()
                                    .text_color(if is_danger { danger } else { muted_foreground })
                                    .child(label),
                            )
                        })
                        .when_some(header_diff, |view, (files, additions, deletions)| {
                            view.child(
                                div()
                                    .debug_selector(|| "header-diff".into())
                                    .flex()
                                    .items_center()
                                    .gap_1()
                                    .px_2()
                                    .rounded_sm()
                                    .bg(muted)
                                    .text_xs()
                                    .text_color(muted_foreground)
                                    .child(format!("{files} files"))
                                    .child(div().text_color(green).child(format!("+{additions}")))
                                    .child(div().text_color(danger).child(format!("-{deletions}"))),
                            )
                        }),
                )
            })
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap_1()
                    .items_center()
                    .when(has_thread, |view| {
                        view.child(
                            Button::new("open-editor")
                                .debug_selector(|| "open-editor".into())
                                .icon(gpui_kit::assets::IconName::ExternalLink)
                                .label("Open")
                                .secondary()
                                .small()
                                .tooltip("Open the worktree in an editor")
                                .on_click(cx.listener(|app, _, _, cx| app.open_in_editor(cx))),
                        )
                        .child(
                            Button::new("project-actions-header")
                                .debug_selector(|| "project-actions-header".into())
                                .label("Actions")
                                .ghost()
                                .small()
                                .tooltip("Edit project actions and setup scripts")
                                .on_click(cx.listener(|app, _, window, cx| {
                                    app.execute_command(
                                        palette::Command::ProjectActions,
                                        window,
                                        cx,
                                    )
                                })),
                        )
                        .child(
                            Button::new("push-pr")
                                .debug_selector(|| "push-pr".into())
                                .icon(gpui_kit::assets::IconName::GitPullRequest)
                                .label("Push & create PR")
                                .secondary()
                                .small()
                                .tooltip("Commit, push, and open a pull request")
                                .on_click(cx.listener(|app, _, _, cx| app.push_and_create_pr(cx))),
                        )
                    })
                    .child(
                        Button::new("toggle-inspector")
                            .debug_selector(|| "toggle-inspector".into())
                            .label(if self.inspector_open {
                                "Hide panel"
                            } else {
                                "Show panel"
                            })
                            .ghost()
                            .small()
                            .tooltip("Show or hide the project inspector")
                            .on_click(cx.listener(|app, _, _, cx| {
                                app.set_inspector_open(!app.inspector_open, cx)
                            })),
                    )
                    // The menu hangs off its trigger rather than a fixed window position.
                    .when(has_thread, |view| {
                        view.child(
                            div()
                                .flex()
                                .flex_col()
                                .child(
                                    Button::new("thread-menu")
                                        .debug_selector(|| "thread-menu".into())
                                        .icon(gpui_kit::assets::IconName::Ellipsis)
                                        .label("More")
                                        .ghost()
                                        .small()
                                        .selected(
                                            self.thread_menu_open && self.thread_menu_row.is_none(),
                                        )
                                        .tooltip("More thread actions")
                                        .on_click(cx.listener(|app, _, _, cx| {
                                            let open = !app.thread_menu_open;
                                            app.close_thread_menu();
                                            app.thread_menu_open = open;
                                            cx.notify();
                                        })),
                                )
                                .when_some(thread_menu, |view, menu| {
                                    view.child(deferred(anchored().snap_to_window().child(menu)))
                                }),
                        )
                    }),
            )
    }

    /// The thread actions menu, shared by the header "More" trigger and a sidebar
    /// row's secondary click. Entries act on `thread_menu_target`.
    pub(crate) fn render_thread_menu(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let now = chrono::Utc::now();
        let target = self.thread_menu_target();
        let menu_thread = target.as_ref().and_then(|id| {
            self.shell
                .threads
                .iter()
                .chain(&self.archived_shell.threads)
                .find(|thread| &thread.id == id)
        });
        let armed = target.is_some() && self.confirm_delete == target;
        div()
            .id("thread-menu-popover")
            .occlude()
            .mt_1()
            .w(px(220.))
            .flex()
            .flex_col()
            .p_2()
            .rounded_md()
            .bg(cx.theme().popover)
            .border_1()
            .border_color(cx.theme().border)
            .shadow_lg()
            .children(
                THREAD_ACTIONS
                    .iter()
                    .filter(|(id, _, _)| {
                        menu_thread.is_none_or(|thread| sidebar::action_applies(thread, id, now))
                    })
                    .map(|(id, label, kind)| {
                        list_row(
                            *id,
                            None,
                            if armed && *id == "delete" {
                                "Confirm delete"
                            } else {
                                *label
                            },
                            false,
                        )
                        .on_click(cx.listener(
                            move |app, _, window, cx| app.run_thread_menu_action(kind, window, cx),
                        ))
                    }),
            )
    }
}
