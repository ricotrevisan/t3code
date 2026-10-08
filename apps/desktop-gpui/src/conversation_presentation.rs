//! View-local timeline preparation identity, verified against projection contents.
use std::collections::HashMap;

pub(crate) struct MessageJump {
    pub(crate) generation: u64,
    pub(crate) environment: String,
    pub(crate) thread: Option<String>,
    pub(crate) message: String,
}

pub(crate) struct PlanFollowup {
    pub(crate) generation: u64,
    pub(crate) environment: String,
    pub(crate) thread: String,
    pub(crate) plan: String,
    pub(crate) prompt: String,
    pub(crate) mode: String,
    pub(crate) accepted: Option<bool>,
}

#[derive(Default)]
pub(crate) struct ConversationPresentation {
    prepared: Option<(u64, u64, String, HashMap<String, bool>)>,
    // Share the projection, never clone it. Arc identity makes unchanged renders O(1)
    // and copy-on-write mutation invalidates even when a caller bypasses revisions.
    prepared_detail: Option<std::sync::Arc<serde_json::Value>>,
    pub(crate) preparations: usize,
    pub(crate) prepared_turns: usize,
    pub(crate) copied: Option<String>,
    pub(crate) code_wrap: HashMap<String, bool>,
    pub(crate) copied_epoch: u64,
    pub(crate) jump: Option<MessageJump>,
    pub(crate) saving: bool,
    pub(crate) plan_followup: Option<PlanFollowup>,
    pub(crate) reading_history: bool,
    pub(crate) media: HashMap<String, crate::views::message_media::MediaPreview>,
    pub(crate) retired_images: Vec<std::sync::Arc<gpui_kit::Image>>,
    pub(crate) media_epoch: u64,
    pub(crate) released_images: usize,
    pub(crate) gallery: Option<(String, serde_json::Value)>,
    pub(crate) gallery_items: Vec<(String, serde_json::Value, String)>,
    pub(crate) gallery_focus: Option<gpui_kit::FocusHandle>,
    pub(crate) zoom: f32,
}

impl ConversationPresentation {
    pub(crate) fn can_prepare_tail(
        &self,
        generation: u64,
        revision: u64,
        thread: &str,
        disclosures: &HashMap<String, bool>,
    ) -> bool {
        self.prepared.as_ref().is_some_and(|(g, r, t, d)| {
            *g == generation && *r == revision && t == thread && d == disclosures
        })
    }

