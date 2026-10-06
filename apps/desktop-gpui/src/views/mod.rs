//! `NativeApp` render surfaces. `Render for NativeApp` in `main.rs` composes
//! them into the window layout; each surface resolves its own theme tokens.

use gpui_kit::App;

use crate::NativeApp;

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
                self.selected_thread.as_ref().and_then(|id| {
                    self.shell
                        .threads
                        .iter()
                        .find(|thread| &thread.id == id)
                        .and_then(|thread| thread.branch.clone())
                })
            })
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
