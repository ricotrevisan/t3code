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
        patch: &str,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let foreground = cx.theme().foreground;
        let muted = cx.theme().muted_foreground;
        let green = cx.theme().green;
        let danger = cx.theme().danger;
        if self.diffs.split {
            let rows = lines(patch);
            return div()
                .flex()
                .flex_col()
                .children(split_rows(&rows).into_iter().map(|(old, new)| {
                    div()
                        .flex()
                        .min_w_0()
                        .children(
                            [(old, true), (new, false)]
                                .into_iter()
                                .map(|(line, before)| {
                                    let number = line
                                        .and_then(|line| if before { line.old } else { line.new });
                                    let text =
                                        line.map(|line| line.text.to_owned()).unwrap_or_default();
                                    let color = match line.map(|line| line.kind) {
                                        Some('-') => danger,
                                        Some('+') => green,
                                        Some('@') => muted,
                                        _ => foreground,
                                    };
                                    div()
                                        .flex()
                                        .flex_1()
                                        .min_w_0()
                                        .gap_1()
                                        .border_r_1()
                                        .border_color(cx.theme().border)
                                        .child(div().w_8().flex_shrink_0().text_color(muted).child(
                                            number.map(|n| n.to_string()).unwrap_or_default(),
                                        ))
                                        .child(
                                            div()
                                                .flex_1()
                                                .min_w_0()
                                                .when(self.diffs.wrap, |view| {
                                                    view.whitespace_normal()
                                                })
                                                .when(!self.diffs.wrap, |view| {
                                                    view.whitespace_nowrap()
                                                })
                                                .text_color(color)
                                                .child(text),
                                        )
                                }),
                        )
                }))
                .into_any_element();
        }
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
            .into_any_element()
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
