use crate::NativeApp;
use gpui_kit::{Context, ScrollHandle, Window};
#[derive(Default)]
pub(crate) struct PickerNavigation {
    pub index: usize,
    pub query: String,
    pub scroll: ScrollHandle,
}
impl NativeApp {
    pub(crate) fn step_branch_picker(&mut self, forward: bool, cx: &mut Context<Self>) {
        let query = self.branch_query.read(cx).value().to_string();
        if query != self.git_picker.query {
            self.git_picker.index = 0;
            self.git_picker.query = query;
        }
        let count = self.branch_picker_rows(cx).len();
        self.git_picker.index =
            t3code_desktop_gpui::palette::step(self.git_picker.index, count, forward);
        self.git_picker.scroll.scroll_to_item(self.git_picker.index);
        cx.notify();
    }
    pub(crate) fn activate_branch_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let index = if self.git_picker.query == self.branch_query.read(cx).value().as_str() {
            self.git_picker.index
        } else {
            0
        };
        if let Some(row) = self.branch_picker_rows(cx).get(index) {
            let branch = row.name.clone();
            let path = row.worktree_path.clone();
            self.pick_checkout(branch, path, window, cx);
        }
    }
}
