//! Canonical rendered-text citations. Byte positions never cross the wire.
use super::*;
use gpui_kit::component::text::{RangeHighlight, TextViewState};
use std::ops::Range;

use crate::conversation_presentation::SourceView;

fn whitespace(c: char) -> bool {
    // ECMAScript /\\s/: match the web selector, including BOM but excluding NEL.
    matches!(
        c,
        '\t' | '\n' | '\u{000b}' | '\u{000c}' | '\r' | ' ' | '\u{00a0}' | '\u{1680}' | '\u{2000}'
            ..='\u{200a}'
                | '\u{2028}'
                | '\u{2029}'
                | '\u{202f}'
                | '\u{205f}'
                | '\u{3000}'
                | '\u{feff}'
    )
}
fn normalize(text: &str) -> String {
    let mut result = String::new();
    let mut space = false;
    for c in text.chars() {
        if whitespace(c) {
            if !space {
                result.push(' ');
            }
            space = true;
        } else {
            result.push(c);
            space = false;
        }
    }
    result
}
fn bounded_suffix(text: &str, limit: usize) -> String {
    let mut chars = Vec::new();
    let mut units = 0;
    for c in text.chars().rev() {
        if units + c.len_utf16() > limit {
            break;
        }
        chars.push(c);
        units += c.len_utf16();
    }
    chars.into_iter().rev().collect()
}
fn bounded_prefix(text: &str, limit: usize) -> String {
    let mut units = 0;
    text.chars()
        .take_while(|c| {
            units += c.len_utf16();
            units <= limit
        })
        .collect()
}

pub(crate) fn selector(text: &str, range: Range<usize>) -> Result<Value, &'static str> {
    let quote = text
        .get(range.clone())
        .ok_or("Selection is no longer available")?;
    if quote.trim().is_empty() {
        return Err("Select assistant text first");
    }
    if quote.encode_utf16().count() > 8000 {
        return Err("Shorten the selection to 8,000 UTF-16 units");
    }
    let normalized = normalize(text);
    let mut before = normalize(&text[..range.start]);
    if range.start > 0
        && text[..range.start]
            .chars()
            .next_back()
            .is_some_and(whitespace)
        && text[range.start..].chars().next().is_some_and(whitespace)
    {
        before.pop();
    }
    let through = normalize(&text[..range.end]);
    let start = before.encode_utf16().count();
    let end = through.encode_utf16().count();
    let suffix = normalized
        .get(through.len()..)
        .ok_or("Invalid selection boundary")?;
    Ok(
        serde_json::json!({"text":quote,"start":start,"end":end,"prefix":bounded_suffix(&before,32),"suffix":bounded_prefix(suffix,32)}),
    )
}

/// Context disambiguates repeated text; saved offsets never break a context tie.
pub(crate) fn resolve(text: &str, citation: &Value) -> Option<Range<usize>> {
    let quote = normalize(citation["text"].as_str()?);
    if quote.trim().is_empty() {
        return None;
    }
    let prefix = normalize(citation["prefix"].as_str()?);
    let suffix = normalize(citation["suffix"].as_str()?);
    let mut normalized = String::new();
    let mut spans = Vec::<(usize, usize, usize, usize)>::new();
    let mut space = false;
    for (offset, c) in text.char_indices() {
        if whitespace(c) && space {
            if let Some(last) = spans.last_mut() {
                last.3 = offset + c.len_utf8();
            }
            continue;
        }
        let start = normalized.len();
        normalized.push(if whitespace(c) { ' ' } else { c });
        spans.push((start, normalized.len(), offset, offset + c.len_utf8()));
        space = whitespace(c);
    }
    let all = normalized
        .char_indices()
        .filter(|(start, _)| normalized[*start..].starts_with(&quote))
        .map(|(start, _)| start..start + quote.len())
        .collect::<Vec<_>>();
    let contextual = all
        .iter()
        .filter(|range| {
            normalized[..range.start].ends_with(&prefix)
                && normalized[range.end..].starts_with(&suffix)
        })
        .collect::<Vec<_>>();
    let found = match contextual.as_slice() {
        [one] => *one,
        [] if all.len() == 1 => &all[0],
        _ => return None,
    };
    let start = spans.iter().find(|span| span.0 == found.start)?.2;
    let end = spans.iter().find(|span| span.1 == found.end)?.3;
    Some(start..end)
}

