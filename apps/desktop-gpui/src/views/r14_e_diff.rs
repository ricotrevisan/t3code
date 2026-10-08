//! Receipt-owned immutable line data. Only visible uniform-list rows become elements.
#[cfg(test)]
#[path = "r14_e_diff_tests.rs"]
mod tests;
use super::diff_lines::{lines, split_rows};
use gpui_kit::{
    ClipboardItem, Context, ListHorizontalSizingBehavior, Render, SharedString,
    UniformListScrollHandle, Window,
    component::ActiveTheme as _,
    component::{
        Sizable,
        button::{Button, ButtonVariants},
    },
    div,
    prelude::*,
    rems, uniform_list,
};
use std::{cell::Cell, rc::Rc};

fn language(path: &str) -> &'static str {
    match path.rsplit('.').next().unwrap_or_default() {
        "rs" => "rust",
        "ts" => "typescript",
        "tsx" => "tsx",
        "js" | "jsx" | "mjs" | "cjs" => "javascript",
        "json" => "json",
        "py" => "python",
        "go" => "go",
        "rb" => "ruby",
        "c" | "h" => "c",
        "cpp" | "hpp" | "cc" => "cpp",
        "css" => "css",
        "html" => "html",
        "md" => "markdown",
        "sh" | "bash" => "bash",
        "toml" => "toml",
        "yaml" | "yml" => "yaml",
        _ => "text",
    }
}

