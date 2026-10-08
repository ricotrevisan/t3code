use gpui_kit::base::Disableable;
use gpui_kit::base::Selectable;
use gpui_kit::component::{
    ActiveTheme as _, Sizable,
    button::{Button, ButtonVariants as _},
    input::{Input, Textarea},
    menu::{ContextMenuExt, PopupMenuItem},
};
use gpui_kit::{Context, div, prelude::*, rems};

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
                            .label("Case sensitive")
                            .small()
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
                            .label("Whole word")
                            .small()
                            .on_click(cx.listener(|app, _, _, cx| {
                                app.content_options.whole_word = !app.content_options.whole_word;
                                app.search_contents(cx);
                            })),
                    )
                    .child(
                        Button::new("content-regex")
                            .ghost()
                            .selected(self.content_options.use_regex)
                            .label("Regex")
                            .small()
                            .on_click(cx.listener(|app, _, _, cx| {
                                app.content_options.use_regex = !app.content_options.use_regex;
                                app.search_contents(cx);
                            })),
                    )
                    .child(
                        Button::new("search-contents")
                            .disabled(!self.connection_ready())
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
                            .max_h(rems(160. / 14.))
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
                                .label(format!("{}:{}  ", hit.path, hit.line_number))
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
                            .disabled(!self.connection_ready())
                            .debug_selector(|| "list-files".into())
                            .label("Browse directory")
                            .secondary()
                            .on_click(cx.listener(|app, _, _, cx| app.list_files(false, cx))),
                    )
                    .child(
                        Button::new("search-files")
                            .disabled(!self.connection_ready())
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
                    .max_h(rems(160. / 14.))
                    .overflow_y_scroll()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .children(self.file_entries.iter().enumerate().map(|(index, entry)| {
                        let path = entry.path.clone();
                        let directory = entry.kind == "directory";
                        let menu_path = path.clone();
                        let entity = cx.entity().downgrade();
                        let owner = self.checkout_context();
                        let generation = self.generation;
                        let can_add = self.selected_thread.is_some();
                        div()
                            .id(gpui_kit::SharedString::from(format!(
                                "file-entry-menu-{index}"
                            )))
                            .context_menu(move |menu, _, _| {
                                let open_path = menu_path.clone();
                                let add_path = menu_path.clone();
                                let copy_path = menu_path.clone();
                                let open_entity = entity.clone();
                                let add_entity = entity.clone();
                                let open_owner = owner.clone();
                                let add_owner = owner.clone();
                                menu.item(
                                    PopupMenuItem::new(if directory {
                                        "Open directory"
                                    } else {
                                        "Open file"
                                    })
                                    .on_click(
                                        move |_, window, cx| {
                                            let _ = open_entity.update(cx, |app, cx| {
                                                if app.generation != generation
                                                    || app.checkout_context() != open_owner
                                                {
                                                    return;
                                                }
                                                app.file_path.update(cx, |input, cx| {
                                                    input.set_value(open_path.clone(), window, cx)
                                                });
                                                if directory {
                                                    app.list_files(false, cx);
                                                } else {
                                                    app.file_operation(false, window, cx);
                                                }
                                            });
                                        },
                                    ),
                                )
                                .item(PopupMenuItem::new("Copy mention").on_click(
                                    move |_, _, cx| {
                                        cx.write_to_clipboard(gpui_kit::ClipboardItem::new_string(
                                            t3code_desktop_gpui::mentions::file_link(&copy_path),
                                        ))
                                    },
                                ))
                                .item(
                                    PopupMenuItem::new("Add to composer")
                                        .disabled(!can_add)
                                        .on_click(move |_, window, cx| {
                                            let _ = add_entity.update(cx, |app, cx| {
                                                if app.generation != generation
                                                    || app.checkout_context() != add_owner
                                                    || app.selected_thread.is_none()
                                                {
                                                    return;
                                                }
                                                let source =
                                                    t3code_desktop_gpui::mentions::file_link(
                                                        &add_path,
                                                    );
                                                app.composer.update(cx, |input, cx| {
                                                    input.replace(format!("{source} "), window, cx);
                                                    input.focus(window, cx);
                                                });
                                                app.remember_composer_draft(cx);
                                                cx.notify();
                                            });
                                        }),
                                )
                            })
                            .child(
                                Button::new(gpui_kit::SharedString::from(format!(
                                    "file-entry-{index}"
                                )))
                                .debug_selector(move || format!("file-entry-{index}"))
                                .ghost()
                                .label(format!(
                                    "{}{}",
                                    entry.path,
                                    if directory { "/" } else { "" }
                                ))
                                .on_click(cx.listener(
                                    move |app, _, window, cx| {
                                        app.file_path.update(cx, |input, cx| {
                                            input.set_value(path.clone(), window, cx)
                                        });
                                        if directory {
                                            app.list_files(false, cx);
                                        } else {
                                            app.file_operation(false, window, cx);
                                        }
                                    },
                                )),
                            )
                    })),
            )
            .child(
                div()
                    .flex()
                    .gap_2()
                    .child(
                        Button::new("read-file")
                            .disabled(!self.connection_ready())
                            .debug_selector(|| "read-file".into())
                            .label("Read file")
                            .secondary()
                            .on_click(cx.listener(|app, _, window, cx| {
                                app.file_operation(false, window, cx)
                            })),
                    )
                    .child(
                        Button::new("save-file")
                            .disabled(!self.connection_ready())
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
            .child(field(
                "Contents",
                Textarea::new(&self.file_contents)
                    .font_family(cx.theme().mono_font_family.clone())
                    .text_size(self.code_font_size(cx)),
                cx,
            ))
    }
}
