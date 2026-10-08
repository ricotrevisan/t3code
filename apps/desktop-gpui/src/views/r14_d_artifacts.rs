//! Validated Codex leaf result cards. Unknown/malformed syntax stays literal Markdown.
use super::*;
use std::collections::HashMap;

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ArtifactTemplate {
    pub(crate) kind: String,
    pub(crate) name: String,
    pub(crate) skill: String,
}

impl ArtifactTemplate {
    fn presentation(&self) -> &'static str {
        match self.kind.as_str() {
            "document" => "Document template",
            "presentation" => "Presentation template",
            "spreadsheet" => "Spreadsheet template",
            "site" => "Site template",
            "google-docs" => "Google Doc template",
            "google-slides" => "Google Slides template",
            "google-sheets" => "Google Sheet template",
            "image" => "Image template",
            "email" => "Email template",
            "slack" => "Slack template",
            _ => unreachable!(),
        }
    }
    pub(crate) fn prompt(&self) -> String {
        let skill = format!("${}", self.skill);
        match self.kind.as_str() {
            "document" => format!("Create a document using this {skill} about…"),
            "presentation" => format!("Create a presentation using the {skill} template about…"),
            "spreadsheet" => format!("Create a spreadsheet using this {skill} about…"),
            "site" => format!("Create a Site using this {skill} about…"),
            "google-docs" => format!("Create a Google Doc using this {skill} about…"),
            "google-slides" => {
                format!("Create a Google Slides presentation using this {skill} about…")
            }
            "google-sheets" => format!("Create a Google Sheet using this {skill} about…"),
            "image" => format!("Create an image using this {skill} of…"),
            "email" => format!("Draft an email using this {skill} about…"),
            "slack" => format!("Draft a Slack message using this {skill} about…"),
            _ => unreachable!(),
        }
    }
}

fn attributes(source: &str) -> Option<HashMap<String, String>> {
    let mut result = HashMap::new();
    let mut rest = source;
    while !rest.trim().is_empty() {
        rest = rest.trim_start();
        let end = rest.find('=')?;
        let key = &rest[..end];
        if key.is_empty() || !key.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_') {
            return None;
        }
        rest = &rest[end + 1..];
        let quote = rest.chars().next()?;
        let value;
        if quote == '"' || quote == '\'' {
            let mut text = String::new();
            let mut escaped = false;
            let mut closing = None;
            for (i, c) in rest[quote.len_utf8()..].char_indices() {
                if escaped {
                    text.push(c);
                    escaped = false;
                } else if c == '\\' {
                    escaped = true;
                } else if c == quote {
                    closing = Some(i + quote.len_utf8() * 2);
                    break;
                } else {
                    text.push(c);
                }
            }
            rest = &rest[closing?..];
            value = text;
        } else {
            let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
            value = rest[..end].to_owned();
            rest = &rest[end..];
        }
        if !rest.is_empty() && !rest.starts_with(char::is_whitespace) {
            return None;
        }
        if result.insert(key.to_owned(), value).is_some() {
            return None;
        }
    }
    Some(result)
}

fn parse(line: &str) -> Option<ArtifactTemplate> {
    let source = line
        .strip_prefix("::artifact-template{")?
        .strip_suffix('}')?;
    let attrs = attributes(source)?;
    let kind = attrs.get("artifact_kind")?;
    if ![
        "document",
        "presentation",
        "spreadsheet",
        "site",
        "google-docs",
        "google-slides",
        "google-sheets",
        "image",
        "email",
        "slack",
    ]
    .contains(&kind.as_str())
    {
        return None;
    }
    let name = attrs.get("display_name")?.trim();
    let skill = attrs.get("skill_name")?;
    let directory = attrs.get("skill_directory")?;
    let bytes = directory.as_bytes();
    let unc = directory
        .strip_prefix("\\\\")
        .map(|tail| tail.split('\\').collect::<Vec<_>>())
        .or_else(|| {
            directory
                .strip_prefix("//")
                .map(|tail| tail.split('/').collect())
        });
    let valid_unc =
        unc.is_some_and(|parts| parts.len() >= 2 && !parts[0].is_empty() && !parts[1].is_empty());
    let absolute = (directory.starts_with('/') && !directory.starts_with("//"))
        || (bytes.len() >= 3
            && bytes[0].is_ascii_alphabetic()
            && bytes[1] == b':'
            && matches!(bytes[2], b'/' | b'\\'))
        || valid_unc;
    if name.is_empty()
        || !skill.starts_with("artifact-template-")
        || !absolute
        || attrs
            .get("gallery_kind")
            .is_some_and(|g| !["imagegen", "product-design"].contains(&g.as_str()))
    {
        return None;
    }
    Some(ArtifactTemplate {
        kind: kind.clone(),
        name: name.to_owned(),
        skill: skill.clone(),
    })
}