impl NativeApp {
    pub(super) fn citation_source_view(
        &mut self,
        message: &str,
        markdown: &str,
        cx: &mut Context<Self>,
    ) -> gpui_kit::Entity<TextViewState> {
        if let Some(source) = self.conversation_presentation.sources.get(message) {
            let state = source.state.clone();
            if source.markdown != markdown {
                state.update(cx, |state, cx| state.set_text(markdown, cx));
                self.conversation_presentation
                    .sources
                    .get_mut(message)
                    .unwrap()
                    .markdown = markdown.to_owned();
            }
            self.highlight_citation_source(message, &state, cx);
            return state;
        }
        // Only mounted/recent source views are retained. Remounts parse again, never replay capture.
        if self.conversation_presentation.sources.len() >= 128 {
            let key = self
                .conversation_presentation
                .sources
                .keys()
                .next()
                .cloned()
                .unwrap();
            self.conversation_presentation.sources.remove(&key);
        }
        let state = cx.new(|cx| TextViewState::markdown(markdown, cx));
        let id = message.to_owned();
        let subscription = cx.observe(&state, move |app, state, cx| {
            app.highlight_citation_source(&id, &state, cx);
            cx.notify();
        });
        self.conversation_presentation.sources.insert(
            message.to_owned(),
            SourceView {
                markdown: markdown.to_owned(),
                state: state.clone(),
                _subscription: subscription,
            },
        );
        self.highlight_citation_source(message, &state, cx);
        state
    }

    pub(super) fn cite_assistant_selection(
        &mut self,
        message: &str,
        state: &gpui_kit::Entity<TextViewState>,
        capture_scope: (&str, &str, u64),
        snapshot: Option<(String, Value)>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (scope, thread, generation) = capture_scope;
        let result = (|| {
            if generation != self.generation
                || scope != self.environment_scope
                || Some(thread) != self.selected_thread.as_deref()
            {
                return Err("Return to the captured conversation before citing");
            }
            let view = state.read(cx);
            let rendered = view.rendered_text();
            let mut citation = if let Some((original, citation)) = snapshot {
                if original != rendered.as_str() {
                    return Err("Assistant text changed; select it again");
                }
                citation
            } else {
                let source = view
                    .selected_source_range()
                    .ok_or("Select assistant text first")?;
                let range = rendered
                    .range_for_source(source)
                    .ok_or("Selection cannot be mapped to assistant text")?;
                selector(rendered.as_str(), range)?
            };
            let environment = self
                .server_config
                .config(self.generation)
                .and_then(|config| config["environment"]["environmentId"].as_str())
                .ok_or(
                    "Environment identity is unavailable; refresh the connection before citing",
                )?;
            citation["version"] = serde_json::json!(1);
            citation["environmentId"] = environment.into();
            citation["threadId"] = thread.into();
            citation["messageId"] = message.into();
            let record = t3code_desktop_gpui::draft_context::DraftContextRecord::citation(
                "Assistant quote",
                citation,
            )?;
            self.intake_draft_context(scope, thread, record, window, cx)?;
            state.update(cx, |state, cx| state.clear_selection(cx));
            self.composer
                .update(cx, |input, cx| input.focus(window, cx));
            Ok(())
        })();
        if let Err(error) = result {
            self.report_error(error, cx);
        }
    }

    /// Route through the retained owner page, never copy source history into the
    /// destination page. Normal selection keeps drafts and dirty-file guards.
    pub(crate) fn navigate_to_citation(
        &mut self,
        citation: &Value,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(environment) = citation["environmentId"]
            .as_str()
            .filter(|id| !id.is_empty())
        else {
            self.report_error("Citation environment is unavailable", cx);
            return;
        };
        let current = self
            .server_config
            .config(self.generation)
            .and_then(|config| config["environment"]["environmentId"].as_str());
        if current == Some(environment) {
            self.load_citation_source(citation, window, cx);
            return;
        }
        if !self.can_change_checkout(cx) {
            return;
        }
        let target = self
            .environment_hub
            .borrow()
            .pages
            .iter()
            .find_map(|(origin, page)| {
                let page = page.upgrade()?;
                if page.entity_id() == cx.entity_id() {
                    return None;
                }
                let owner = page.read(cx);
                (owner
                    .server_config
                    .config(owner.generation)
                    .and_then(|config| config["environment"]["environmentId"].as_str())
                    == Some(environment))
                .then(|| (origin.clone(), page.clone()))
            });
        let Some((origin, page)) = target else {
            self.report_error("Citation belongs to another environment or thread; connect its saved environment to open the source", cx);
            return;
        };
        let opened = page.update(cx, |app, cx| app.load_citation_source(citation, window, cx));
        if opened {
            self.present_environment_page(&origin, window, cx);
        } else {
            self.report_error("Citation source could not be opened; check its environment connection and unsaved edits", cx);
        }
    }

