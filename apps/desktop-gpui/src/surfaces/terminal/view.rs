use crate::NativeApp;
use alacritty_terminal::term::cell::Flags;
use gpui_kit::{
    component::{
        ActiveTheme as _, Disableable as _, Selectable as _,
        button::{Button, ButtonVariants},
        input::Input,
    },
    prelude::*,
    *,
};
use serde_json::json;
use t3code_desktop_gpui::terminal::Size;
#[path = "palette.rs"]
mod palette;

impl NativeApp {
    pub(super) fn terminal_active(&self) -> Option<&super::Tab> {
        let current = |tab: &&super::Tab| {
            tab.scope == self.environment_scope
                && Some(&tab.thread) == self.selected_thread.as_ref()
        };
        self.terminal_drawer
            .tabs
            .iter()
            .filter(current)
            .find(|tab| Some(&tab.id) == self.terminal_drawer.active.as_ref())
            .or_else(|| self.terminal_drawer.tabs.iter().find(current))
    }
    fn terminal_point(
        &self,
        position: Point<Pixels>,
        cell_width: f32,
        line_height: f32,
    ) -> Option<(usize, usize)> {
        let bounds = self.terminal_drawer.bounds?;
        let tab = self.terminal_active()?;
        let col = (((position.x - bounds.origin.x) / px(cell_width)).max(0.) as usize)
            .min(tab.screen.size.cols - 1);
        let row = (((position.y - bounds.origin.y) / px(line_height)).max(0.) as usize)
            .min(tab.screen.size.rows - 1);
        Some((row, col))
    }
    pub(crate) fn render_terminal(&mut self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if !self.terminal_drawer.open {
            return None;
        }
        let theme = cx.theme();
        let foreground = theme.foreground;
        let border = theme.border;
        let background = theme.background;
        let tab_id = self.terminal_active().map(|tab| tab.id.clone());
        let tab_busy = self.terminal_active().is_some_and(|tab| tab.busy);
        let connected =
            self.shell.synchronized && self.connection_phase.ready() && self.commands.is_some();
        let tab_buttons = self
            .terminal_drawer
            .tabs
            .iter()
            .filter(|tab| {
                tab.scope == self.environment_scope
                    && Some(&tab.thread) == self.selected_thread.as_ref()
            })
            .map(|tab| {
                let id = tab.id.clone();
                Button::new(SharedString::from(format!("terminal-tab-{id}")))
                    .ghost()
                    .flex_none()
                    .label(tab.title.clone())
                    .selected(tab_id.as_ref() == Some(&id))
                    .on_click(cx.listener(move |app, _, window, cx| {
                        app.terminal_drawer.active = Some(id.clone());
                        app.terminal_drawer.focus.focus(window, cx);
                        cx.notify();
                    }))
            })
            .collect::<Vec<_>>();
        let mut header = div()
            .px_2()
            .flex()
            .items_center()
            .gap_1()
            .min_w_0()
            .child(
                div()
                    .id("terminal-tabs")
                    .debug_selector(|| "terminal-tabs".into())
                    .flex()
                    .flex_1()
                    .min_w_0()
                    .overflow_x_scroll()
                    .children(tab_buttons),
            )
            .child(
                Button::new("terminal-new")
                    .debug_selector(|| "terminal-new".into())
                    .ghost()
                    .label("New")
                    .disabled(!connected)
                    .on_click(cx.listener(|app, _, window, cx| {
                        app.new_terminal(cx);
                        app.terminal_drawer.focus.focus(window, cx);
                    })),
            )
            .child(
                Button::new("terminal-hide")
                    .debug_selector(|| "terminal-hide".into())
                    .ghost()
                    .label("Hide")
                    .on_click(cx.listener(|app, _, window, cx| app.toggle_terminal(window, cx))),
            );
        if let Some(id) = &tab_id {
            let rename_id = id.clone();
            let close_id = id.clone();
            header = header
                .child(
                    Button::new("terminal-rename")
                        .debug_selector(|| "terminal-rename".into())
                        .ghost()
                        .label("Rename")
                        .on_click(cx.listener(move |app, _, window, cx| {
                            let title = app
                                .terminal_drawer
                                .tabs
                                .iter()
                                .find(|tab| tab.id == rename_id)
                                .unwrap()
                                .title
                                .clone();
                            app.terminal_drawer.renaming = Some(rename_id.clone());
                            app.terminal_drawer.rename.update(cx, |input, cx| {
                                input.set_value(title, window, cx);
                                input.focus(window, cx);
                            });
                            cx.notify();
                        })),
                )
                .child(
                    Button::new("terminal-close")
                        .debug_selector(|| "terminal-close".into())
                        .ghost()
                        .label("Close")
                        .disabled(!connected || tab_busy)
                        .on_click(cx.listener(move |app, _, _, cx| {
                            app.terminal_close(&close_id, false, cx)
                        })),
                );
        }
        let mut drawer = div()
            .id("terminal-drawer")
            .debug_selector(|| "terminal-drawer".into())
            .flex()
            .flex_col()
            .flex_none()
            .h(px(self.terminal_drawer.height))
            .min_w_0()
            .overflow_hidden()
            .bg(background)
            .text_color(foreground)
            .border_t_1()
            .border_color(border)
            .child(
                div()
                    .id("terminal-divider")
                    .debug_selector(|| "terminal-divider".into())
                    .h(px(6.))
                    .cursor_row_resize()
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|app, event: &MouseDownEvent, _, _| {
                            app.terminal_drawer.drag =
                                Some((event.position.y / px(1.), app.terminal_drawer.height));
                        }),
                    ),
            )
            .child(header);
        if let Some(id) = self.terminal_drawer.renaming.clone() {
            let save_id = id.clone();
            drawer = drawer.child(
                div()
                    .flex()
                    .gap_1()
                    .child(Input::new(&self.terminal_drawer.rename).w(px(180.)))
                    .child(
                        Button::new("terminal-rename-save")
                            .debug_selector(|| "terminal-rename-save".into())
                            .ghost()
                            .label("Save name")
                            .on_click(cx.listener(move |app, _, window, cx| {
                                let value = app
                                    .terminal_drawer
                                    .rename
                                    .read(cx)
                                    .value()
                                    .trim()
                                    .chars()
                                    .take(128)
                                    .collect::<String>();
                                if !value.is_empty()
                                    && let Some(tab) = app
                                        .terminal_drawer
                                        .tabs
                                        .iter_mut()
                                        .find(|tab| tab.id == save_id)
                                {
                                    tab.title = value;
                                }
                                app.terminal_drawer.renaming = None;
                                app.terminal_drawer.focus.focus(window, cx);
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("terminal-rename-cancel")
                            .debug_selector(|| "terminal-rename-cancel".into())
                            .ghost()
                            .label("Cancel")
                            .on_click(cx.listener(|app, _, window, cx| {
                                app.terminal_drawer.renaming = None;
                                app.terminal_drawer.focus.focus(window, cx);
                                cx.notify();
                            })),
                    ),
            );
        }
        if let Some(id) = self.terminal_drawer.confirm.clone()
            && let Some(tab) = self.terminal_drawer.tabs.iter().find(|tab| {
                tab.id == id
                    && tab.scope == self.environment_scope
                    && Some(&tab.thread) == self.selected_thread.as_ref()
            })
        {
            drawer = drawer.child(
                div()
                    .id("terminal-close-confirm")
                    .debug_selector(|| "terminal-close-confirm".into())
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap_2()
                    .child(format!(
                        "Close {} in {}? A process may still be running.",
                        tab.title, tab.cwd
                    ))
                    .child(
                        Button::new("terminal-confirm-close")
                            .debug_selector(|| "terminal-confirm-close".into())
                            .danger()
                            .label("Close terminal")
                            .disabled(!connected || tab_busy)
                            .on_click(
                                cx.listener(move |app, _, _, cx| app.terminal_close(&id, true, cx)),
                            ),
                    )
                    .child(
                        Button::new("terminal-cancel-close")
                            .debug_selector(|| "terminal-cancel-close".into())
                            .ghost()
                            .label("Cancel")
                            .on_click(cx.listener(|app, _, window, cx| {
                                app.terminal_drawer.confirm = None;
                                app.terminal_drawer.focus.focus(window, cx);
                                cx.notify();
                            })),
                    ),
            );
        }
        if !connected {
            drawer = drawer
                .child(div().child("Disconnected — input disabled; terminal state is unknown."));
        }
        let Some(id) = tab_id else {
            return Some(
                drawer
                    .child("Select a saved thread and connect to open a terminal.")
                    .into_any_element(),
            );
        };
        let tab = self.terminal_active().unwrap();
        drawer = drawer.child(
            div()
                .px_2()
                .text_xs()
                .child(format!("{} · {}", tab.status, tab.cwd)),
        );
        if connected && !tab.stream_ready {
            drawer = drawer
                .child(div().child("Waiting for output attachment snapshot — input disabled."));
        }
        if let Some(error) = &tab.error {
            drawer = drawer.child(div().text_color(cx.theme().danger).child(error.clone()));
        }
        let cells = tab.screen.cells();
        let composition = self.terminal_composition().map(|mark| mark.text.clone());
        let input_entity = cx.entity();
        let input_focus = self.terminal_drawer.focus.clone();
        let font_size = cx.theme().mono_font_size;
        let line_height = (font_size / px(1.) * 1.5).ceil();
        // Use the actual monospace advance, not a guessed character width.
        let font = font(cx.theme().mono_font_family.clone());
        let theme = cx.theme().clone();
        let draw_id = id.clone();
        let resize_id = id.clone();
        let weak = cx.entity().downgrade();
        let drag_app = weak.clone();
        // The canvas owns no timer. Cursor blinking is intentionally disabled.
        let canvas = canvas(
            move |bounds, window, cx| {
                let run = TextRun {
                    len: 1,
                    font: font.clone(),
                    color: theme.foreground,
                    background_color: None,
                    underline: None,
                    strikethrough: None,
                };
                let width = window
                    .text_system()
                    .shape_line("M".into(), font_size, &[run], None)
                    .width
                    / px(1.);
                let size = Size::new(
                    (bounds.size.width / px(width)) as usize,
                    (bounds.size.height / px(line_height)) as usize,
                );
                let weak = weak.clone();
                let id = resize_id.clone();
                cx.defer(move |cx| {
                    let _ = weak.update(cx, |app, cx| {
                        app.terminal_drawer.bounds = Some(bounds);
                        app.terminal_drawer.cell_width = width;
                        app.terminal_drawer.line_height = line_height;
                        app.terminal_resize(&id, size, cx);
                    });
                });
                (width, font.clone(), theme.clone())
            },
            move |bounds, (width, font, theme), window, cx| {
                window.handle_input(
                    &input_focus,
                    ElementInputHandler::new(bounds, input_entity.clone()),
                    cx,
                );
                let drag_end = drag_app.clone();
                window.on_mouse_event(move |_: &MouseUpEvent, phase, _, cx| {
                    if phase == DispatchPhase::Capture {
                        let _ = drag_end.update(cx, |app, _| {
                            if app.terminal_drawer.drag.take().is_some() {
                                app.remember_terminal_height();
                            }
                        });
                    }
                });
                let drag_app = drag_app.clone();
                window.on_mouse_event(move |event: &MouseMoveEvent, phase, window, cx| {
                    if phase != DispatchPhase::Capture {
                        return;
                    }
                    let _ = drag_app.update(cx, |app, cx| {
                        if let Some((start, height)) = app.terminal_drawer.drag {
                            if event.pressed_button == Some(MouseButton::Left) {
                                app.terminal_drawer.height =
                                    (height + start - event.position.y / px(1.)).clamp(
                                        180.,
                                        (window.viewport_size().height / px(1.) * 0.75).max(180.),
                                    );
                                cx.notify();
                            } else {
                                app.terminal_drawer.drag = None;
                            }
                        }
                    });
                });
                window.with_content_mask(Some(ContentMask { bounds }), |window| {
                    for cell in &cells {
                        if cell.flags.contains(Flags::WIDE_CHAR_SPACER) {
                            continue;
                        }
                        let origin = point(
                            bounds.origin.x + px(cell.col as f32 * width),
                            bounds.origin.y + px(cell.row as f32 * line_height),
                        );
                        let mut fg = palette::resolve(cell.fg, &theme);
                        let mut bg = palette::resolve(cell.bg, &theme);
                        if cell.flags.contains(Flags::INVERSE) {
                            std::mem::swap(&mut fg, &mut bg);
                        }
                        if cell.selected {
                            bg = theme.selection;
                        }
                        let cell_bounds = Bounds::new(
                            origin,
                            size(
                                px(width
                                    * if cell.flags.contains(Flags::WIDE_CHAR) {
                                        2.
                                    } else {
                                        1.
                                    }),
                                px(line_height),
                            ),
                        );
                        window.paint_quad(fill(cell_bounds, bg));
                        if cell.cursor {
                            window.paint_quad(fill(
                                Bounds::new(
                                    point(origin.x, origin.y + px(line_height - 2.)),
                                    size(px(width), px(2.)),
                                ),
                                theme.caret,
                            ));
                        }
                        if cell.text != " " && !cell.flags.contains(Flags::HIDDEN) {
                            let mut font = font.clone();
                            if cell.flags.contains(Flags::BOLD) {
                                font.weight = FontWeight::BOLD;
                            }
                            if cell.flags.contains(Flags::ITALIC) {
                                font.style = FontStyle::Italic;
                            }
                            let run = TextRun {
                                len: cell.text.len(),
                                font,
                                color: fg,
                                background_color: None,
                                underline: None,
                                strikethrough: None,
                            };
                            let line = window.text_system().shape_line(
                                cell.text.clone().into(),
                                font_size,
                                &[run],
                                None,
                            );
                            let _ = line.paint(
                                origin,
                                px(line_height),
                                TextAlign::Left,
                                None,
                                window,
                                cx,
                            );
                        }
                    }
                    if let Some(text) = &composition
                        && let Some(cell) = cells.iter().find(|cell| cell.cursor)
                    {
                        let origin = point(
                            bounds.origin.x + px(cell.col as f32 * width),
                            bounds.origin.y + px(cell.row as f32 * line_height),
                        );
                        let run = TextRun {
                            len: text.len(),
                            font: font.clone(),
                            color: theme.foreground,
                            background_color: None,
                            underline: Some(UnderlineStyle {
                                thickness: px(1.),
                                color: Some(theme.caret),
                                wavy: false,
                            }),
                            strikethrough: None,
                        };
                        let line = window.text_system().shape_line(
                            text.clone().into(),
                            font_size,
                            &[run],
                            None,
                        );
                        window.paint_quad(fill(
                            Bounds::new(origin, size(line.width, px(line_height))),
                            theme.background,
                        ));
                        let _ =
                            line.paint(origin, px(line_height), TextAlign::Left, None, window, cx);
                    }
                });
            },
        )
        .size_full();
        // Input captures only when the grid owns focus; cmd-j bubbles to the app.
        let input_id = id.clone();
        let surface = div()
            .id("terminal-grid")
            .debug_selector(|| "terminal-grid".into())
            .key_context("NativeTerminal")
            .track_focus(&self.terminal_drawer.focus)
            .flex_1()
            .min_h_0()
            .overflow_hidden()
            .on_action(cx.listener(|app, key: &super::keys::TerminalKey, _, cx| {
                if let Some(tab) = app.terminal_active() {
                    let id = tab.id.clone();
                    if let Some(data) = tab.screen.key(&key.key, None, key.control, false) {
                        app.terminal_rpc(&id, "terminal.write", json!({"data":data}), cx);
                    }
                }
            }))
            .capture_key_down(cx.listener(move |app, event: &KeyDownEvent, _, cx| {
                let key = &event.keystroke;
                // Copy/paste belong to the terminal. All other Cmd chords are app actions.
                if key.modifiers.platform && !matches!(key.key.as_str(), "c" | "v") {
                    return;
                }
                // Printable input, including Option/dead-key layouts, must pass through
                // the platform input handler exactly once (not both keyDown and insertText).
                if !key.modifiers.platform
                    && !key.modifiers.control
                    && (key.key_char.is_some() || app.terminal_composition().is_some())
                {
                    return;
                }
                let Some(tab) = app
                    .terminal_drawer
                    .tabs
                    .iter()
                    .find(|tab| tab.id == input_id)
                else {
                    return;
                };
                if key.modifiers.platform && key.key == "c" {
                    if let Some(text) = tab.screen.selection() {
                        cx.write_to_clipboard(ClipboardItem::new_string(text));
                    }
                } else if key.modifiers.platform && key.key == "v" {
                    if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
                        let data = tab.screen.paste(&text);
                        app.terminal_rpc(&input_id, "terminal.write", json!({"data":data}), cx);
                    }
                } else if !key.modifiers.platform
                    && let Some(data) = tab.screen.key(
                        &key.key,
                        key.key_char.as_deref(),
                        key.modifiers.control,
                        key.modifiers.alt,
                    )
                {
                    app.terminal_rpc(&input_id, "terminal.write", json!({"data":data}), cx);
                }
                cx.stop_propagation();
            }))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |app, event: &MouseDownEvent, window, cx| {
                    app.terminal_drawer.focus.focus(window, cx);
                    let width = app.terminal_drawer.cell_width;
                    if let Some((row, col)) = app.terminal_point(event.position, width, line_height)
                        && let Some(tab) = app
                            .terminal_drawer
                            .tabs
                            .iter_mut()
                            .find(|tab| tab.id == draw_id)
                    {
                        tab.screen.select(row, col, true);
                        cx.notify();
                    }
                }),
            )
            .on_mouse_move(cx.listener(|app, event: &MouseMoveEvent, _, cx| {
                if event.pressed_button != Some(MouseButton::Left) {
                    return;
                }
                let Some(id) = app.terminal_active().map(|tab| tab.id.clone()) else {
                    return;
                };
                let width = app.terminal_drawer.cell_width;
                if let Some((row, col)) =
                    app.terminal_point(event.position, width, app.terminal_drawer.line_height)
                    && let Some(tab) = app.terminal_drawer.tabs.iter_mut().find(|tab| tab.id == id)
                {
                    tab.screen.select(row, col, false);
                    cx.notify();
                }
            }))
            .on_scroll_wheel(cx.listener(|app, event: &ScrollWheelEvent, _, cx| {
                let Some(id) = app.terminal_active().map(|tab| tab.id.clone()) else {
                    return;
                };
                if let Some(tab) = app.terminal_drawer.tabs.iter_mut().find(|tab| tab.id == id) {
                    tab.screen.scroll(
                        (event
                            .delta
                            .pixel_delta(px(app.terminal_drawer.line_height))
                            .y
                            / px(app.terminal_drawer.line_height)) as i32,
                    );
                    cx.notify();
                }
                cx.stop_propagation();
            }))
            .child(canvas);
        Some(
            drawer
                .child(div().flex().flex_1().min_h_0().px_2().pb_2().child(surface))
                .into_any_element(),
        )
    }
}
