use gpui_kit::base::Selectable;
use gpui_kit::component::{
    ActiveTheme as _, Sizable,
    button::{Button, ButtonVariants as _},
    tooltip::Tooltip,
};
use gpui_kit::{Anchor, Context, anchored, deferred, div, prelude::*, px};
use t3code_desktop_gpui::{connection::Phase, notifications};

use crate::NativeApp;

impl NativeApp {
    pub(crate) fn render_status_bar(
        &self,
        window: &gpui_kit::Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
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
        let environment_label = format!(
            "{} · {}",
            self.environment_name
                .as_deref()
                .unwrap_or(&environment_host),
            self.connection_state_text()
        );
        // Size only the identity's contents, not a proportion of the whole bar.
        // The text system caches this shape; extra space is kit xs icon/padding.
        let environment_width = window
            .text_system()
            .shape_line(
                environment_label.clone().into(),
                window.rem_size() * 0.75,
                &[gpui_kit::TextRun {
                    len: environment_label.len(),
                    font: gpui_kit::font(cx.theme().font_family.clone()),
                    color: muted_foreground,
                    background_color: None,
                    underline: None,
                    strikethrough: None,
                }],
                None,
            )
            .width
            + window.rem_size() * 1.75;
        let worktree_path = self.selected_thread_worktree();
        let status_worktree = worktree_path
            .as_deref()
            .and_then(|path| {
                path.trim_end_matches('/')
                    .rsplit('/')
                    .find(|segment| !segment.is_empty())
            })
            .map(str::to_owned);
        let status_is_error = self.status_kind == notifications::Outcome::Error;
        let status_text = self.status_text();
        div()
            .id("status-bar")
            .debug_selector(|| "status-bar".into())
            .flex()
            .items_center()
            .justify_start()
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
                    // Pack identity chips at the left; the message owns all spare width.
                    .gap_1()
                    .flex_shrink_1()
                    .min_w_0()
                    .child(
                        div()
                            .id("connection-status-dot")
                            .debug_selector(|| "connection-status-dot".into())
                            .flex_shrink_0()
                            .w(px(8.))
                            .h(px(8.))
                            .rounded_full()
                            .tooltip({
                                let text = format!(
                                    "{} · {}",
                                    self.connection_state_text(),
                                    self.environment_scope
                                );
                                move |window, cx| {
                                    let text = text.clone();
                                    Tooltip::element(move |_, _| div().child(text.clone()))
                                        .build(window, cx)
                                }
                            })
                            .bg(match &self.connection_phase {
                                Phase::Ready if self.connection_ready() => green,
                                Phase::Connecting
                                | Phase::Authenticating
                                | Phase::Syncing
                                | Phase::Reconnecting { .. } => warning,
                                Phase::Error(_) => danger,
                                _ => muted_foreground,
                            }),
                    )
                    .child(
                        div()
                            .w(environment_width.min(window.rem_size() * 20.))
                            .min_w(px(24.))
                            .flex_shrink_1()
                            .child(
                                Button::new("environment-chip")
                                    .debug_selector(|| "environment-chip".into())
                                    .icon(gpui_kit::assets::IconName::Settings)
                                    .w_full()
                                    .label(environment_label)
                                    .ghost()
                                    .xsmall()
                                    .tooltip(format!(
                                        "Open connection settings · {} · {}",
                                        self.connection_state_text(),
                                        self.environment_scope
                                    ))
                                    .on_click(cx.listener(|app, _, window, cx| {
                                        app.open_settings("", window, cx)
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
                        view.child(
                            div()
                                .max_w(gpui_kit::rems(5.))
                                .min_w_0()
                                .flex_shrink_1()
                                .truncate()
                                .child(active_model),
                        )
                    }),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .flex_1()
                    .min_w(px(if busy { 166. } else { 96. }))
                    .when(
                        matches!(self.connection_phase, Phase::Reconnecting { .. }),
                        |view| {
                            view.child(
                                Button::new("connection-retry-now")
                                    .debug_selector(|| "connection-retry-now".into())
                                    .label("Retry now")
                                    .ghost()
                                    .xsmall()
                                    .on_click(
                                        cx.listener(|app, _, _, cx| app.retry_connection_now(cx)),
                                    ),
                            )
                        },
                    )
                    .when(busy, |view| {
                        view.child(div().flex_shrink_0().text_color(warning).child("Working…"))
                    })
                    .child(
                        div()
                            .id("status-message")
                            .debug_selector(|| "status-message".into())
                            .flex_1()
                            .min_w(px(96.))
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
            .w(gpui_kit::rems(5.))
            .min_w(px(24.))
            .flex_shrink_1()
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
