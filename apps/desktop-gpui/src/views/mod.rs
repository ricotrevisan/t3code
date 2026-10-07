//! `NativeApp` render surfaces. `Render for NativeApp` in `main.rs` composes
//! them into the window layout; each surface resolves its own theme tokens.

use gpui_kit::assets::IconName;
use gpui_kit::component::{Icon, Sizable, list::ListItem};
use gpui_kit::{App, Role, SharedString, div, prelude::*};

use crate::NativeApp;

mod branch_picker;
mod catalog;
mod composer;
mod conversation;
mod inspector;
mod overlay;
mod palette;
mod settings;
mod sidebar;
mod status_bar;
mod thread_header;
mod title_bar;
mod toasts;

/// A left-aligned row for menus, pickers and section headers. `selected` marks the
/// keyboard-highlighted or current row with the theme's list-active background
/// and exposes it as the row's selected state. The label keeps the selector
/// `<id>-label`; callers add `on_click`, `disabled` or a trailing check.
pub(crate) fn list_row(
    id: impl Into<SharedString>,
    icon: Option<IconName>,
    label: impl Into<SharedString>,
    selected: bool,
) -> ListItem {
    let id = id.into();
    let label = label.into();
    let label_selector = format!("{id}-label");
    ListItem::new(id.clone())
        .debug_selector(move || id.to_string())
        .role(Role::ListBoxOption)
        .aria_selected(selected)
        .accessibility_label(label.clone())
        .selected(selected)
        .px_2()
        .rounded_md()
        .text_sm()
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .min_w_0()
                .children(icon.map(|icon| Icon::new(icon).xsmall()))
                .child(
                    div()
                        .debug_selector(move || label_selector)
                        .min_w_0()
                        .truncate()
                        .child(label),
                ),
        )
}

impl NativeApp {
    fn selected_thread_title(&self) -> String {
        self.detail
            .thread
            .as_ref()
            .and_then(|detail| detail["title"].as_str())
            .map(str::to_owned)
            .or_else(|| {
                self.selected_thread.as_ref().and_then(|id| {
                    self.shell
                        .threads
                        .iter()
                        .chain(self.pending_threads.iter().map(|draft| &draft.thread))
                        .find(|thread| &thread.id == id)
                        .map(|thread| thread.title.clone())
                })
            })
            .unwrap_or_else(|| "No thread selected".to_owned())
    }

    fn selected_thread_branch(&self) -> Option<String> {
        self.detail
            .thread
            .as_ref()
            .and_then(|detail| detail["branch"].as_str())
            .map(str::to_owned)
            .or_else(|| {
                self.selected_thread_summary()
                    .and_then(|thread| thread.branch.clone())
            })
    }

    /// The selected thread's worktree path; None when it runs in the project root.
    fn selected_thread_worktree(&self) -> Option<String> {
        self.selected_thread_summary()
            .and_then(|thread| thread.worktree_path.clone())
            .filter(|path| !path.is_empty())
    }

    fn selected_thread_summary(&self) -> Option<&t3code_desktop_gpui::shell::Thread> {
        let id = self.selected_thread.as_ref()?;
        self.shell
            .threads
            .iter()
            .chain(self.pending_threads.iter().map(|draft| &draft.thread))
            .find(|thread| &thread.id == id)
    }

    fn active_model_label(&self, cx: &App) -> String {
        let instance = self.provider.read(cx).value().to_string();
        let model = self.model.read(cx).value().to_string();
        if instance.is_empty() {
            String::new()
        } else if model.is_empty() {
            instance
        } else {
            format!("{instance} · {model}")
        }
    }
}