    /// Validate the hint against every input outside its declared tail turn.
    /// Unknown fields, global request footers and ambiguous ordering fall back
    /// to full preparation rather than trust stream revision bookkeeping.
    pub(crate) fn tail_matches(&self, detail: &serde_json::Value, turn: &str) -> bool {
        // A folded tail hides its rows, but still owns global latest-activity
        // and checkpoint flags. The rendered tail cannot prove those flags.
        if self
            .prepared
            .as_ref()
            .is_some_and(|(_, _, _, disclosures)| {
                disclosures.get(&format!("fold:{turn}")) == Some(&true)
            })
        {
            return false;
        }
        let Some(old) = self.prepared_detail.as_ref().and_then(|value| value.as_object()) else {
            return false;
        };
        let Some(new) = detail.as_object() else {
            return false;
        };
        if old
            .get("latestTurn")
            .is_none_or(|latest| latest["turnId"] != turn)
            || detail["latestTurn"]["turnId"] != turn
            || new
                .keys()
                .any(|key| key != "updatedAt" && !old.contains_key(key))
        {
            return false;
        }
        let mut prefix_end = None;
        let mut tail_start = None;
        for (key, previous) in old {
            // latestTurn summarizes only this same tail; updatedAt is projection
            // bookkeeping and is not read by timeline_rows/group_turns.
            if matches!(key.as_str(), "latestTurn" | "updatedAt") {
                continue;
            }
            let Some(current) = new.get(key) else {
                return false;
            };
            if !matches!(
                key.as_str(),
                "messages" | "activities" | "proposedPlans" | "checkpoints"
            ) {
                if previous != current {
                    return false;
                }
                continue;
            }
            let (Some(previous), Some(current)) = (previous.as_array(), current.as_array()) else {
                return false;
            };
            if !previous
                .iter()
                .filter(|item| item["turnId"] != turn)
                .eq(current.iter().filter(|item| item["turnId"] != turn))
            {
                return false;
            }
            // Only append/upsert shapes whose ownership and ordering are
            // stable can reuse the prefix. Untracked deletion or mutation of
            // any existing non-message record must take the full path.
            let mut tail_items = HashMap::new();
            for item in current.iter().filter(|item| item["turnId"] == turn) {
                tail_items
                    .entry(item["id"].as_str())
                    .and_modify(|(_, count)| *count += 1)
                    .or_insert((item, 1));
            }
            if tail_items.values().any(|(_, count)| *count != 1) {
                return false;
            }
            for item in previous.iter().filter(|item| item["turnId"] == turn) {
                let Some(&(next, 1)) = tail_items.get(&item["id"].as_str()) else {
                    return false;
                };
                if item == next {
                    continue;
                }
                let (Some(item), Some(next)) = (item.as_object(), next.as_object()) else {
                    return false;
                };
                let mutable = |field: &str| match key.as_str() {
                    "messages" => matches!(
                        field,
                        "text" | "streaming" | "updatedAt" | "attachments" | "context"
                    ),
                    "checkpoints" => field == "assistantMessageId",
                    _ => false,
                };
                if item
                    .iter()
                    .any(|(field, value)| !mutable(field) && next.get(field) != Some(value))
                    || next
                        .keys()
                        .any(|field| !mutable(field) && !item.contains_key(field))
                {
                    return false;
                }
            }
            // Pending requests fold the whole activity log, not just one turn.
            if key == "activities"
                && previous
                    .iter()
                    .chain(current)
                    .any(|item| item["payload"].get("requestId").is_some())
            {
                return false;
            }
            let time_key = if key == "checkpoints" {
                "completedAt"
            } else {
                "createdAt"
            };
            for item in current {
                let Some(time) = item[time_key]
                    .as_str()
                    .and_then(|time| chrono::DateTime::parse_from_rfc3339(time).ok())
                else {
                    return false;
                };
                if item["turnId"] == turn {
                    tail_start = Some(
                        tail_start.map_or(time, |start: chrono::DateTime<chrono::FixedOffset>| {
                            start.min(time)
                        }),
                    );
                } else {
                    prefix_end = Some(
                        prefix_end.map_or(time, |end: chrono::DateTime<chrono::FixedOffset>| {
                            end.max(time)
                        }),
                    );
                }
            }
        }
        tail_start.is_some_and(|start| prefix_end.is_none_or(|end| start > end))
    }

    pub(crate) fn same_projection(&self, detail: Option<&std::sync::Arc<serde_json::Value>>) -> bool {
        match (self.prepared_detail.as_ref(), detail) {
            (Some(old), Some(new)) => std::sync::Arc::ptr_eq(old, new),
            (None, None) => true,
            _ => false,
        }
    }

    pub(crate) fn needs_preparation(
        &mut self,
        generation: u64,
        revision: u64,
        thread: &str,
        disclosures: &HashMap<String, bool>,
        detail: Option<&std::sync::Arc<serde_json::Value>>,
    ) -> bool {
        if self.same_projection(detail)
            && self.prepared.as_ref().is_some_and(|(g, r, t, d)| {
                *g == generation && *r == revision && t == thread && d == disclosures
            })
        {
            return false;
        }
        if self
            .prepared
            .as_ref()
            .is_none_or(|(g, _, t, _)| *g != generation || t != thread)
        {
            self.media_epoch = self.media_epoch.wrapping_add(1);
            self.retired_images.extend(self.media.drain().filter_map(
                |(_, preview)| match preview {
                    crate::views::message_media::MediaPreview::Ready(image) => Some(image),
                    _ => None,
                },
            ));
            self.copied = None;
            self.copied_epoch = self.copied_epoch.wrapping_add(1);
            self.gallery = None;
            self.zoom = 1.;
        }
        self.prepared = Some((generation, revision, thread.to_owned(), disclosures.clone()));
        self.prepared_detail = detail.cloned();
        self.preparations += 1;
        true
    }
}