    fn load_citation_source(
        &mut self,
        citation: &Value,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(thread) = citation["threadId"].as_str().filter(|id| !id.is_empty()) else {
            self.report_error("Citation source thread is unavailable", cx);
            return false;
        };
        if citation["messageId"].as_str().is_none_or(str::is_empty) {
            self.report_error("Citation source message is unavailable", cx);
            return false;
        }
        if !self.connection_ready() {
            self.report_error(
                "Reconnect the citation's environment before opening its source",
                cx,
            );
            return false;
        }
        self.select_thread(thread, window, cx);
        if self.selected_thread.as_deref() != Some(thread) {
            return false;
        }
        self.conversation_presentation.citation_navigation =
            Some(crate::conversation_presentation::CitationNavigation {
                generation: self.generation,
                environment: self.environment_scope.clone(),
                citation: citation.clone(),
                unavailable: false,
            });
        cx.notify();
        true
    }

    pub(super) fn resolve_citation_navigation(&mut self, cx: &mut Context<Self>) {
        let Some(pending) = self.conversation_presentation.citation_navigation.as_ref() else {
            return;
        };
        if pending.generation != self.generation
            || pending.environment != self.environment_scope
            || pending.citation["threadId"].as_str() != self.selected_thread.as_deref()
        {
            self.conversation_presentation.citation_navigation = None;
            return;
        }
        if pending.unavailable {
            return;
        }
        let citation = pending.citation.clone();
        let found = self
            .detail
            .thread
            .as_ref()
            .and_then(|thread| thread["messages"].as_array())
            .is_some_and(|messages| {
                messages.iter().any(|message| {
                    message["id"] == citation["messageId"] && message["role"] == "assistant"
                })
            });
        if found {
            self.conversation_presentation.citation_navigation = None;
            self.jump_to_citation(&citation, cx);
        } else if self.detail.synchronized {
            self.citation_navigation_failed(citation["threadId"].as_str().unwrap_or_default(), cx);
            self.report_error(
                "Citation source response is unavailable; the saved quote is unchanged",
                cx,
            );
        }
    }

    pub(crate) fn citation_navigation_failed(&mut self, thread: &str, cx: &mut Context<Self>) {
        if let Some(pending) = self.conversation_presentation.citation_navigation.as_mut()
            && pending.generation == self.generation
            && pending.environment == self.environment_scope
            && pending.citation["threadId"] == thread
        {
            pending.unavailable = true;
            cx.notify();
        }
    }

    pub(super) fn render_citation_navigation(&self, cx: &Context<Self>) -> Option<gpui_kit::Div> {
        let pending = self
            .conversation_presentation
            .citation_navigation
            .as_ref()?;
        if pending.generation != self.generation
            || pending.environment != self.environment_scope
            || pending.citation["threadId"].as_str() != self.selected_thread.as_deref()
        {
            return None;
        }
        let unavailable = pending.unavailable;
        let navigation_generation = pending.generation;
        let cancel_citation = pending.citation.clone();
        Some(
            div()
                .debug_selector(|| "citation-source-status".into())
                .flex()
                .flex_col()
                .items_start()
                .min_w_0()
                .gap_1()
                .p_2()
                .child(if unavailable {
                    "Source response unavailable · saved quote preserved"
                } else {
                    "Loading cited conversation…"
                })
                .when(unavailable, |view| {
                    let citation = pending.citation.clone();
                    view.child(
                        Button::new("citation-navigation-retry")
                            .debug_selector(|| "citation-navigation-retry".into())
                            .ghost()
                            .xsmall()
                            .label("Reload source")
                            .on_click(cx.listener(move |app, _, window, cx| {
                                if app.generation != navigation_generation
                                    || !app
                                        .conversation_presentation
                                        .citation_navigation
                                        .as_ref()
                                        .is_some_and(|pending| pending.citation == citation)
                                {
                                    return;
                                }
                                if !app.load_citation_source(&citation, window, cx) {
                                    return;
                                }
                                if let Err(error) =
                                    app.send_thread_selection(app.selected_thread.clone())
                                {
                                    app.report_error(error, cx);
                                    return;
                                }
                                app.detail = Default::default();
                                cx.notify();
                            })),
                    )
                })
                .child(
                    Button::new("citation-navigation-cancel")
                        .debug_selector(|| "citation-navigation-cancel".into())
                        .ghost()
                        .xsmall()
                        .label(if unavailable { "Dismiss" } else { "Cancel" })
                        .on_click(cx.listener(move |app, _, _, cx| {
                            if app.generation != navigation_generation
                                || !app
                                    .conversation_presentation
                                    .citation_navigation
                                    .as_ref()
                                    .is_some_and(|pending| pending.citation == cancel_citation)
                            {
                                return;
                            }
                            app.conversation_presentation.citation_navigation = None;
                            cx.notify();
                        })),
                ),
        )
    }

