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
    let contexts = context_tokens(text);
    for (range, token) in files.chain(skills).chain(contexts).take(200) {
        if let Ok(next) = content.clone().with_token(range, token) {
            content = next;
        }
    }
    content
}
/// Insert source as native edits, preserving token history and surrounding annotations.
/// Kit 0.7.1 exposes individual atomic edits, not a multi-token transaction;
/// undo traverses the pasted segments without losing the pre-paste history.
pub(crate) fn insert_prompt_content(
    input: &mut gpui_kit::component::input::TextareaState,
    text: &str,
    window: &mut gpui_kit::Window,
    cx: &mut gpui_kit::Context<gpui_kit::component::input::TextareaState>,
) {
    let content = prompt_content(text);
    if !input.selected_range().is_empty() {
        input.replace("", window, cx);
    }
    let mut offset = 0;
    for span in content.tokens() {
        let range = span.range();
        if offset < range.start {
            input.replace(&text[offset..range.start], window, cx);
        }
        let _ = input.replace_with_token(span.token().clone(), window, cx);
        offset = range.end;
    }
    if offset < text.len() {
        input.replace(&text[offset..], window, cx);
    }
}

/// Canonical source is the clipboard/undo representation; Kit owns atomic caret editing.
fn context_tokens(text: &str) -> Vec<(std::ops::Range<usize>, InlineToken)> {
    t3code_desktop_gpui::draft_context::context_links(text)
        .into_iter()
        .map(|(range, href, label)| {
            let token = InlineToken::new(href, text[range.clone()].to_owned()).with_label(label);
            (range, token)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn r18_c_escaped_unicode_context_labels_rehydrate_without_partial_chips() {
        let record = t3code_desktop_gpui::draft_context::DraftContextRecord::terminal(
            "Build [日本語] \\ output",
            "t",
            "Build",
            1,
            1,
            "output",
        )
        .unwrap();
        let text = format!("Before {} after", record.reference());
        let content = prompt_content(&text);
        assert_eq!(content.tokens().len(), 1);
        let span = &content.tokens()[0];
        assert_eq!(&text[span.range()], record.reference());
        assert_eq!(span.token().label().as_ref(), record.label());
    }

    #[test]
    fn r18_c_escaped_open_bracket_is_literal_not_a_context_chip() {
        let text = r"\[Build](t3-context://v1/terminal/ctx_1)";
        assert!(prompt_content(text).tokens().is_empty());
    }
}
use gpui_kit::{Entity, Subscription, Task};

#[derive(Default)]
pub(crate) struct ComposerState {
    pub(crate) catalog_search: Option<Entity<InputState>>,
    pub(crate) catalog_subscription: Option<Subscription>,
    pub(crate) catalog_index: usize,
    pub(crate) catalog_query: String,
    pub(crate) catalog_provider: Option<String>,
    pub(crate) catalog_favorites_only: bool,
    pub(crate) catalog_show_hidden: bool,
    pub(crate) model_preferences:
        std::collections::HashMap<String, Vec<t3code_desktop_gpui::drafts::ModelPreference>>,
    pub(crate) show_skills_in_slash_menu: Option<bool>,
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
    pub(crate) preview_loading: std::collections::HashSet<(String, String)>,
    pub(crate) preview_errors: std::collections::HashSet<(String, String)>,
    pub(crate) expanded_preview: Option<(String, String)>,
    pub(crate) previews:
        std::collections::HashMap<(String, String), std::sync::Arc<gpui_kit::Image>>,
    retired_previews: std::cell::RefCell<Vec<std::sync::Arc<gpui_kit::Image>>>,
    pub(crate) pending_paste: Option<(crate::app::attachments::AttachmentScope, String)>,
    pub(crate) plan_intents:
        std::collections::HashMap<(String, String), t3code_desktop_gpui::drafts::PlanIntent>,
    pub(crate) plan_confirmations: std::collections::HashMap<String, (u64, String, bool)>,
    pub(crate) contexts: std::collections::HashMap<
        (String, String),
        Vec<t3code_desktop_gpui::draft_context::DraftContextRecord>,
    >,
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