/// Remove inline workspace image URLs from generic Markdown rendering. The
/// caller renders these resources through the same exact-file asset service.
/// External images are explicit links; data/reference previews are unavailable.
/// Only the bounded loader may hand image bytes to GPUI.
pub(crate) fn workspace_markdown_images(markdown: &str) -> (String, Vec<(String, String)>) {
    let mut rendered = String::new();
    let mut images = Vec::new();
    let mut fenced = false;
    for line in markdown.split_inclusive('\n') {
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            fenced = !fenced;
        }
        if fenced {
            rendered.push_str(line);
            continue;
        }
        let mut rest = line;
        while let Some(start) = rest.find("![") {
            let Some(close) = rest[start + 2..]
                .find("](")
                .map(|offset| start + 2 + offset)
            else {
                break;
            };
            let target_start = close + 2;
            let mut depth = 1;
            let end = rest[target_start..]
                .char_indices()
                .find_map(|(offset, ch)| {
                    if ch == '(' {
                        depth += 1;
                    } else if ch == ')' {
                        depth -= 1;
                    }
                    (depth == 0).then_some(target_start + offset)
                });
            let Some(end) = end else {
                break;
            };
            let alt = &rest[start + 2..close];
            let target = rest[target_start..end].trim();
            let target = if target.starts_with('<') {
                target
                    .split_once('>')
                    .map_or(target, |(path, _)| path.trim_start_matches('<'))
            } else {
                target.split(" \"").next().unwrap_or(target)
            };
            rendered.push_str(&rest[..start]);
            if url::Url::parse(target).is_ok() || target.starts_with("//") {
                // Never let generic GPUI Markdown decode unbounded external/data images.
                // HTTP resources remain explicitly openable links, not inline previews.
                if target.starts_with("http://") || target.starts_with("https://") {
                    rendered.push_str(&format!("[Image: {alt}]({target})"));
                } else {
                    rendered.push_str(&format!("[Image: {alt} · preview unavailable]"));
                }
            } else {
                rendered.push_str(&format!("[Image: {alt}]"));
                images.push((alt.to_owned(), target.to_owned()));
            }
            rest = &rest[end + 1..];
        }
        // Reference-style images have not been resolved through our bounded loader.
        // Render them as links rather than let the generic renderer decode originals.
        let rest = rest.replace("![", "[Image: ");
        let mut start = 0;
        for (index, character) in rest.char_indices() {
            if character == '<'
                && rest
                    .get(index + 1..index + 4)
                    .is_some_and(|tag| tag.eq_ignore_ascii_case("img"))
                && rest.get(index + 4..).is_some_and(|suffix| {
                    suffix
                        .starts_with(|ch: char| ch.is_ascii_whitespace() || ch == '>' || ch == '/')
                })
            {
                rendered.push_str(&rest[start..index]);
                rendered.push_str("&lt;");
                start = index + 1;
            }
        }
        rendered.push_str(&rest[start..]);
    }
    (rendered, images)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workspace_images_do_not_escape_into_unauthenticated_markdown() {
        let (text, images) = workspace_markdown_images(
            "![plot](charts/a(b).png) ![remote](https://example.com/a.png)\n```\n![code](literal.png)\n```\n",
        );
        assert_eq!(images, vec![("plot".into(), "charts/a(b).png".into())]);
        assert!(text.contains("[Image: plot]"));
        assert!(text.contains("[Image: remote](https://example.com/a.png)"));
        assert!(text.contains("![code](literal.png)"));
    }

    #[test]
    fn conversation_fixe_raw_html_and_reference_images_cannot_bypass_decode_limits() {
        let (text, images) = workspace_markdown_images(
            "<IMG src=\"https://example.com/bomb.gif\">\n![reference][huge]\n[huge]: https://example.com/bomb.gif\n```html\n<img src=\"literal\">\n```\n",
        );
        assert!(text.contains("&lt;IMG"));
        assert!(text.contains("[Image: reference][huge]"));
        assert!(
            text.contains("<img src=\"literal\">"),
            "code fences stay literal"
        );
        assert!(images.is_empty());
    }

    #[test]
    fn preparation_identity_ignores_unrelated_state_and_invalidates_replacements() {
        let mut cache = ConversationPresentation::default();
        let mut disclosures = HashMap::new();
        assert!(cache.needs_preparation(1, 7, "t", &disclosures, None));
        assert!(!cache.needs_preparation(1, 7, "t", &disclosures, None));
        assert_eq!(cache.preparations, 1);
        assert!(cache.needs_preparation(1, 8, "t", &disclosures, None));
        disclosures.insert("reasoning:r".into(), true);
        assert!(cache.needs_preparation(1, 8, "t", &disclosures, None));
        assert!(cache.needs_preparation(1, 8, "other", &disclosures, None));
        assert!(cache.needs_preparation(2, 8, "other", &disclosures, None));
    }
}
