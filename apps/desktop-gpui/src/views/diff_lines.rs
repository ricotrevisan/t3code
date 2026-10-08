//! Unified patch rows retain patch metadata and use independent old/new counters.
use crate::NativeApp;
use gpui_kit::component::ActiveTheme as _;
use gpui_kit::{Context, div, prelude::*};
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
impl NativeApp {
    pub(crate) fn render_patch_lines(
        &self,
        patch: &str,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let foreground = cx.theme().foreground;
        let muted = cx.theme().muted_foreground;
        let green = cx.theme().green;
        let danger = cx.theme().danger;
        div()
            .flex()
            .flex_col()
            .children(lines(patch).into_iter().map(|line| {
                div()
                    .flex()
                    .min_w_0()
                    .gap_2()
                    .child(
                        div()
                            .w_16()
                            .flex_shrink_0()
                            .text_color(muted)
                            .child(format!(
                                "{:>5} {:>5}",
                                line.old.map(|n| n.to_string()).unwrap_or_default(),
                                line.new.map(|n| n.to_string()).unwrap_or_default()
                            )),
                    )
                    .child(
                        div()
                            .min_w_0()
                            .when(self.diffs.wrap, |view| view.flex_1().whitespace_normal())
                            .when(!self.diffs.wrap, |view| view.whitespace_nowrap())
                            .text_color(match line.kind {
                                '+' => green,
                                '-' => danger,
                                '@' => muted,
                                _ => foreground,
                            })
                            .child(line.text.to_owned()),
                    )
            }))
    }
}
#[cfg(test)]
mod tests {
    use super::*;
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
