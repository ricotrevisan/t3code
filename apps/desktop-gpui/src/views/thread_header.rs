use gpui_kit::assets::IconName;
use gpui_kit::base::Selectable;
use gpui_kit::component::{
    ActiveTheme as _, Disableable, Icon, Sizable,
    button::{Button, ButtonVariants as _},
    tooltip::Tooltip,
};
use gpui_kit::{Context, FontWeight, anchored, deferred, div, prelude::*};
use t3code_desktop_gpui::{conversation, palette, sidebar};

use crate::NativeApp;

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
        let branch_label = header_branch.clone().or_else(|| {
            (has_thread && self.selected_workspace().is_some())
                .then(|| "Choose checkout".to_owned())
        });
        let branch_trigger = if header_branch.is_some() {
            "branch-selector"
        } else {
            "checkout-selector"
        };
        let header_worktree = self.selected_thread_worktree().is_some();
        let branch_picker = self.branch_picker == Some(branch_trigger);
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
        let linked_pr = self
            .detail
            .thread
            .as_ref()
            .filter(|detail| detail["id"].as_str() == self.selected_thread.as_deref())
            .and_then(|detail| detail["pullRequests"].as_array())
            .and_then(|links| links.first())
            .and_then(|link| {
                let url = link["url"].as_str()?.to_owned();
                let number = link["number"].as_u64()?;
                let state = link["snapshot"]["state"]
                    .as_str()
                    .unwrap_or("status unknown");
                Some((format!("PR #{number} · {state}"), url))
            });
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
            .debug_selector(|| "thread-header".into())
            .flex()
            .flex_col()
            .min_w_0()
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
                        .w_full()
                        .min_w_0()
                        // Identity never competes with branch/status intrinsic widths.
                        .when_some(header_project, |view, project| {
                            view.child(
                                div()
                                    .id("header-project")
                                    .debug_selector(|| "header-project".into())
                                    .w_24()
                                    .flex_shrink_0()
                                    .min_w_0()
                                    .truncate()
                                    .text_sm()
                                    .text_color(muted_foreground)
                                    .tooltip({
                                        let project = project.clone();
                                        move |window, cx| {
                                            Tooltip::new(project.clone()).build(window, cx)
                                        }
                                    })
                                    .cursor_pointer()
                                    .on_click(cx.listener(|app, _, window, cx| app.new_thread(window, cx)))
                                    .child(project),
                            )
                        })
                        .when_some(self.header_rename.input.as_ref(), |view, input| view.child(
                            div().flex_1().min_w_0()
                                .capture_key_down(cx.listener(|app, event: &gpui_kit::KeyDownEvent, window, cx| {
                                    if event.keystroke.key == "escape" { app.finish_header_rename(false, window, cx); cx.stop_propagation(); }
                                }))
                                .child(gpui_kit::component::input::Input::new(input).small())
                        ))
                        .when(self.header_rename.input.is_none(), |view| view.child(
                            div()
                                .id("header-title")
                                .debug_selector(|| "header-title".into())
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .font_weight(FontWeight::SEMIBOLD)
                                .tooltip({
                                    let title = header_title.clone();
                                    move |window, cx| Tooltip::new(title.clone()).build(window, cx)
                                })
                                .cursor_text()
                                .on_click(cx.listener({
                                    let title = header_title.clone();
                                    move |app, event: &gpui_kit::ClickEvent, window, cx| {
                                        if event.click_count() == 2 { app.begin_header_rename(title.clone(), window, cx); }
                                    }
                                }))
                                .child(header_title),
                        )),
                )
                .child(
                    div()
                        .debug_selector(|| "header-metadata".into())
                        .flex()
                        .items_center()
                        .gap_2()
                        .w_full()
                        .min_w_0()
                        .when_some(branch_label, |view, branch| {
                            view.child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .flex_1()
                                    .min_w_0()
                                    .child(
                                        Button::new(branch_trigger)
                                            .debug_selector(move || branch_trigger.into())
                                            .icon(IconName::GitBranch)
                                            .label(branch)
                                            .when(header_worktree, |button| {
                                                button
                                                    .child(Icon::new(IconName::FolderGit).xsmall())
                                            })
                                            .ghost()
                                            .small()
                                            .w_full()
                                            .selected(branch_picker)
                                            .tooltip(header_branch.as_ref().map(|branch| format!("Switch branch or worktree · {branch}")).unwrap_or_else(|| "Choose this thread's checkout, create a branch, or checkout a PR".into()))
                                            .on_click(cx.listener(move |app, _, window, cx| {
                                                app.open_branch_picker(branch_trigger, window, cx)
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
                        .when_some(linked_pr, |view, (label, url)| view.child(
                            Button::new("header-linked-pr").ghost().small().label(label).tooltip(url.clone())
                                .on_click(cx.listener(move |_, _, _, cx| { cx.open_url(&url); }))
                        ))
                        .when_some(header_session, |view, (label, is_danger)| {
                            view.child(
                                div()
                                    .id("header-session")
                                    .debug_selector(|| "header-session".into())
                                    .flex_shrink_0()
                                    .max_w_32()
                                    .min_w_0()
                                    .truncate()
                                    .px_2()
                                    .rounded_sm()
                                    .bg(muted)
                                    .text_xs()
                                    .text_color(if is_danger { danger } else { muted_foreground })
                                    .tooltip({
                                        let label = label.clone();
                                        move |window, cx| {
                                            Tooltip::new(label.clone()).build(window, cx)
                                        }
                                    })
                                    .child(label),
                            )
                        }),
                )
                // Diff totals have their own line rather than squeezing out runtime state.
                .when_some(header_diff, |view, (files, additions, deletions)| {
                    view.child(
                        div()
                            .id("header-diff-click")
                            .debug_selector(|| "header-diff".into())
                            .cursor_pointer()
                            .on_click(cx.listener(|app, _, _, cx| {
                                if let Some(turn) = app.detail.thread.as_ref().and_then(|detail| detail["checkpoints"].as_array()).and_then(|list| list.last()).and_then(|checkpoint| checkpoint["checkpointTurnCount"].as_u64()) { app.view_checkpoint_diff(turn, cx); }
                            }))
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
                })
            })
            .when(self.editor_choice.open, |view| view.child(self.render_editor_choices(cx)))
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
                                .disabled((self.editor_unavailable_reason().is_some()) || !self.connection_ready())
                                .tooltip(self.editor_unavailable_reason().unwrap_or("Open the worktree in the preferred editor"))
                                .on_click(cx.listener(|app, _, _, cx| app.open_in_editor(cx))),
                        )
                        .child(Button::new("open-editor-picker").ghost().small().label("Open in…")
                            .on_click(cx.listener(|app, _, _, cx| { app.editor_choice.open = !app.editor_choice.open; cx.notify(); })))
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
                            self.render_git_split(cx),
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
        self.render_sidebar_thread_menu(cx)
    }
}
