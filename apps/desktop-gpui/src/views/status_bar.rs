use gpui_kit::base::Selectable;
use gpui_kit::component::{
    ActiveTheme as _, Sizable,
    button::{Button, ButtonVariants as _},
    tooltip::Tooltip,
};
use gpui_kit::{Anchor, Context, anchored, deferred, div, prelude::*, px};
use t3code_desktop_gpui::notifications;

use crate::NativeApp;

impl NativeApp {
    pub(crate) fn render_status_bar(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let muted_foreground = cx.theme().muted_foreground;
        let border = cx.theme().border;
        let warning = cx.theme().warning;
        let green = cx.theme().green;
        let danger = cx.theme().danger;
        let busy = self.pending_actions > 0;
        let active_model = self.active_model_label(cx);
        let header_branch = self.selected_thread_branch();
        let environment_host = url::Url::parse(&self.environment_scope)
            .ok()
            .and_then(|url| url.host_str().map(str::to_owned))
            .unwrap_or_else(|| self.environment_scope.clone());
        let worktree_path = self.selected_thread_worktree();
        let status_worktree = worktree_path
            .as_deref()
            .and_then(|path| {
                path.trim_end_matches('/')
                    .rsplit('/')
                    .find(|segment| !segment.is_empty())
            })
            .map(str::to_owned);
        let status_is_error = notifications::is_error(&self.status);
        let status_text = notifications::status_bar_text(self.commands.is_some(), &self.status);
        div()
            .id("status-bar")
            .debug_selector(|| "status-bar".into())
            .flex()
            .items_center()
            .justify_between()
            .gap_3()
            .flex_shrink_0()
            .px_3()
            .py_1()
            .text_xs()
            .text_color(muted_foreground)
            .border_t_1()
            .border_color(border)
            .child(
                div()
                    .flex()
                    .items_center()
                    // Intrinsic chip widths must not steal the message's lane.
                    .gap_1()
                    .flex_1()
                    .min_w_0()
                    .child(div().flex_shrink_0().w(px(8.)).h(px(8.)).rounded_full().bg(
                        if self.commands.is_some() {
                            green
                        } else {
                            muted_foreground
                        },
                    ))
                    .child(
                        div().flex_1().min_w_0().child(
                            Button::new("environment-chip")
                                .debug_selector(|| "environment-chip".into())
                                .icon(gpui_kit::assets::IconName::Settings)
                                .label(environment_host)
                                .ghost()
                                .xsmall()
                                .w_full()
                                .tooltip(format!(
                                    "Open connection settings · {}",
                                    self.environment_scope
                                ))
                                .on_click(cx.listener(|app, _, _, cx| {
                                    app.settings_open = true;
                                    cx.notify();
                                })),
                        ),
                    )
                    .when_some(status_worktree, |view, name| {
                        view.child(
                            self.branch_picker_chip(
                                Button::new("worktree-chip")
                                    .debug_selector(|| "worktree-chip".into())
                                    .icon(gpui_kit::assets::IconName::FolderGit)
                                    .label(name)
                                    .tooltip(format!(
                                        "Switch branch or worktree · {}",
                                        worktree_path.as_deref().unwrap_or_default()
                                    )),
                                "worktree-chip",
                                cx,
                            ),
                        )
                    })
                    .when_some(header_branch, |view, branch| {
                        view.child(
                            self.branch_picker_chip(
                                Button::new("branch-chip")
                                    .debug_selector(|| "branch-chip".into())
                                    .icon(gpui_kit::assets::IconName::GitBranch)
                                    .label(branch.clone())
                                    .tooltip(format!("Switch branch or worktree · {branch}")),
                                "branch-chip",
                                cx,
                            ),
                        )
                    })
                    .when(!active_model.is_empty(), |view| {
                        view.child(div().flex_1().min_w_0().truncate().child(active_model))
                    }),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .flex_1()
                    .flex_grow(1.25)
                    .min_w_0()
                    .when(busy, |view| {
                        view.child(div().flex_shrink_0().text_color(warning).child("Working…"))
                    })
                    .child(
                        div()
                            .id("status-message")
                            .debug_selector(|| "status-message".into())
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_color(if status_is_error {
                                danger
                            } else {
                                muted_foreground
                            })
                            .tooltip({
                                let text = status_text.clone();
                                move |window, cx| {
                                    let text = text.clone();
                                    let max_width =
                                        window.viewport_size().width - window.rem_size() * 3.;
                                    Tooltip::element(move |_, _| {
                                        div()
                                            .id("status-tooltip-text")
                                            .debug_selector(|| "status-tooltip-text".into())
                                            .max_w(max_width)
                                            .child(text.clone())
                                    })
                                    .build(window, cx)
                                }
                            })
                            .child(status_text),
                    ),
            )
    }

    /// A status bar chip that opens the branch picker upward from itself.
    fn branch_picker_chip(
        &self,
        chip: Button,
        trigger: &'static str,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let open = self.branch_picker == Some(trigger);
        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_w_0()
            .when(open, |view| {
                view.child(deferred(
                    anchored()
                        .anchor(Anchor::BottomLeft)
                        .snap_to_window()
                        .child(self.render_branch_picker(true, cx)),
                ))
            })
            .child(chip.ghost().xsmall().w_full().selected(open).on_click(
                cx.listener(move |app, _, window, cx| app.open_branch_picker(trigger, window, cx)),
            ))
    }
}
