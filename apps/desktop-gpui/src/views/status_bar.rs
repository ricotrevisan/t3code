use gpui_kit::base::Selectable;
use gpui_kit::component::{
    ActiveTheme as _, Sizable,
    button::{Button, ButtonVariants as _},
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
        let status_worktree = self
            .selected_thread_worktree()
            .as_deref()
            .and_then(|path| {
                path.trim_end_matches('/')
                    .rsplit('/')
                    .find(|segment| !segment.is_empty())
            })
            .map(str::to_owned);
        let status_is_error = notifications::is_error(&self.status);
        div()
            .id("status-bar")
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
                    .gap_2()
                    .min_w_0()
                    .child(div().w(px(8.)).h(px(8.)).rounded_full().bg(
                        if self.commands.is_some() {
                            green
                        } else {
                            muted_foreground
                        },
                    ))
                    .child(
                        Button::new("environment-chip")
                            .debug_selector(|| "environment-chip".into())
                            .icon(gpui_kit::assets::IconName::Settings)
                            .label(environment_host)
                            .ghost()
                            .small()
                            .tooltip("Open connection settings")
                            .on_click(cx.listener(|app, _, _, cx| {
                                app.settings_open = true;
                                cx.notify();
                            })),
                    )
                    .when_some(status_worktree, |view, name| {
                        view.child(
                            self.branch_picker_chip(
                                Button::new("worktree-chip")
                                    .debug_selector(|| "worktree-chip".into())
                                    .icon(gpui_kit::assets::IconName::FolderGit)
                                    .label(format!("Worktree · {name}"))
                                    .tooltip("Switch branch or worktree"),
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
                                    .label(branch)
                                    .tooltip("Switch branch or worktree"),
                                "branch-chip",
                                cx,
                            ),
                        )
                    })
                    .when(!active_model.is_empty(), |view| {
                        view.child(div().truncate().child(active_model))
                    }),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .flex_shrink_0()
                    .when(busy, |view| {
                        view.child(div().text_color(warning).child("Working…"))
                    })
                    .child(
                        div()
                            .max_w(px(360.))
                            .truncate()
                            .text_color(if status_is_error {
                                danger
                            } else {
                                muted_foreground
                            })
                            .child(self.status.clone()),
                    )
                    .child(if self.commands.is_some() {
                        "Connected"
                    } else {
                        "Offline"
                    }),
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
            .when(open, |view| {
                view.child(deferred(
                    anchored()
                        .anchor(Anchor::BottomLeft)
                        .snap_to_window()
                        .child(self.render_branch_picker(true, cx)),
                ))
            })
            .child(chip.ghost().small().selected(open).on_click(
                cx.listener(move |app, _, window, cx| app.open_branch_picker(trigger, window, cx)),
            ))
    }
}
