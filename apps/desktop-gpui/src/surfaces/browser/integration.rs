//! Additive shell hook: hide native children even when the panel is not rendered.
use crate::NativeApp;
use gpui_kit::Context;

impl NativeApp {
    pub(crate) fn sync_browser_panel(&mut self, cx: &mut Context<Self>) {
        let covered = self.header_rename.overflow_open
            || self.git_header.open
            || self.editor_choice.open
            || self.settings_open
            || self.palette_open
            || self.catalog_open
            || self.branch_picker.is_some()
            || self.thread_menu_open
            || self.confirm_delete.is_some()
            || self.confirm_revert.is_some()
            || self.confirm_project_removal.is_some()
            || self.confirm_worktree_removal.is_some();
        let body_available = matches!(
            self.right_tab,
            crate::RightTab::Inspector | crate::RightTab::Changes | crate::RightTab::Agents | crate::RightTab::PullRequests
        );
        self.browser_panel.update(cx, |panel, cx| {
            panel.sync_scope(&self.environment_scope, cx);
            panel.sync_visibility(self.inspector_open && body_available, covered);
        });
    }
}
