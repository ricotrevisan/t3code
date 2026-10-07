use gpui_kit::assets::IconName;
use gpui_kit::base::Selectable;
use gpui_kit::component::{
    ActiveTheme as _, Icon, Sizable,
    button::{Button, ButtonVariants as _},
    input::Input,
    separator::Separator,
};
use gpui_kit::{App, Context, div, prelude::*, px};
use t3code_desktop_gpui::workspaces;

use crate::{InspectorSection, NativeApp};

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
                Button::new(gpui_kit::SharedString::from(id.clone()))
                    .debug_selector(move || id.clone())
                    .ghost()
                    .small()
                    .selected(is_current)
                    .when(is_current, |button| button.icon(IconName::Check))
                    .label(row.name.clone())
                    .when_some(row.worktree_path.clone(), |button, path| {
                        button
                            .tooltip(path)
                            .child(Icon::new(IconName::FolderGit).xsmall())
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
            .child(Input::new(&self.branch_query).small())
            .child(
                div()
                    .id("branch-picker-rows")
                    .max_h(px(320.))
                    .overflow_y_scroll()
                    .flex()
                    .flex_col()
                    .gap_1()
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
                            Button::new("branch-picker-more")
                                .debug_selector(|| "branch-picker-more".into())
                                .ghost()
                                .small()
                                .label("More branches")
                                .on_click(cx.listener(|app, _, _, cx| {
                                    app.worktree_operation("vcs.listRefs", true, cx)
                                })),
                        )
                    }),
            )
            .child(Separator::horizontal())
            .child(
                Button::new("branch-picker-manage")
                    .debug_selector(|| "branch-picker-manage".into())
                    .ghost()
                    .small()
                    .label("Manage worktrees…")
                    .on_click(cx.listener(|app, _, window, cx| {
                        app.close_branch_picker(window, cx);
                        app.reveal_inspector(InspectorSection::Worktrees, window, cx);
                    })),
            )
    }
}
