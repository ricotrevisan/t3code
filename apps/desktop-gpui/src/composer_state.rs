//! Device-local composer interaction state, never sent as thread metadata.
use gpui_kit::component::input::{InlineToken, InputContent, InputState};

/// Rehydrate annotations without losing the serializable prompt source.
pub(crate) fn prompt_content(text: &str) -> InputContent {
    let mut content = InputContent::new(text.to_owned());
    let files = t3code_desktop_gpui::mentions::file_tokens(text)
        .into_iter()
        .map(|(range, path)| {
            let label = path.rsplit(['/', '\\']).next().unwrap_or(&path);
            let token = InlineToken::new(format!("mention:{path}"), text[range.clone()].to_owned())
                .with_label(format!("@{label}"));
            (range, token)
        });
    let skills = t3code_desktop_gpui::mentions::skill_tokens(text)
        .into_iter()
        .map(|(range, name)| {
            let token = InlineToken::new(format!("skill:{name}"), text[range.clone()].to_owned());
            (range, token)
        });
    for (range, token) in files.chain(skills).take(200) {
        if let Ok(next) = content.clone().with_token(range, token) {
            content = next;
        }
    }
    content
}
use gpui_kit::{Entity, Subscription, Task};

#[derive(Default)]
pub(crate) struct ComposerState {
    pub(crate) catalog_search: Option<Entity<InputState>>,
    pub(crate) catalog_subscription: Option<Subscription>,
    pub(crate) catalog_index: usize,
    pub(crate) catalog_query: String,
    pub(crate) catalog_provider: Option<String>,
    pub(crate) catalog_scroll: gpui_kit::ScrollHandle,
    pub(crate) catalog_anchor: Option<gpui_kit::ScrollAnchor>,
    pub(crate) stash_picker: bool,
    pub(crate) stash_index: usize,
    pub(crate) stash_focus: Option<gpui_kit::FocusHandle>,
    pub(crate) history_target: Option<(String, String)>,
    pub(crate) history_id: Option<String>,
    pub(crate) history_draft: String,
    pub(crate) history_recalled: String,
    pub(crate) send_shortcut: t3code_desktop_gpui::drafts::SendShortcut,
    pub(crate) command_scope: Option<(u64, String, String, String)>,
    pub(crate) command_task: Option<Task<()>>,
    pub(crate) command_loading: bool,
    pub(crate) command_error: Option<String>,
    pub(crate) previews:
        std::collections::HashMap<(String, String), std::sync::Arc<gpui_kit::Image>>,
    retired_previews: std::cell::RefCell<Vec<std::sync::Arc<gpui_kit::Image>>>,
    pub(crate) pending_paste: Option<(crate::app::attachments::AttachmentScope, String)>,
}

impl ComposerState {
    /// Preview bodies are expendable; uploaded-reference ownership is not.
    /// Keep this bounded across threads, environments and long-lived stashes.
    pub(crate) fn cache_preview(
        &mut self,
        origin: String,
        id: String,
        image: std::sync::Arc<gpui_kit::Image>,
    ) {
        const LIMIT: usize = 32 * 1024 * 1024;
        if image.bytes.len() > LIMIT {
            return;
        }
        let key = (origin, id);
        self.retire_preview(&key);
        let mut retained: usize = self
            .previews
            .values()
            .map(|preview| preview.bytes.len())
            .sum();
        while retained + image.bytes.len() > LIMIT || self.previews.len() >= 128 {
            let Some(old) = self.previews.keys().next().cloned() else {
                break;
            };
            if let Some(removed) = self.previews.remove(&old) {
                retained -= removed.bytes.len();
                self.retired_previews.borrow_mut().push(removed);
            }
        }
        self.previews.insert(key, image);
    }
    pub(crate) fn retire_preview(&mut self, key: &(String, String)) {
        if let Some(image) = self.previews.remove(key) {
            self.retired_previews.borrow_mut().push(image);
        }
    }
    pub(crate) fn release_retired_previews(
        &self,
        cx: &mut gpui_kit::App,
        shared: impl Fn(&gpui_kit::Image) -> bool,
    ) {
        for image in self.retired_previews.take() {
            // Equal content shares GPUI's hash-keyed asset; retain other local owners.
            if !self.previews.values().any(|live| live.id() == image.id()) && !shared(&image) {
                image.remove_asset(cx);
            }
        }
    }
}