/// Render card-only messages. Mixed prose/directives stays literal until the text
/// renderer supports ordered custom nodes; never reorder cards or duplicate labels.
pub(crate) fn extract(markdown: &str) -> (String, Vec<ArtifactTemplate>) {
    if !markdown.trim_start().starts_with("::artifact-template{") {
        return (markdown.to_owned(), Vec::new());
    }
    let mut body = String::new();
    let mut cards = Vec::new();
    let mut fence: Option<(char, usize)> = None;
    for line in markdown.split_inclusive('\n') {
        let trimmed = line.trim_end_matches(['\r', '\n']);
        let fence_line = trimmed.trim_start();
        let marker = fence_line.chars().next().filter(|c| matches!(c, '`' | '~'));
        let width = marker.map_or(0, |c| fence_line.chars().take_while(|m| *m == c).count());
        if width >= 3
            && let Some(marker) = marker
        {
            if let Some((open, size)) = fence {
                if open == marker && width >= size && fence_line[width..].trim().is_empty() {
                    fence = None;
                }
            } else {
                fence = Some((marker, width));
            }
            body.push_str(line);
            continue;
        }
        if fence.is_none()
            && cards.len() < 32
            && let Some(card) = parse(trimmed)
        {
            body.push_str(&format!("{} ({})\n\n", card.name, card.presentation()));
            cards.push(card);
        } else {
            body.push_str(line);
        }
    }
    if cards.is_empty()
        || body.lines().filter(|line| !line.trim().is_empty()).count() != cards.len()
    {
        return (markdown.to_owned(), Vec::new());
    }
    // Only the canonical labels we just substituted may remain in this message.
    let expected = cards
        .iter()
        .map(|card| format!("{} ({})\n\n", card.name, card.presentation()))
        .collect::<String>();
    if body.trim() != expected.trim() {
        return (markdown.to_owned(), Vec::new());
    }
    (String::new(), cards)
}

pub(crate) fn copy_text(markdown: &str) -> String {
    let (_, cards) = extract(markdown);
    if cards.is_empty() {
        markdown.to_owned()
    } else {
        cards
            .iter()
            .map(|card| format!("{} ({})\n\n", card.name, card.presentation()))
            .collect()
    }
}

impl NativeApp {
    pub(super) fn render_artifact_templates(
        &self,
        cards: Vec<ArtifactTemplate>,
        cx: &mut Context<Self>,
    ) -> gpui_kit::Div {
        let generation = self.generation;
        let scope = self.environment_scope.clone();
        let thread = self.selected_thread.clone();
        let busy = self.pending_actions > 0
            || self.attachment_drafts.uploading
            || self.attachment_drafts.stash_deleting
            || self.attachment_drafts.stash_writing;
        div()
            .flex()
            .flex_col()
            .gap_2()
            .children(cards.into_iter().enumerate().map(|(index, card)| {
                let scope = scope.clone();
                let thread = thread.clone();
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .p_2()
                    .rounded_md()
                    .border_1()
                    .border_color(cx.theme().border)
                    .bg(cx.theme().group_box)
                    .child(
                        Icon::new(match card.kind.as_str() {
                            "image" => IconName::Image,
                            "site" => IconName::Globe,
                            _ => IconName::File,
                        })
                        .small()
                        .text_color(cx.theme().muted_foreground),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(div().text_sm().child(card.name.clone()))
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(card.presentation()),
                            ),
                    )
                    .child(
                        Button::new(gpui_kit::SharedString::from(format!(
                            "artifact-template-use-{index}"
                        )))
                        .debug_selector(move || format!("artifact-template-use-{index}"))
                        .ghost()
                        .small()
                        .label("Use template")
                        .disabled(busy)
                        .on_click(cx.listener(
                            move |app, _, window, cx| {
                                if app.generation != generation
                                    || app.environment_scope != scope
                                    || app.selected_thread != thread
                                {
                                    return;
                                }
                                if app.pending_actions > 0
                                    || app.attachment_drafts.uploading
                                    || app.attachment_drafts.stash_deleting
                                    || app.attachment_drafts.stash_writing
                                {
                                    app.report_progress(
                                        "Wait for the composer operation before adding a template",
                                        cx,
                                    );
                                    return;
                                }
                                let mut draft = app.composer.read(cx).value().to_string();
                                let prompt = card.prompt();
                                let trimmed = draft.trim_end();
                                let start = trimmed.len().checked_sub(prompt.len());
                                let already = start.is_some_and(|i| {
                                    trimmed.get(i..) == Some(prompt.as_str())
                                        && (i == 0
                                            || trimmed[..i]
                                                .chars()
                                                .next_back()
                                                .is_some_and(char::is_whitespace))
                                });
                                if !already {
                                    if !draft.is_empty() && !draft.ends_with(char::is_whitespace) {
                                        draft.push(' ');
                                    }
                                    draft.push_str(&prompt);
                                    app.composer.update(cx, |input, cx| {
                                        input.set_value(draft, window, cx);
                                        input.focus(window, cx);
                                    });
                                    app.remember_composer_draft(cx);
                                }
                            },
                        )),
                    )
            }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const DIRECTIVE: &str = "::artifact-template{artifact_kind=\"document\" display_name=\"Report\" skill_directory=\"/skills/report\" skill_name=\"artifact-template-report\"}";
    #[test]
    fn r14_d_artifact_parser_preserves_code_and_malformed_directives() {
        let (body, cards) = extract(DIRECTIVE);
        assert_eq!(cards.len(), 1);
        assert!(body.is_empty());
        assert_eq!(copy_text(DIRECTIVE), "Report (Document template)\n\n");
        assert_eq!(
            cards[0].prompt(),
            "Create a document using this $artifact-template-report about…"
        );
        for input in [
            format!("Intro\n\n{DIRECTIVE}"),
            format!("```text\n{DIRECTIVE}\n```"),
            format!("    {DIRECTIVE}"),
            DIRECTIVE.replace("/skills/report", "relative"),
            DIRECTIVE.replace("document", "unknown"),
        ] {
            let (body, cards) = extract(&input);
            assert!(cards.is_empty());
            assert_eq!(body, input);
        }
    }
}
