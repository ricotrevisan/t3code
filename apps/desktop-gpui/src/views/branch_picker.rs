use gpui_kit::assets::IconName;
use gpui_kit::component::{
    ActiveTheme as _, Disableable, Icon, Selectable, Sizable, input::Input, separator::Separator,
    tooltip::Tooltip,
};
use gpui_kit::{App, Context, div, prelude::*, px};
use t3code_desktop_gpui::workspaces;

use super::list_row;
use crate::app::branch_workflows::BranchWorkflow;
use crate::{InspectorSection, NativeApp};
use gpui_kit::component::button::{Button, ButtonVariants};

impl NativeApp {
    pub(crate) fn branch_picker_rows(&self, cx: &App) -> Vec<&workspaces::Ref> {
        workspaces::picker_rows(
            &self.refs,
            &self.branch_query.read(cx).value(),
            self.selected_thread_branch().as_deref(),
        )
    }

    /// The branch and worktree picker, anchored by whichever chip opened it; `upward`
    /// when it opens above that chip. Rows switch the selection's checkout;
    /// worktrees are managed in the Inspector.
    pub(crate) fn render_branch_picker(
        &self,
        upward: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let muted_foreground = cx.theme().muted_foreground;
        let recovery = self.checkout_recovery().map(|op| {
            let text = if op.running {
                "Checkout receipt/reconciliation pending; message retained".to_owned()
            } else {
                op.error
                    .clone()
                    .unwrap_or_else(|| "Checkout outcome retained".into())
            };
            div()
                .flex()
                .flex_col()
                .gap_2()
                .child(
                    div()
                        .debug_selector(|| "checkout-recovery-status".into())
                        .text_sm()
                        .child(text),
                )
                .when_some(op.destination.as_ref(), |view, destination| {
                    view.child(div().text_xs().child(format!(
                        "{} · {}",
                        destination.branch.as_deref().unwrap_or("detached"),
                        destination.path.as_deref().unwrap_or(&op.root)
                    )))
                })
                .child(
                    div()
                        .flex()
                        .gap_2()
                        .child(
                            Button::new("checkout-recovery-retry")
                                .debug_selector(|| "checkout-recovery-retry".into())
                                .small()
                                .primary()
                                .label("Adopt / retry metadata")
                                .disabled(op.running || op.destination.is_none())
                                .on_click(cx.listener(|app, _, window, cx| {
                                    app.retry_checkout_metadata(window, cx)
                                })),
                        )
                        .child(
                            Button::new("checkout-recovery-dismiss")
                                .debug_selector(|| "checkout-recovery-dismiss".into())
                                .small()
                                .ghost()
                                .label("Dismiss known outcome")
                                .disabled(!op.safely_dismissible())
                                .on_click(
                                    cx.listener(|app, _, _, cx| app.dismiss_checkout_outcome(cx)),
                                ),
                        ),
                )
                .child(
                    Button::new("checkout-recovery-refresh")
                        .small()
                        .ghost()
                        .label("Inspect actual refs")
                        .disabled(op.running)
                        .on_click(cx.listener(|app, _, window, cx| {
                            app.show_branch_workflow(BranchWorkflow::Browse, window, cx);
                            app.worktree_operation("vcs.listRefs", false, cx);
                        })),
                )
                .into_any_element()
        });
        if self.branch_workflow != BranchWorkflow::Browse {
            let page = self.branch_workflow;
            let busy = self.branch_workflow_busy;
            let restriction = self.branch_workflow_restriction().or_else(|| {
                (page == BranchWorkflow::Environment)
                    .then(|| self.branch_environment_restriction())
                    .flatten()
            });
            let title = match page {
                BranchWorkflow::Create => "New branch",
                BranchWorkflow::PullRequest => "Checkout PR",
                _ => "Draft environment",
            };
            return div()
                .id("branch-picker")
                .debug_selector(|| "branch-picker".into())
                .key_context("NativeBranchPicker")
                .occlude()
                .map(|view| if upward { view.mb_1() } else { view.mt_1() })
                .w(px(300.)).flex().flex_col().gap_2().p_2().rounded_md()
                .bg(cx.theme().popover).border_1().border_color(cx.theme().border).shadow_lg()
                .child(div().text_sm().child(title))
                .when_some(recovery, |view, recovery| view.child(recovery))
                .child(Input::new(&self.branch_workflow_input).small().disabled(busy || self.checkout_transition_pending()))
                .when(page == BranchWorkflow::Environment, |view| view
                    .child(div().debug_selector(|| "draft-environment-mode".into()).text_xs().child(if self.prepare_new_worktree { "Current mode: New worktree (saved base shown above)" } else { "Current mode: Selected checkout / Local" }))
                    .child(div().text_xs().text_color(muted_foreground).child(
                        "Unsent drafts only. Local uses the root, never the typed base. Worktree prepares from the saved base when sent."
                    )))
                .when(busy, |view| view.child(div().text_sm().child("Preparing checkout…")))
                .when_some(self.branch_workflow_error.as_deref().or(restriction), |view, error| view.child(
                    div().debug_selector(|| "branch-workflow-error".into()).text_sm().text_color(cx.theme().danger).child(error.to_owned())
                ))
                .child(div().flex().gap_2()
                    .child(Button::new("branch-workflow-back").debug_selector(|| "branch-workflow-back".into()).ghost().small().label(if busy { "Cancel" } else { "Back" })
                        .on_click(cx.listener(|app, _, window, cx| app.show_branch_workflow(BranchWorkflow::Browse, window, cx))))
                    .when(page == BranchWorkflow::Create, |view| view.child(
                        Button::new("branch-workflow-create").debug_selector(|| "branch-workflow-create".into()).primary().small().label(if self.branch_workflow_error.is_some() { "Retry" } else { "Create branch" })
                            .disabled(busy || restriction.is_some() || self.checkout_transition_pending())
                            .on_click(cx.listener(|app, _, window, cx| app.run_branch_workflow(false, window, cx)))
                    ))
                    .when(page != BranchWorkflow::Create, |view| view
                        .child(Button::new("branch-workflow-local").debug_selector(|| "branch-workflow-local".into()).ghost().small().label("Local").selected(page == BranchWorkflow::Environment && !self.prepare_new_worktree).disabled(busy || restriction.is_some() || self.checkout_transition_pending())
                            .on_click(cx.listener(move |app, _, window, cx| {
                                if page == BranchWorkflow::Environment { app.select_branch_environment(false, window, cx); }
                                else { app.run_branch_workflow(false, window, cx); }
                            })))
                        .child(Button::new("branch-workflow-worktree").debug_selector(|| "branch-workflow-worktree".into()).primary().small().label("Worktree").selected(page == BranchWorkflow::Environment && self.prepare_new_worktree).disabled(busy || restriction.is_some() || self.checkout_transition_pending())
                            .on_click(cx.listener(move |app, _, window, cx| {
                                if page == BranchWorkflow::Environment { app.select_branch_environment(true, window, cx); }
                                else { app.run_branch_workflow(true, window, cx); }
                            }))))
                )
                .into_any_element();
        }
        let current = self.selected_thread_branch();
        let rows = self.branch_picker_rows(cx);
        let loading = self.worktree_busy && self.refs.is_empty();
        let notice = if loading {
            Some(("Loading branches…".to_owned(), false))
        } else if let Some(error) = &self.refs_error {
            Some((error.clone(), true))
        } else if self.refs.is_empty() {
            Some(("No branches found".to_owned(), false))
        } else if rows.is_empty() {
            Some(("No matching branches".to_owned(), false))
        } else {
            None
        };
        let rows = rows
            .into_iter()
            .enumerate()
            .map(|(index, row)| {
                let id = format!("branch-ref-{index}");
                let branch = row.name.clone();
                let path = row.worktree_path.clone();
                let is_current = workspaces::is_current(row, current.as_deref());
                list_row(id, None, row.name.clone(), false)
                    .check_icon(IconName::Check)
                    .confirmed(is_current)
                    .when_some(row.worktree_path.clone(), |item, path| {
                        item.tooltip(move |window, cx| Tooltip::new(path.clone()).build(window, cx))
                            .suffix(|_, _| Icon::new(IconName::FolderGit).xsmall())
                    })
                    .on_click(cx.listener(move |app, _, window, cx| {
                        app.pick_checkout(branch.clone(), path.clone(), window, cx)
                    }))
            })
            .collect::<Vec<_>>();
        div()
            .id("branch-picker")
            .debug_selector(|| "branch-picker".into())
            .key_context("NativeBranchPicker")
            .occlude()
            .map(|view| if upward { view.mb_1() } else { view.mt_1() })
            .w(px(300.))
            .flex()
            .flex_col()
            .gap_1()
            .p_2()
            .rounded_md()
            .bg(cx.theme().popover)
            .border_1()
            .border_color(cx.theme().border)
            .shadow_lg()
            .when_some(recovery, |view, recovery| view.child(recovery))
            .child(Input::new(&self.branch_query).small())
            .child(
                div()
                    .id("branch-picker-rows")
                    .max_h(px(320.))
                    .overflow_y_scroll()
                    .flex()
                    .flex_col()
                    .when_some(notice, |view, (text, is_error)| {
                        view.child(
                            div()
                                .debug_selector(|| "branch-picker-notice".into())
                                .px_2()
                                .py_1()
                                .text_sm()
                                .text_color(if is_error {
                                    cx.theme().danger
                                } else {
                                    muted_foreground
                                })
                                .child(text),
                        )
                    })
                    .children(rows)
                    .when(self.refs_cursor.is_some(), |view| {
                        view.child(
                            list_row("branch-picker-more", None, "More branches", false).on_click(
                                cx.listener(|app, _, _, cx| {
                                    app.worktree_operation("vcs.listRefs", true, cx)
                                }),
                            ),
                        )
                    }),
            )
            .child(Separator::horizontal())
            .children(
                [
                    ("branch-picker-new", "New branch…", BranchWorkflow::Create),
                    (
                        "branch-picker-pr",
                        "Checkout PR…",
                        BranchWorkflow::PullRequest,
                    ),
                    (
                        "branch-picker-environment",
                        "Local or Worktree…",
                        BranchWorkflow::Environment,
                    ),
                ]
                .into_iter()
                .map(|(id, label, page)| {
                    list_row(id, None, label, false).on_click(cx.listener(
                        move |app, _, window, cx| app.show_branch_workflow(page, window, cx),
                    ))
                }),
            )
            .child(
                list_row("branch-picker-manage", None, "Manage worktrees…", false).on_click(
                    cx.listener(|app, _, window, cx| {
                        app.close_branch_picker(window, cx);
                        app.reveal_inspector(InspectorSection::Worktrees, window, cx);
                    }),
                ),
            )
            .into_any_element()
    }
}
