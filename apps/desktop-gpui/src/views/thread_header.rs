//! A single identity/action band; secondary controls move into anchored overflow.
use crate::NativeApp;
use gpui_kit::assets::IconName;
use gpui_kit::base::{FocusTrapElement, Selectable};
use gpui_kit::component::{
    ActiveTheme as _, Disableable, Sizable,
    button::{Button, ButtonVariants as _},
    menu::{ContextMenuExt, PopupMenuItem},
    tooltip::Tooltip,
};
use gpui_kit::{
    Context, FontWeight, Render, WeakEntity, Window, anchored, deferred, div, prelude::*,
};
use t3code_desktop_gpui::{conversation, sidebar};

pub(crate) struct HeaderView {
    app: WeakEntity<NativeApp>,
}
impl Render for HeaderView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.app
            .update(cx, |app, cx| {
                app.render_header_band(window, cx).into_any_element()
            })
            .unwrap_or_else(|_| div().into_any_element())
    }
}
impl NativeApp {
    pub(crate) fn render_thread_header(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let app = cx.entity().downgrade();
        let view = self
            .header_rename
            .view
            .get_or_init(|| cx.new(|_| HeaderView { app }))
            .clone();
        // The wrapper supplies actual window/rem geometry, not a stale header
        // projection. App notifications must also invalidate its subtree.
        view.update(cx, |_, cx| cx.notify());
        view
    }
    fn render_header_band(
        &self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let width = f32::from(window.viewport_size().width)
            - if self.sidebar_open {
                self.sidebar_width
            } else {
                0.
            }
            - if self.inspector_open {
                self.inspector_width
            } else {
                0.
            };
        let collapsed = width < f32::from(window.rem_size()) * 68.;
        let overflow_focus = self
            .header_rename
            .overflow_focus
            .get_or_init(|| cx.focus_handle())
            .clone();
        let overflow_bounds = self.header_rename.overflow_bounds.clone();
        let overflow_offset = self
            .header_rename
            .overflow_bounds
            .get()
            .map_or(window.rem_size() * 1.75, |bounds| bounds.size.height)
            + window.rem_size() * 0.25;
        let muted = cx.theme().muted_foreground;
        let has_thread = self.selected_thread.is_some();
        let title = self.selected_thread_title();
        let project = self
            .selected_project
            .as_ref()
            .and_then(|id| self.shell.projects.iter().find(|p| &p.id == id))
            .map(|p| sidebar::project_name(&p.title, &p.workspace_root).to_owned());
        let thread_menu = (self.thread_menu_open && self.thread_menu_row.is_none())
            .then(|| self.render_thread_menu(cx));
        div()
            .debug_selector(|| "thread-header".into())
            .flex()
            .items_center()
            .min_w_0()
            .gap_2()
            .flex_shrink_0()
            .px_3()
            .py_2()
            .border_b_1()
            .border_color(cx.theme().border)
            .when(has_thread, |view| {
                view.child(
                    div()
                        .flex()
                        .items_center()
                        .flex_1()
                        .min_w_0()
                        .gap_1()
                        .when_some(project, |view, project| {
                            view.child(
                                div()
                                    .id("header-project")
                                    .debug_selector(|| "header-project".into())
                                    .max_w_24()
                                    .flex_shrink_0()
                                    .min_w_0()
                                    .truncate()
                                    .text_sm()
                                    .text_color(muted)
                                    .tooltip({
                                        let label = project.clone();
                                        move |window, cx| {
                                            Tooltip::new(label.clone()).build(window, cx)
                                        }
                                    })
                                    .cursor_pointer()
                                    .on_click(
                                        cx.listener(|app, _, window, cx| {
                                            app.new_thread(window, cx)
                                        }),
                                    )
                                    .child(project)
                                    .context_menu({
                                        let entity = cx.entity().downgrade();
                                        move |menu, _, _| {
                                            let entity = entity.clone();
                                            menu.item(
                                                PopupMenuItem::new("Project settings").on_click(
                                                    move |_, window, cx| {
                                                        let _ = entity.update(cx, |app, cx| {
                                                            app.open_header_project_settings(
                                                                window, cx,
                                                            )
                                                        });
                                                    },
                                                ),
                                            )
                                        }
                                    }),
                            )
                            .child(div().text_color(muted).child("/"))
                        })
                        .when_some(self.header_rename.input.as_ref(), |view, input| {
                            view.child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .capture_key_down(cx.listener(
                                        |app, event: &gpui_kit::KeyDownEvent, window, cx| {
                                            if event.keystroke.key == "escape" {
                                                app.finish_header_rename(false, window, cx);
                                                cx.stop_propagation();
                                            }
                                        },
                                    ))
                                    .child(gpui_kit::component::input::Input::new(input).small()),
                            )
                        })
                        .when(self.header_rename.input.is_none(), |view| {
                            view.child(
                                div()
                                    .id("header-title")
                                    .debug_selector(|| "header-title".into())
                                    .flex_1()
                                    .min_w_0()
                                    .truncate()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .tooltip({
                                        let title = title.clone();
                                        move |window, cx| {
                                            Tooltip::new(title.clone()).build(window, cx)
                                        }
                                    })
                                    .cursor_pointer()
                                    .on_click(cx.listener({
                                        let title = title.clone();
                                        move |app, event: &gpui_kit::ClickEvent, window, cx| {
                                            if event.click_count() == 2 {
                                                app.close_thread_menu();
                                                app.begin_header_rename(title.clone(), window, cx);
                                            } else {
                                                let open = !app.thread_menu_open;
                                                app.close_thread_menu();
                                                app.thread_menu_open = open;
                                                app.header_rename.menu_position = Some(event.position() + gpui_kit::point(gpui_kit::px(0.), window.rem_size()));
                                                cx.notify();
                                            }
                                        }
                                    }))
                                    .child(title),
                            )
                        })
                        .child(
                            Button::new("header-title-actions")
                                .debug_selector(|| "header-title-actions".into())
                                .ghost()
                                .small()
                                .icon(IconName::ChevronDown)
                                .tooltip("Thread actions")
                                .on_click(cx.listener(|app, event: &gpui_kit::ClickEvent, window, cx| {
                                    let open = !app.thread_menu_open;
                                    app.close_thread_menu();
                                    app.thread_menu_open = open;
                                    app.header_rename.menu_position = Some(event.position() + gpui_kit::point(gpui_kit::px(0.), window.rem_size()));
                                    cx.notify();
                                })),
                        )
                        .when_some(thread_menu, |view, menu| {
                            view.child(deferred(anchored().when_some(self.header_rename.menu_position, |anchor,position| anchor.position(position)).snap_to_window().child(menu)))
                        }),
                )
            })
            .when(has_thread && !collapsed, |view| {
                view.child(self.render_header_checkout(cx))
                    .child(self.render_header_diff_stat(cx))
                    .child(self.render_header_open(cx))
                    .child(self.render_preferred_header_action(cx))
                    .child(self.render_git_split(cx))
            })
            .child(
                div()
                    .relative()
                    .flex()
                    .flex_col()
                    .flex_shrink_0()
                    .child(
                        gpui_kit::canvas(
                            move |bounds, _, _| overflow_bounds.set(Some(bounds)),
                            |_, _, _, _| {},
                        )
                        .absolute()
                        .size_full(),
                    )
                    .child(
                        Button::new("header-overflow")
                            .debug_selector(|| "header-overflow".into())
                            .ghost()
                            .small()
                            .icon(IconName::Ellipsis)
                            .selected(self.header_rename.overflow_open)
                            .tooltip("More header actions")
                            .on_click(cx.listener(|app, _, window, cx| {
                                app.header_rename.overflow_open = !app.header_rename.overflow_open;
                                if app.header_rename.overflow_open {
                                    app.header_rename.overflow_previous_focus = window.focused(cx);
                                    app.header_rename
                                        .overflow_focus
                                        .get()
                                        .unwrap()
                                        .focus(window, cx);
                                } else if let Some(focus) = app.header_rename.overflow_previous_focus.take() {
                                    focus.focus(window, cx);
                                }
                                cx.notify();
                            })),
                    )
                    .when(
                        self.header_rename.overflow_open
                            || (collapsed
                                && (self.branch_picker.is_some() || self.git_header.open)),
                        |view| {
                            view.child(div().child(deferred(
                            anchored().offset(gpui_kit::point(gpui_kit::px(0.), overflow_offset)).snap_to_window().child(
                                div()
                                    .id("header-overflow-panel")
                                    .debug_selector(|| "header-overflow-panel".into())
                                    .occlude()
                                    .track_focus(&overflow_focus)
                                    .focus_trap("header-overflow-trap", &overflow_focus)
                                    .w_80()
                                    .max_w_full()
                                    .flex()
                                    .flex_col()
                                    .gap_2()
                                    .p_2()
                                    .rounded_md()
                                    .bg(cx.theme().popover)
                                    .border_1()
                                    .border_color(cx.theme().border)
                                    .shadow_lg()
                                    .on_mouse_down_out(cx.listener(
                                        |app, event: &gpui_kit::MouseDownEvent, window, cx| {
                                            if app.git_header.open || app.branch_picker.is_some() {
                                                return;
                                            }
                                            if !app.header_rename.overflow_bounds.get().is_some_and(
                                                |bounds| bounds.contains(&event.position),
                                            ) {
                                                app.header_rename.overflow_open = false;
                                                if let Some(focus) = app.header_rename.overflow_previous_focus.take() {
                                                    focus.focus(window, cx);
                                                }
                                                cx.notify();
                                            }
                                        },
                                    ))
                                    .capture_key_down(cx.listener(
                                        |app, event: &gpui_kit::KeyDownEvent, window, cx| {
                                            if event.keystroke.key == "escape" {
                                                if app.settings_open || app.palette_open || app.thread_menu_open || app.catalog_open {
                                                    app.dismiss_overlay(window, cx);
                                                } else if app.git_header.open {
                                                    app.dismiss_git_header(cx);
                                                } else if app.branch_picker.is_some() {
                                                    app.close_branch_picker(window, cx);
                                                } else {
                                                    app.header_rename.overflow_open = false;
                                                    if let Some(focus) = app.header_rename.overflow_previous_focus.take() {
                                                        focus.focus(window, cx);
                                                    }
                                                }
                                                cx.stop_propagation();
                                                cx.notify();
                                            }
                                        },
                                    ))
                                    .when_some(
                                        self.detail
                                            .thread
                                            .as_ref()
                                            .and_then(|d| d["session"]["status"].as_str())
                                            .filter(|status| !matches!(*status, "idle" | "ready")),
                                        |view, status| {
                                            view.child(
                                                div()
                                                    .debug_selector(|| "header-session".into())
                                                    .text_xs()
                                                    .text_color(muted)
                                                    .child(format!("Session · {status}")),
                                            )
                                        },
                                    )
                                    .when(has_thread && collapsed, |view| {
                                        view.child(self.render_header_checkout(cx))
                                            .child(self.render_header_diff_stat(cx))
                                            .child(self.render_header_open(cx))
                                            .child(self.render_git_split(cx))
                                    })
                                    .when(has_thread, |view| {
                                        view.child(
                                            Button::new("open-editor-picker")
                                                .ghost()
                                                .small()
                                                .label("Open in…")
                                                .on_click(cx.listener(|app, _, _, cx| {
                                                    app.editor_choice.open =
                                                        !app.editor_choice.open;
                                                    cx.notify();
                                                })),
                                        )
                                        .when(self.editor_choice.open, |view| {
                                            view.child(self.render_editor_choices(cx))
                                        })
                                        .child(self.render_header_actions(collapsed, cx))
                                        .child(self.render_shell_terminal_control(cx))
                                    })
                                    .when(has_thread, |view| {
                                        view.child(
                                            Button::new("thread-menu")
                                                .debug_selector(|| "thread-menu".into())
                                                .ghost()
                                                .small()
                                                .label("Thread actions…")
                                                .on_click(cx.listener(|app, event: &gpui_kit::ClickEvent, window, cx| {
                                                    app.header_rename.overflow_open = false;
                                                    if let Some(focus) = app.header_rename.overflow_previous_focus.take() {
                                                        focus.focus(window, cx);
                                                    }
                                                    app.close_thread_menu();
                                                    app.thread_menu_open = true;
                                                    app.header_rename.menu_position = Some(event.position() + gpui_kit::point(gpui_kit::px(0.), window.rem_size()));
                                                    cx.notify();
                                                })),
                                        )
                                    }),
                            ),
                        )))
                        },
                    ),
            )
            .child(
                Button::new("toggle-inspector")
                    .debug_selector(|| "toggle-inspector".into())
                    .ghost()
                    .small()
                    .icon(IconName::PanelRight)
                    .accessibility_label("Toggle project panel")
                    .tooltip(if self.inspector_open {
                        "Hide panel"
                    } else {
                        "Show panel"
                    })
                    .on_click(
                        cx.listener(|app, _, _, cx| {
                            app.set_inspector_open(!app.inspector_open, cx)
                        }),
                    ),
            )
    }
    fn render_header_open(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        Button::new("open-editor")
            .debug_selector(|| "open-editor".into())
            .icon(IconName::ExternalLink)
            .label("Open")
            .secondary()
            .small()
            .disabled(self.editor_unavailable_reason().is_some() || !self.connection_ready())
            .tooltip(
                self.editor_unavailable_reason()
                    .unwrap_or("Open the worktree in the preferred editor"),
            )
            .on_click(cx.listener(|app, _, _, cx| app.open_in_editor(cx)))
    }
    fn render_header_checkout(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let branch = self.selected_thread_branch();
        let trigger = if branch.is_some() {
            "branch-selector"
        } else {
            "checkout-selector"
        };
        let label = branch.unwrap_or_else(|| "Choose checkout".into());
        div()
            .flex()
            .flex_col()
            .min_w_0()
            .max_w_40()
            .child(
                Button::new(trigger)
                    .debug_selector(move || trigger.into())
                    .icon(IconName::GitBranch)
                    .label(label.clone())
                    .ghost()
                    .small()
                    .selected(self.branch_picker == Some(trigger))
                    .tooltip(format!("Switch branch or worktree · {label}"))
                    .on_click(cx.listener(move |app, _, window, cx| {
                        app.open_branch_picker(trigger, window, cx)
                    })),
            )
            .when(self.branch_picker == Some(trigger), |view| {
                view.child(
                    div().child(deferred(
                        anchored()
                            .snap_to_window()
                            .child(self.render_branch_picker(false, cx)),
                    )),
                )
            })
    }
    fn render_header_diff_stat(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let live = self.live_git_status().filter(|s| s["isRepo"] == true);
        let checkpoint = self
            .detail
            .thread
            .as_ref()
            .filter(|detail| detail["id"].as_str() == self.selected_thread.as_deref())
            .and_then(|d| d["checkpoints"].as_array())
            .and_then(|l| l.last());
        let totals = live
            .map(|s| {
                (
                    s["workingTree"]["files"].as_array().map_or(0, Vec::len),
                    s["workingTree"]["insertions"].as_u64().unwrap_or(0),
                    s["workingTree"]["deletions"].as_u64().unwrap_or(0),
                )
            })
            .or_else(|| {
                checkpoint
                    .map(|c| {
                        let files = c["files"].as_array().map(Vec::as_slice).unwrap_or_default();
                        let (a, d) = conversation::diff_totals(files);
                        (files.len(), a, d)
                    })
                    .filter(|(files, _, _)| *files > 0)
            });
        let current = live.is_some();
        let turn = checkpoint.and_then(|c| c["checkpointTurnCount"].as_u64());
        let owner = (
            self.generation,
            self.environment_scope.clone(),
            self.checkout_context(),
        );
        div()
            .flex()
            .items_center()
            .gap_1()
            .when_some(totals, |view, (files, a, d)| {
                view.child(
                    Button::new(if current {
                        "header-current-diff"
                    } else {
                        "header-diff"
                    })
                    .debug_selector(move || {
                        if current {
                            "header-current-diff".into()
                        } else {
                            "header-diff".into()
                        }
                    })
                    .ghost()
                    .small()
                    .label(format!("{files}"))
                    .child(div().text_color(cx.theme().green).child(format!("+{a}")))
                    .child(div().text_color(cx.theme().danger).child(format!("−{d}")))
                    .tooltip(if current {
                        "Current checkout changes"
                    } else {
                        "Last checkpoint changes"
                    })
                    .on_click(cx.listener(move |app, _, _, cx| {
                        if (
                            app.generation,
                            app.environment_scope.clone(),
                            app.checkout_context(),
                        ) != owner
                        {
                            return;
                        }
                        if current {
                            app.view_git_diff("working-tree", cx);
                        } else if let Some(turn) = turn {
                            app.view_checkpoint_diff(turn, cx);
                        }
                    })),
                )
            })
            .when_some(
                live.and_then(|status| {
                    let pr = &status["pr"];
                    Some((
                        format!(
                            "PR #{} · {}",
                            pr["number"].as_u64()?,
                            pr["state"].as_str().unwrap_or("status unknown")
                        ),
                        pr["url"].as_str()?.to_owned(),
                    ))
                })
                .or_else(|| {
                    let detail = self
                        .detail
                        .thread
                        .as_ref()
                        .filter(|d| d["id"].as_str() == self.selected_thread.as_deref())?;
                    let pr = detail["pullRequests"].as_array()?.first()?;
                    Some((
                        format!(
                            "PR #{} · {}",
                            pr["number"].as_u64()?,
                            pr["snapshot"]["state"].as_str().unwrap_or("status unknown")
                        ),
                        pr["url"].as_str()?.to_owned(),
                    ))
                }),
                |view, (label, url)| {
                    view.child(
                        Button::new("header-linked-pr")
                            .ghost()
                            .small()
                            .icon(IconName::ExternalLink)
                            .label("PR")
                            .tooltip(format!("{label}\n{url}"))
                            .on_click(cx.listener(move |app, _, window, cx| app.open_pull_request_url(&url, window, cx))),
                    )
                },
            )
    }
    pub(crate) fn render_thread_menu(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        self.render_sidebar_thread_menu(cx)
    }
}
