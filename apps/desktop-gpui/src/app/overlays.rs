//! Command palette, thread menu, overlay dismissal, and side-panel layout.

use crate::{InspectorSection, NativeApp};
use gpui_kit::component::resizable::ResizableState;
use gpui_kit::{AppContext as _, Context, Focusable as _, Window};
use serde_json::json;
use t3code_desktop_gpui::{commands, palette};

impl NativeApp {
    /// Closes the topmost open overlay; false when there was nothing to close.
    pub(crate) fn dismiss_overlay(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if self.palette_open {
            self.close_palette(window, cx);
            return true;
        }
        if self.branch_picker.is_some() {
            self.close_branch_picker(window, cx);
            return true;
        }
        if self.thread_menu_open {
            self.close_thread_menu();
        } else if self.catalog_open {
            self.catalog_open = false;
        } else if self.settings_open {
            self.settings_open = false;
        } else {
            return false;
        }
        cx.notify();
        true
    }

    pub(crate) fn toggle_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.palette_open {
            self.close_palette(window, cx);
            return;
        }
        self.previous_focus = window.focused(cx);
        self.palette_open = true;
        self.palette_index = 0;
        self.palette_query.update(cx, |input, cx| {
            input.set_value("", window, cx);
            input.focus(window, cx);
        });
        cx.notify();
    }

    /// Opens the branch picker under `trigger` with its search focused, and
    /// reloads the branch listing.
    pub(crate) fn open_branch_picker(
        &mut self,
        trigger: &'static str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.close_thread_menu();
        self.catalog_open = false;
        self.previous_focus = window.focused(cx);
        self.branch_picker = Some(trigger);
        self.branch_query.update(cx, |input, cx| {
            input.set_value("", window, cx);
            input.focus(window, cx);
        });
        self.worktree_operation("vcs.listRefs", false, cx);
        cx.notify();
    }

    pub(crate) fn close_branch_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.branch_picker = None;
        if let Some(focus) = self.previous_focus.take() {
            window.focus(&focus, cx);
        }
        cx.notify();
    }

    pub(crate) fn close_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.palette_open = false;
        if let Some(focus) = self.previous_focus.take() {
            window.focus(&focus, cx);
        }
        cx.notify();
    }

    pub(crate) fn move_palette(&mut self, forward: bool, cx: &mut Context<Self>) {
        let count = palette::matching(
            &self.palette_query.read(cx).value(),
            self.selected_thread.is_some(),
        )
        .len();
        self.palette_index = palette::step(self.palette_index, count, forward);
        cx.notify();
    }

    pub(crate) fn execute_command(
        &mut self,
        command: palette::Command,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        use palette::Command;
        if command == Command::Delete
            && self
                .pending_threads
                .iter()
                .any(|draft| Some(&draft.thread.id) == self.selected_thread.as_ref())
        {
            self.discard_pending(window, cx);
            return;
        }
        match command {
            Command::SaveProjectDefault => self.update_project_default(false, cx),
            Command::ClearProjectDefault => self.update_project_default(true, cx),
            Command::InheritProjectDefault => {
                self.project_actions(Some(json!({"operation":"model-inherit"})), cx)
            }
            Command::ProjectActions => {
                self.reveal_inspector(InspectorSection::ProjectActions, window, cx)
            }
            Command::Worktrees => self.reveal_inspector(InspectorSection::Worktrees, window, cx),
            Command::CloneProject => self.reveal_inspector(InspectorSection::Clone, window, cx),
            Command::Files => {
                self.reveal_inspector(InspectorSection::Files, window, cx);
                self.file_path
                    .update(cx, |input, cx| input.focus(window, cx));
            }
            Command::ContentSearch => self.reveal_inspector(InspectorSection::Files, window, cx),
            Command::AddProject => {
                self.reveal_inspector(InspectorSection::Project, window, cx);
                match commands::create_project(
                    &self.title.read(cx).value(),
                    &self.workspace.read(cx).value(),
                ) {
                    Ok(command) => self.dispatch(command, cx),
                    Err(error) => {
                        self.set_status(error, cx);
                        cx.notify();
                    }
                }
            }
            Command::RenameProject => self.update_project(false, cx),
            Command::RemoveProject => self.update_project(true, cx),
            Command::NewThread => self.new_thread(window, cx),
            Command::Search => self.search.update(cx, |input, cx| input.focus(window, cx)),
            Command::CustomSnooze => self.reveal_inspector(InspectorSection::Thread, window, cx),
            Command::Stash => self.stash_prompt(false, window, cx),
            Command::RestoreStash => self.stash_prompt(true, window, cx),
            Command::Connect => self.connect(cx),
            Command::Disconnect => self.disconnect(cx),
            Command::Settings => {
                self.settings_open = true;
                cx.notify();
            }
            Command::ToggleSidebar => self.set_sidebar_open(!self.sidebar_open, window, cx),
            Command::Rename => {
                self.reveal_inspector(InspectorSection::Project, window, cx);
                self.lifecycle("thread.meta.update", cx);
            }
            _ => self.lifecycle(
                match command {
                    Command::Archive => "thread.archive",
                    Command::Unarchive => "thread.unarchive",
                    Command::Settle => "thread.settle",
                    Command::Unsettle => "thread.unsettle",
                    Command::Snooze => "thread.snooze",
                    Command::Unsnooze => "thread.unsnooze",
                    Command::Pin => "thread.pin",
                    Command::Unpin => "thread.unpin",
                    Command::Delete => "thread.delete",
                    Command::Interrupt => "thread.turn.interrupt",
                    Command::Stop => "thread.session.stop",
                    _ => unreachable!(),
                },
                cx,
            ),
        }
    }

    /// Hides or shows the sidebar. Focus held by a sidebar input moves to the composer
    /// rather than leaving the window without a focused view.
    pub(crate) fn set_sidebar_open(
        &mut self,
        open: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let sidebar_focused = self.search.read(cx).focus_handle(cx).is_focused(window);
        if !open && sidebar_focused {
            self.composer
                .update(cx, |input, cx| input.focus(window, cx));
        }
        self.sidebar_open = open;
        self.remember_layout();
        self.reset_splits(cx);
        cx.notify();
    }

    /// A split rescales every panel in proportion when its container resizes; a fresh
    /// state lays the side panels out at their remembered widths instead.
    pub(crate) fn reset_splits(&mut self, cx: &mut Context<Self>) {
        self.sidebar_split = cx.new(|_| ResizableState::default());
        self.workspace_split = cx.new(|_| ResizableState::default());
    }

    pub(crate) fn set_inspector_open(&mut self, open: bool, cx: &mut Context<Self>) {
        self.inspector_open = open;
        self.remember_layout();
        cx.notify();
    }

    /// Mirrors the panel layout into the state written on shutdown.
    pub(crate) fn remember_layout(&mut self) {
        if let Some(state) = &mut self.saved_window {
            state.sidebar_open = self.sidebar_open;
            state.sidebar_width = self.sidebar_width;
            state.right_panel_open = self.inspector_open;
            state.right_panel_width = self.inspector_width;
        }
    }

    pub(crate) fn close_thread_menu(&mut self) {
        self.thread_menu_open = false;
        self.thread_menu_row = None;
        self.confirm_delete = None;
    }

    /// The thread the open thread menu acts on: its sidebar row, else the selection.
    pub(crate) fn thread_menu_target(&self) -> Option<String> {
        self.thread_menu_row
            .as_ref()
            .map(|(id, _)| id.clone())
            .or_else(|| self.selected_thread.clone())
    }

    /// Runs a thread menu entry on the menu's target thread. Rename from a row
    /// selects that thread and focuses the inspector title, since the title field
    /// carries the new name.
    pub(crate) fn run_thread_menu_action(
        &mut self,
        kind: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(target) = self.thread_menu_target() else {
            self.close_thread_menu();
            cx.notify();
            return;
        };
        let from_row = self.thread_menu_row.is_some();
        if from_row && kind == "thread.meta.update" {
            self.close_thread_menu();
            if self.selected_thread.as_ref() != Some(&target) {
                self.select_thread(&target, window, cx);
            }
            self.reveal_inspector(InspectorSection::Project, window, cx);
            self.set_status("Edit the title, then choose Rename from More", cx);
            cx.notify();
            return;
        }
        let confirming = self.confirm_delete.as_ref() == Some(&target);
        if !from_row
            && kind == "thread.delete"
            && self
                .pending_threads
                .iter()
                .any(|draft| draft.thread.id == target)
        {
            self.discard_pending(window, cx);
        } else {
            self.thread_lifecycle(&target, kind, cx);
        }
        // Delete keeps the menu open so the second click confirms.
        if kind != "thread.delete" || confirming {
            self.close_thread_menu();
        }
        cx.notify();
    }
}