#[derive(Clone)]
struct Line {
    text: SharedString,
    old: Option<u64>,
    new: Option<u64>,
    kind: char,
    // Buffer side, byte offset and patch-prefix width; wrapping retains this mapping.
    syntax: Option<(usize, usize, usize)>,
    old_syntax: Option<(usize, usize, usize)>,
}
#[derive(Clone, PartialEq)]
pub(crate) struct Options {
    pub split: bool,
    pub columns: Option<usize>,
}
pub(crate) struct PatchView {
    lines: Vec<Line>,
    syntax: Rc<Vec<gpui_kit::component::highlighter::SyntaxHighlighter>>,
    pairs: Vec<(Option<usize>, Option<usize>)>,
    displayed: Rc<Vec<(Option<Line>, Option<Line>)>>,
    options: Options,
    longest: usize,
    scroll: UniformListScrollHandle,
    pub(crate) constructions: Rc<Cell<usize>>,
}
fn fragments(line: &Line, columns: Option<usize>) -> Vec<Line> {
    let Some(columns) = columns else {
        return vec![line.clone()];
    };
    if line.text.is_empty() {
        return vec![line.clone()];
    }
    let mut chunks = Vec::new();
    let mut chunk = String::new();
    let mut cells = 0;
    for character in line.text.chars() {
        // Conservative cell budget for wide glyphs and tabs. Preserve original
        // characters and keep the verbatim source for clipboard/editor actions.
        let width = if character == '\t' {
            8
        } else if character.is_ascii() {
            1
        } else {
            2
        };
        if cells + width > columns.max(1) && !chunk.is_empty() {
            chunks.push(std::mem::take(&mut chunk));
            cells = 0;
        }
        chunk.push(character);
        cells += width;
    }
    chunks.push(chunk);
    let mut offset: usize = 0;
    chunks
        .into_iter()
        .enumerate()
        .map(|(index, text)| {
            let syntax = line.syntax.map(|(side, start, prefix)| {
                (
                    side,
                    start + offset.saturating_sub(prefix),
                    if index == 0 { prefix } else { 0 },
                )
            });
            offset += text.len();
            Line {
                text: text.into(),
                old: (index == 0).then_some(line.old).flatten(),
                new: (index == 0).then_some(line.new).flatten(),
                kind: line.kind,
                syntax,
                old_syntax: None,
            }
        })
        .collect()
}
impl PatchView {
    #[cfg(test)]
    pub(crate) fn new(patch: &str, options: Options) -> Self {
        Self::with_path(patch, options, "")
    }
    pub(crate) fn with_path(patch: &str, options: Options, path: &str) -> Self {
        let parsed = lines(patch);
        let indices = parsed
            .iter()
            .enumerate()
            .map(|(index, line)| (line as *const _, index))
            .collect::<std::collections::HashMap<_, _>>();
        let pairs = split_rows(&parsed)
            .into_iter()
            .map(|(a, b)| {
                let index = |line: &super::diff_lines::DiffLine<'_>| indices[&(line as *const _)];
                (a.map(index), b.map(index))
            })
            .collect();
        let mut lines: Vec<Line> = parsed
            .into_iter()
            .map(|line| Line {
                text: line.text.to_owned().into(),
                old: line.old,
                new: line.new,
                kind: line.kind,
                syntax: None,
                old_syntax: None,
            })
            .collect();
        let mut bodies = [String::new(), String::new()];
        for line in &mut lines {
            if line.old.is_none() && line.new.is_none() {
                continue;
            }
            let side = usize::from(line.kind != '-');
            let start = bodies[side].len();
            let prefix = usize::from(matches!(line.kind, '+' | '-' | ' '));
            line.syntax = Some((side, start, prefix));
            bodies[side].push_str(&line.text[prefix..]);
            bodies[side].push('\n');
            if line.kind == ' ' {
                line.old_syntax = Some((0, bodies[0].len(), prefix));
                bodies[0].push_str(&line.text[prefix..]);
                bodies[0].push('\n');
            }
        }
        let syntax = bodies
            .iter()
            .map(|body| {
                let mut parser =
                    gpui_kit::component::highlighter::SyntaxHighlighter::new(language(path));
                parser.update(None, &gpui_kit::component::Rope::from_str(body), None);
                parser
            })
            .collect();
        let mut view = Self {
            lines,
            syntax: Rc::new(syntax),
            pairs,
            displayed: Rc::new(Vec::new()),
            options,
            longest: 0,
            scroll: UniformListScrollHandle::new(),
            constructions: Rc::new(Cell::new(0)),
        };
        view.rebuild();
        view
    }
    pub(crate) fn configure(&mut self, options: Options, cx: &mut Context<Self>) {
        if self.options != options {
            self.options = options;
            self.rebuild();
            cx.notify();
        }
    }
    fn rebuild(&mut self) {
        let pairs = if self.options.split {
            self.pairs.clone()
        } else {
            (0..self.lines.len())
                .map(|index| (Some(index), None))
                .collect()
        };
        self.displayed = Rc::new(
            pairs
                .into_iter()
                .flat_map(|(old, new)| {
                    let old = old
                        .map(|i| {
                            let mut line = self.lines[i].clone();
                            if self.options.split
                                && let Some(before) = line.old_syntax
                            {
                                line.syntax = Some(before);
                            }
                            fragments(&line, self.options.columns)
                        })
                        .unwrap_or_default();
                    let new = new
                        .map(|i| fragments(&self.lines[i], self.options.columns))
                        .unwrap_or_default();
                    (0..old.len().max(new.len()))
                        .map(|i| (old.get(i).cloned(), new.get(i).cloned()))
                        .collect::<Vec<_>>()
                })
                .collect(),
        );
        self.longest = self
            .displayed
            .iter()
            .enumerate()
            .max_by_key(|(_, (a, b))| {
                a.as_ref().map_or(0, |line| line.text.len())
                    + b.as_ref().map_or(0, |line| line.text.len())
            })
            .map_or(0, |(index, _)| index);
    }
}
impl Render for PatchView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let rows = self.displayed.clone();
        let split = self.options.split;
        let count = self.constructions.clone();
        let syntax = self.syntax.clone();
        uniform_list("patch-lines", rows.len(), move |range, _, cx| {
            count.set(count.get() + range.len());
            range
                .map(|index| {
                    let (old, new) = &rows[index];
                    div().h(rems(1.5)).flex().min_w_0().children(
                        [old, new]
                            .into_iter()
                            .enumerate()
                            .filter(|(side, _)| split || *side == 0)
                            .map(|(side, line)| {
                                let number = line
                                    .as_ref()
                                    .map(|line| {
                                        if split {
                                            if side == 0 { line.old } else { line.new }
                                                .map(|n| n.to_string())
                                                .unwrap_or_default()
                                        } else {
                                            format!(
                                                "{:>5} {:>5}",
                                                line.old.map(|n| n.to_string()).unwrap_or_default(),
                                                line.new.map(|n| n.to_string()).unwrap_or_default()
                                            )
                                        }
                                    })
                                    .unwrap_or_default();
                                let color = match line.as_ref().map(|line| line.kind) {
                                    Some('+') => cx.theme().green,
                                    Some('-') => cx.theme().danger,
                                    Some('@') => cx.theme().muted_foreground,
                                    _ => cx.theme().foreground,
                                };
                                div()
                                    .flex()
                                    .flex_1()
                                    .min_w_0()
                                    .gap_1()
                                    .child(
                                        div()
                                            .w(if split { rems(2.5) } else { rems(5.) })
                                            .flex_shrink_0()
                                            .text_color(color)
                                            .child(number),
                                    )
                                    .child(
                                        div().whitespace_nowrap().text_color(color).child(
                                            line.as_ref()
                                                .map(|line| {
                                                    let mut text = gpui_kit::StyledText::new(
                                                        line.text.clone(),
                                                    );
                                                    if let Some((side, start, prefix)) = line.syntax
                                                    {
                                                        let end = start + line.text.len() - prefix;
                                                        text = text.with_highlights(
                                                            syntax[side]
                                                                .styles(
                                                                    &(start..end),
                                                                    &*cx.theme().highlight_theme,
                                                                )
                                                                .into_iter()
                                                                .map(|(range, style)| {
                                                                    (
                                                                        (range.start - start
                                                                            + prefix)
                                                                            ..(range.end - start
                                                                                + prefix),
                                                                        style,
                                                                    )
                                                                }),
                                                        );
                                                    }
                                                    text
                                                })
                                                .unwrap_or_else(|| gpui_kit::StyledText::new("")),
                                        ),
                                    )
                            }),
                    )
                })
                .collect::<Vec<_>>()
        })
        .with_width_from_item(Some(self.longest))
        .with_horizontal_sizing_behavior(if self.options.columns.is_some() {
            ListHorizontalSizingBehavior::FitList
        } else {
            ListHorizontalSizingBehavior::Unconstrained
        })
        .track_scroll(&self.scroll)
        .h(rems(18.))
        .w_full()
        .text_xs()
        .font_family(cx.theme().mono_font_family.clone())
    }
}
pub(crate) struct FullFileView {
    bodies: [SharedString; 2],
    lines: [Rc<Vec<SharedString>>; 2],
    offsets: [Rc<Vec<usize>>; 2],
    syntax: [Rc<gpui_kit::component::highlighter::SyntaxHighlighter>; 2],
    longest: [usize; 2],
    scroll: [UniformListScrollHandle; 2],
    pub(crate) constructions: Rc<Cell<usize>>,
}
impl FullFileView {
    pub(crate) fn new(old: SharedString, new: SharedString, path: &str) -> Self {
        let bodies = [old, new];
        let offsets = std::array::from_fn(|side| {
            Rc::new(
                bodies[side]
                    .split_inclusive('\n')
                    .scan(0, |start, line| {
                        let offset = *start;
                        *start += line.len();
                        Some(offset)
                    })
                    .collect(),
            )
        });
        let syntax = std::array::from_fn(|side| {
            let mut parser =
                gpui_kit::component::highlighter::SyntaxHighlighter::new(language(path));
            parser.update(
                None,
                &gpui_kit::component::Rope::from_str(&bodies[side]),
                None,
            );
            Rc::new(parser)
        });
        let lines = [
            Rc::new(
                bodies[0]
                    .lines()
                    .map(|line| SharedString::from(line.to_owned()))
                    .collect::<Vec<_>>(),
            ),
            Rc::new(
                bodies[1]
                    .lines()
                    .map(|line| SharedString::from(line.to_owned()))
                    .collect::<Vec<_>>(),
            ),
        ];
        let longest = std::array::from_fn(|i| {
            lines[i]
                .iter()
                .enumerate()
                .max_by_key(|(_, line)| line.len())
                .map_or(0, |(index, _)| index)
        });
        Self {
            bodies,
            lines,
            offsets,
            syntax,
            longest,
            scroll: std::array::from_fn(|_| UniformListScrollHandle::new()),
            constructions: Rc::new(Cell::new(0)),
        }
    }
}
impl Render for FullFileView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div().flex().min_w_0().gap_2().children(
            ["Before (complete)", "After (complete)"]
                .into_iter()
                .enumerate()
                .map(|(index, label)| {
                    let lines = self.lines[index].clone();
                    let body = self.bodies[index].clone();
                    let offsets = self.offsets[index].clone();
                    let syntax = self.syntax[index].clone();
                    let count = self.constructions.clone();
                    div()
                        .flex()
                        .flex_col()
                        .flex_1()
                        .min_w_0()
                        .child(
                            Button::new(SharedString::from(format!("copy-full-{index}")))
                                .debug_selector(move || format!("copy-full-{index}"))
                                .ghost()
                                .small()
                                .label(format!("Copy {label}"))
                                .on_click(move |_, _, cx| {
                                    cx.write_to_clipboard(ClipboardItem::new_string(
                                        body.to_string(),
                                    ))
                                }),
                        )
                        .child(
                            uniform_list(
                                SharedString::from(format!("full-lines-{index}")),
                                lines.len(),
                                move |range, _, cx| {
                                    count.set(count.get() + range.len());
                                    range
                                        .map(|i| {
                                            div().h(rems(1.5)).whitespace_nowrap().child(
                                                gpui_kit::StyledText::new(lines[i].clone())
                                                    .with_highlights(
                                                        syntax
                                                            .styles(
                                                                &(offsets[i]
                                                                    ..offsets[i] + lines[i].len()),
                                                                &*cx.theme().highlight_theme,
                                                            )
                                                            .into_iter()
                                                            .map(|(range, style)| {
                                                                (
                                                                    (range.start - offsets[i])
                                                                        ..(range.end - offsets[i]),
                                                                    style,
                                                                )
                                                            }),
                                                    ),
                                            )
                                        })
                                        .collect::<Vec<_>>()
                                },
                            )
                            .with_width_from_item(Some(self.longest[index]))
                            .with_horizontal_sizing_behavior(
                                ListHorizontalSizingBehavior::Unconstrained,
                            )
                            .track_scroll(&self.scroll[index])
                            .h(rems(12.))
                            .text_xs()
                            .font_family(cx.theme().mono_font_family.clone()),
                        )
                }),
        )
    }
}