    pub(crate) fn jump_to_citation(&mut self, citation: &Value, cx: &mut Context<Self>) {
        let environment = self
            .server_config
            .config(self.generation)
            .and_then(|config| config["environment"]["environmentId"].as_str());
        if citation["environmentId"].as_str() != environment
            || citation["threadId"].as_str() != self.selected_thread.as_deref()
        {
            self.report_error("Citation belongs to another environment or thread; open its source conversation first", cx);
            return;
        }
        let Some(message) = citation["messageId"].as_str() else {
            self.report_error("Citation source is unavailable", cx);
            return;
        };
        if !self
            .detail
            .thread
            .as_ref()
            .and_then(|thread| thread["messages"].as_array())
            .is_some_and(|messages| {
                messages
                    .iter()
                    .any(|item| item["id"] == message && item["role"] == "assistant")
            })
        {
            self.report_error("Citation source message is no longer available", cx);
            return;
        }
        self.conversation_presentation.citation_target = Some(citation.clone());
        self.scroll_to_message(message);
        if let Some(state) = self
            .conversation_presentation
            .sources
            .get(message)
            .map(|source| source.state.clone())
        {
            self.highlight_citation_source(message, &state, cx);
        }
        cx.notify();
    }

    fn highlight_citation_source(
        &mut self,
        message: &str,
        state: &gpui_kit::Entity<TextViewState>,
        cx: &mut Context<Self>,
    ) {
        let Some(citation) = self
            .conversation_presentation
            .citation_target
            .as_ref()
            .filter(|citation| citation["messageId"] == message)
            .cloned()
        else {
            return;
        };
        let text = state.read(cx).rendered_text();
        if text.is_empty() {
            return;
        } // Initial parse has not landed yet.
        self.conversation_presentation.citation_target = None;
        if let Some(range) = resolve(text.as_str(), &citation) {
            let color = cx.theme().accent.opacity(0.35);
            for source in self.conversation_presentation.sources.values() {
                source
                    .state
                    .update(cx, |state, cx| state.clear_range_highlights(cx));
            }
            state.update(cx, |state, cx| {
                let _ = state.set_range_highlights([RangeHighlight::new(range.clone(), color)], cx);
                let _ = state.reveal_range(range, cx);
            });
        } else {
            self.report_error(
                "Citation text changed or is ambiguous; source message opened without a highlight",
                cx,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn r13_d2_citation_utf16_context_roundtrip_and_ambiguity() {
        let text = "😀 before\n\nhello **world** after";
        let start = text.find("hello").unwrap();
        let citation = selector(text, start..start + 5).unwrap();
        assert_eq!(citation["start"], 10);
        assert_eq!(citation["text"], "hello");
        assert_eq!(resolve(text, &citation), Some(start..start + 5));
        assert!(
            resolve(
                "same same",
                &serde_json::json!({"text":"same","prefix":"","suffix":""})
            )
            .is_none()
        );
        assert!(
            resolve(
                "aaa",
                &serde_json::json!({"text":"aa","prefix":"","suffix":""})
            )
            .is_none()
        );
        assert!(selector("😀", 1..4).is_err());
        assert!(selector(&"😀".repeat(4001), 0..16004).is_err());
    }
}
