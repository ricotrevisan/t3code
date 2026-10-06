use gpui_kit::base::Selectable;
use gpui_kit::component::{
    ActiveTheme as _,
    button::{Button, ButtonVariants as _},
    input::{Input, Textarea},
};
use gpui_kit::{Context, div, prelude::*, px};

use super::field;
use crate::NativeApp;

impl NativeApp {
    pub(super) fn render_files(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let selection = cx.theme().selection;
        div()
            .flex()
            .flex_col()
            .gap_2()
            .child(field(
                "Search contents",
                div()
                    .key_context("NativeContentSearch")
                    .child(Input::new(&self.content_query)),
                cx,
            ))
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap_2()
                    .child(
                        Button::new("content-case")
                            .ghost()
                            .selected(self.content_options.case_sensitive)
                            .label(if self.content_options.case_sensitive {
                                "Case sensitive ✓"
                            } else {
                                "Case sensitive"
                            })
                            .on_click(cx.listener(|app, _, _, cx| {
                                app.content_options.case_sensitive =
                                    !app.content_options.case_sensitive;
                                app.search_contents(cx);
                            })),
                    )
                    .child(
                        Button::new("content-word")
                            .ghost()
                            .selected(self.content_options.whole_word)
                            .label(if self.content_options.whole_word {
                                "Whole word ✓"
                            } else {
                                "Whole word"
                            })
                            .on_click(cx.listener(|app, _, _, cx| {
                                app.content_options.whole_word = !app.content_options.whole_word;
                                app.search_contents(cx);
                            })),
                    )
                    .child(
                        Button::new("content-regex")
                            .ghost()
                            .selected(self.content_options.use_regex)
                            .label(if self.content_options.use_regex {
                                "Regex ✓"
                            } else {
                                "Regex"
                            })
                            .on_click(cx.listener(|app, _, _, cx| {
                                app.content_options.use_regex = !app.content_options.use_regex;
                                app.search_contents(cx);
                            })),
                    )
                    .child(
                        Button::new("search-contents")
                            .debug_selector(|| "search-contents".into())
                            .label("Search contents")
                            .secondary()
                            .on_click(cx.listener(|app, _, _, cx| app.search_contents(cx))),
                    ),
            )
            .children(self.content_results.as_ref().map(|results| {
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .when(results.truncated, |view| {
                        view.child("Content results truncated; refine the search")
                    })
                    .when(results.matches.is_empty(), |view| {
                        view.child("No content matches")
                    })
                    .child(
                        div()
                            .id("content-result-list")
                            .track_scroll(&self.content_scroll)
                            .max_h(px(160.))
                            .overflow_y_scroll()
                            .flex()
                            .flex_col()
                            .children(results.matches.iter().enumerate().map(|(index, hit)| {
                                Button::new(gpui_kit::SharedString::from(format!(
                                    "content-hit-{index}"
                                )))
                                .debug_selector(move || format!("content-hit-{index}"))
                                .ghost()
                                .selected(index == self.content_index)
                                .label(format!(
                                    "{}{}:{}  {}",
                                    if index == self.content_index {
                                        "› "
                                    } else {
                                        ""
                                    },
                                    hit.path,
                                    hit.line_number,
                                    ""
                                ))
                                .child(
                                    gpui_kit::StyledText::new(hit.line_content.clone())
                                        .with_highlights(hit.byte_ranges().into_iter().map(
                                            |range| {
                                                (
                                                    range,
                                                    gpui_kit::HighlightStyle {
                                                        background_color: Some(selection),
                                                        ..Default::default()
                                                    },
                                                )
                                            },
                                        )),
                                )
                                .on_click(cx.listener(
                                    move |app, _, window, cx| {
                                        app.open_content_hit(index, window, cx)
                                    },
                                ))
                            })),
                    )
            }))
            .child(field("File path", Input::new(&self.file_path), cx))
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap_2()
                    .child(
                        Button::new("list-files")
                            .debug_selector(|| "list-files".into())
                            .label("Browse directory")
                            .secondary()
                            .on_click(cx.listener(|app, _, _, cx| app.list_files(false, cx))),
                    )
                    .child(
                        Button::new("search-files")
                            .debug_selector(|| "search-files".into())
                            .label("Search file paths")
                            .secondary()
                            .on_click(cx.listener(|app, _, _, cx| app.list_files(true, cx))),
                    ),
            )
            .when(self.file_listing_truncated, |view| {
                view.child("Results truncated; refine the path or search")
            })
            .child(
                div()
                    .id("file-entry-list")
                    .max_h(px(160.))
                    .overflow_y_scroll()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .children(self.file_entries.iter().enumerate().map(|(index, entry)| {
                        let path = entry.path.clone();
                        let directory = entry.kind == "directory";
                        Button::new(gpui_kit::SharedString::from(format!("file-entry-{index}")))
                            .debug_selector(move || format!("file-entry-{index}"))
                            .ghost()
                            .label(format!(
                                "{}{}",
                                entry.path,
                                if directory { "/" } else { "" }
                            ))
                            .on_click(cx.listener(move |app, _, window, cx| {
                                app.file_path.update(cx, |input, cx| {
                                    input.set_value(path.clone(), window, cx)
                                });
                                if directory {
                                    app.list_files(false, cx);
                                } else {
                                    app.file_operation(false, window, cx);
                                }
                            }))
                    })),
            )
            .child(
                div()
                    .flex()
                    .gap_2()
                    .child(
                        Button::new("read-file")
                            .debug_selector(|| "read-file".into())
                            .label("Read file")
                            .secondary()
                            .on_click(cx.listener(|app, _, window, cx| {
                                app.file_operation(false, window, cx)
                            })),
                    )
                    .child(
                        Button::new("save-file")
                            .debug_selector(|| "save-file".into())
                            .label("Save file")
                            .primary()
                            .on_click(cx.listener(|app, _, window, cx| {
                                app.file_operation(true, window, cx)
                            })),
                    ),
            )
            .children(self.file_editor.target.as_ref().map(|target| {
                format!(
                    "Loaded: {} / {}{}",
                    target.cwd,
                    target.path,
                    if self.file_editor.writable() {
                        ""
                    } else {
                        " (read-only)"
                    }
                )
            }))
            .child(
                Button::new("discard-file-edits")
                    .debug_selector(|| "discard-file-edits".into())
                    .label("Discard unsaved file edits")
                    .ghost()
                    .on_click(cx.listener(|app, _, window, cx| {
                        if app.file_busy {
                            app.status =
                                "Wait for the pending file operation before discarding".into();
                            cx.notify();
                            return;
                        }
                        let text = app.file_editor.saved.clone();
                        app.file_contents
                            .update(cx, |input, cx| input.set_value(text, window, cx));
                        cx.notify();
                    })),
            )
            .child(field("Contents", Textarea::new(&self.file_contents), cx))
    }
}
