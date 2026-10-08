//! Unified patch rows retain patch metadata and use independent old/new counters.
use crate::NativeApp;
use gpui_kit::component::ActiveTheme as _;
use gpui_kit::{Context, prelude::*};
#[derive(Debug, PartialEq)]
pub(crate) struct DiffLine<'a> {
    pub text: &'a str,
    pub old: Option<u64>,
    pub new: Option<u64>,
    pub kind: char,
}
fn hunk_start(raw: &str) -> Option<u64> {
    raw.get(1..)?.split(',').next()?.parse().ok()
}
pub(crate) fn lines(patch: &str) -> Vec<DiffLine<'_>> {
    let mut old = 0;
    let mut new = 0;
    let mut in_hunk = false;
    patch
        .lines()
        .map(|text| {
            if text.starts_with("@@ ") {
                let mut fields = text.split_whitespace();
                fields.next();
                old = fields.next().and_then(hunk_start).unwrap_or(0);
                new = fields.next().and_then(hunk_start).unwrap_or(0);
                in_hunk = true;
                return DiffLine {
                    text,
                    old: None,
                    new: None,
                    kind: '@',
                };
            }
            let kind = text.chars().next().unwrap_or(' ');
            let (before, after) = if in_hunk {
                match kind {
                    '+' => {
                        let n = new;
                        new += 1;
                        (None, Some(n))
                    }
                    '-' => {
                        let n = old;
                        old += 1;
                        (Some(n), None)
                    }
                    ' ' => {
                        let o = old;
                        let n = new;
                        old += 1;
                        new += 1;
                        (Some(o), Some(n))
                    }
                    _ => (None, None),
                }
            } else {
                (None, None)
            };
            DiffLine {
                text,
                old: before,
                new: after,
                kind: if before.is_none() && after.is_none() {
                    '@'
                } else {
                    kind
                },
            }
        })
        .collect()
}
/// Pair adjacent removed/added runs; gaps never invent line numbers.
pub(crate) fn split_rows<'a>(
    rows: &'a [DiffLine<'a>],
) -> Vec<(Option<&'a DiffLine<'a>>, Option<&'a DiffLine<'a>>)> {
    let mut result = Vec::new();
    let mut index = 0;
    while index < rows.len() {
        if matches!(rows[index].kind, '-' | '+') {
            let start = index;
            while index < rows.len() && matches!(rows[index].kind, '-' | '+') {
                index += 1;
            }
            let old = rows[start..index]
                .iter()
                .filter(|line| line.kind == '-')
                .collect::<Vec<_>>();
            let new = rows[start..index]
                .iter()
                .filter(|line| line.kind == '+')
                .collect::<Vec<_>>();
            for offset in 0..old.len().max(new.len()) {
                result.push((old.get(offset).copied(), new.get(offset).copied()));
            }
        } else {
            result.push((Some(&rows[index]), Some(&rows[index])));
            index += 1;
        }
    }
    result
}
impl NativeApp {
    pub(crate) fn render_patch_lines(
        &self,
        file: &crate::app::diffs::PatchFile,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let split = self.diffs.split;
        // Reflow only on explicit wrap/layout changes, not composer or streaming updates.
        let columns = self.diffs.wrap.then(|| {
            let rem = crate::settings_appearance::current(cx).effective_size();
            let font = cx
                .text_system()
                .resolve_font(&gpui_kit::font(cx.theme().mono_font_family.clone()));
            let cell = cx
                .text_system()
                .advance(font, gpui_kit::px(rem * 0.75), 'W')
                .map_or(rem * 0.75, |size| f32::from(size.width));
            // The inspector width is a persisted physical split boundary; subtract
            // rem-based panel padding and the same gutters the row renderer uses.
            let available = (self.inspector_width - rem * 3.) / if split { 2. } else { 1. };
            ((available - rem * if split { 3. } else { 5.5 }) / cell).max(1.) as usize
        });
        let options = super::r14_e_diff::Options { split, columns };
        let entity = file
            .view
            .get_or_init(|| {
                cx.new(|_| {
                    super::r14_e_diff::PatchView::with_path(
                        &file.patch,
                        options.clone(),
                        &file.path,
                    )
                })
            })
            .clone();
        entity.update(cx, |view, cx| view.configure(options, cx));
        entity.cached(
            gpui_kit::StyleRefinement::default()
                .w_full()
                .h(gpui_kit::rems(18.)),
        )
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn r13_e_split_pairs_replacements_and_preserves_unmatched_lines() {
        let rows = lines("@@ -7,3 +9,2 @@\n-old1\n-old2\n+new\n same\n");
        let pairs = split_rows(&rows);
        assert_eq!(pairs[1].0.unwrap().old, Some(7));
        assert_eq!(pairs[1].1.unwrap().new, Some(9));
        assert_eq!(pairs[2].0.unwrap().old, Some(8));
        assert!(pairs[2].1.is_none());
        assert_eq!(pairs[3].0.unwrap().old, Some(9));
        assert_eq!(pairs[3].1.unwrap().new, Some(10));
    }
    #[test]
    fn unified_line_numbers_follow_hunks_not_metadata() {
        let rows =
            lines("--- a/x\n+++ b/x\n@@ -7,2 +9,2 @@\n-old\n+new\n same\n@@ -20 +30 @@\n+more\n");
        assert_eq!((rows[0].old, rows[1].new), (None, None));
        assert_eq!((rows[3].old, rows[3].new), (Some(7), None));
        assert_eq!((rows[4].old, rows[4].new), (None, Some(9)));
        assert_eq!((rows[5].old, rows[5].new), (Some(8), Some(10)));
        assert_eq!(rows[7].new, Some(30));
    }
}
