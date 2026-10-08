use crate::NativeApp;
use alacritty_terminal::term::cell::Flags;
use gpui_kit::{
    component::{
        ActiveTheme as _, Disableable as _, Selectable as _, Sizable as _,
        button::{Button, ButtonVariants},
        input::Input,
        menu::{ContextMenuExt as _, PopupMenuItem},
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
        let tab = self.terminal_active()?;
        let bounds = tab.bounds?;
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
                        if app
                            .terminal_drawer
                            .split
                            .as_ref()
                            .is_some_and(|(_, _, ids)| !ids.contains(&id))
                        {
                            app.terminal_drawer.split = None;
                        }
                        app.terminal_drawer.focus.focus(window, cx);
                        cx.notify();
                    }))
            })
            .collect::<Vec<_>>();
        let mut header = div()
            .px_2()
            .flex()
            .flex_wrap()
            .items_center()
            .gap_1()
            .min_w_0()
            .child(
                div()
                    .id("terminal-tabs")
                    .debug_selector(|| "terminal-tabs".into())
                    .flex()
                    .flex_1()
                    .min_w(px(120.))
                    .overflow_x_scroll()
                    .children(tab_buttons),
            )
            .child(
                Button::new("terminal-new")
                    .debug_selector(|| "terminal-new".into())
                    .ghost()
                    .small()
                    .icon(assets::IconName::Plus)
                    .tooltip("New terminal")
                    .disabled(!connected)
                    .on_click(cx.listener(|app, _, window, cx| {
                        app.new_terminal(cx);
                        app.terminal_drawer.focus.focus(window, cx);
                    })),
            )
            .child(
                Button::new("terminal-split")
                    .debug_selector(|| "terminal-split".into())
                    .ghost()
                    .small()
                    .icon(assets::IconName::PanelRight)
                    .tooltip("Split terminal side by side (max 4 panes)")
                    .disabled(
                        !connected
                            || tab_id.is_none()
                            || self.terminal_drawer.split.as_ref().is_some_and(
                                |(scope, thread, ids)| {
                                    scope == &self.environment_scope
                                        && Some(thread) == self.selected_thread.as_ref()
                                        && ids.len() >= 4
                                },
                            ),
                    )
                    .on_click(cx.listener(|app, _, window, cx| {
                        app.terminal_split(cx);
                        app.terminal_drawer.focus.focus(window, cx);
                    })),
            )
            .child(
                Button::new("terminal-unsplit")
                    .debug_selector(|| "terminal-unsplit".into())
                    .ghost()
                    .small()
                    .icon(assets::IconName::PanelLeftClose)
                    .tooltip("Show only the active terminal (keep sessions open)")
                    .disabled(self.terminal_drawer.split.is_none())
                    .on_click(cx.listener(|app, _, window, cx| {
                        app.terminal_drawer.split = None;
                        app.terminal_drawer.focus.focus(window, cx);
                        cx.notify();
                    })),
            )
            .child(
                Button::new("terminal-search")
                    .debug_selector(|| "terminal-search".into())
                    .ghost()
                    .small()
                    .icon(assets::IconName::Search)
                    .tooltip("Find in terminal")
                    .on_click(cx.listener(|app, _, window, cx| {
                        app.terminal_drawer.searching = !app.terminal_drawer.searching;
                        if app.terminal_drawer.searching {
                            app.terminal_drawer
                                .search
                                .update(cx, |input, cx| input.focus(window, cx));
                        } else {
                            app.terminal_drawer.focus.focus(window, cx);
                        }
                        cx.notify();
                    })),
            )
            .child(
                Button::new("terminal-discover")
                    .debug_selector(|| "terminal-discover".into())
                    .ghost()
                    .small()
                    .icon(assets::IconName::RefreshCw)
                    .tooltip("Restore sessions")
                    .disabled(!connected)
                    .on_click(cx.listener(|app, _, _, cx| app.terminal_discover(cx))),
            )
            .child(
                Button::new("terminal-hide")
                    .debug_selector(|| "terminal-hide".into())
                    .ghost()
                    .small()
                    .icon(assets::IconName::ChevronDown)
                    .tooltip("Hide terminal drawer")
                    .on_click(cx.listener(|app, _, window, cx| app.toggle_terminal(window, cx))),
            );
        if let Some(id) = &tab_id {
            let rename_id = id.clone();
            let close_id = id.clone();
            let restart_id = id.clone();
            let clear_id = id.clone();
            let ready = self.terminal_active().is_some_and(|tab| {
                tab.exists && tab.stream_ready && tab.attached == Some(self.generation)
            });
            header = header
                .child(
                    Button::new("terminal-restart")
                        .debug_selector(|| "terminal-restart".into())
                        .ghost()
                        .small().icon(assets::IconName::RefreshCw)
                        .tooltip(if self.terminal_active().is_some_and(|tab| tab.launch.is_null()) { "Restart unavailable: original launch environment/provider configuration is unknown" } else { "Restart terminal" })
                        .disabled(!connected || tab_busy || !ready || self.terminal_active().is_some_and(|tab| tab.launch.is_null()))
                        .on_click(cx.listener(move |app, _, _, cx| {
                            app.terminal_restart(&restart_id, false, cx)
                        })),
                )
                .child(
                    Button::new("terminal-clear")
                        .debug_selector(|| "terminal-clear".into())
                        .ghost()
                        .small().icon(assets::IconName::Eraser).tooltip("Clear terminal")
                        .disabled(!connected || tab_busy || !ready)
                        .on_click(cx.listener(move |app, _, _, cx| {
                            app.terminal_rpc(&clear_id, "terminal.clear", json!({}), cx)
                        })),
                )
                .child(
                    Button::new("terminal-rename")
                        .debug_selector(|| "terminal-rename".into())
                        .ghost()
                        .small().icon(assets::IconName::Pencil).tooltip("Rename terminal")
                        .on_click(cx.listener(move |app, _, window, cx| {
                            let title = app
                                .terminal_drawer
                                .tabs
                                .iter()
                                .find(|tab| {
                                    tab.id == rename_id
                                        && tab.scope == app.environment_scope
                                        && Some(&tab.thread) == app.selected_thread.as_ref()
                                })
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
                        .small().icon(assets::IconName::X).tooltip("Close terminal")
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
        if self.terminal_drawer.searching {
            drawer = drawer.child(
                div()
                    .flex()
                    .items_center()
                    .gap_1()
                    .px_2()
                    .child(Input::new(&self.terminal_drawer.search).w(px(180.)))
                    .child(
                        Button::new("terminal-find-previous")
                            .ghost()
                            .label("Previous")
                            .on_click(cx.listener(|app, _, _, cx| app.terminal_find(true, cx))),
                    )
                    .child(
                        Button::new("terminal-find-next")
                            .debug_selector(|| "terminal-find-next".into())
                            .ghost()
                            .label("Next")
                            .on_click(cx.listener(|app, _, _, cx| app.terminal_find(false, cx))),
                    )
                    .when(self.terminal_drawer.search_missing, |this| {
                        this.child("No matches")
                    }),
            );
        }
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
                                if !value.is_empty() {
                                    app.terminal_rename(&save_id, value, cx);
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
        if let Some(owner) = self.terminal_drawer.confirm.clone()
            && let Some(tab) = self.terminal_active().filter(|tab| {
                owner
                    == (
                        tab.scope.clone(),
                        tab.thread.clone(),
                        tab.id.clone(),
                        self.generation,
                        tab.epoch,
                    )
            })
        {
            let id = owner.2.clone();
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
        if let Some(owner) = self.terminal_drawer.restart_confirm.clone()
            && self.terminal_active().is_some_and(|tab| {
                owner
                    == (
                        tab.scope.clone(),
                        tab.thread.clone(),
                        tab.id.clone(),
                        self.generation,
                        tab.epoch,
                    )
            })
        {
            let id = owner.2.clone();
            drawer = drawer.child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .flex_wrap()
                    .child(format!(
                        "Restart {} in {} / thread {}? Its running process will be stopped.",
                        self.terminal_active().unwrap().title,
                        owner.0,
                        owner.1
                    ))
                    .child(
                        Button::new("terminal-confirm-restart")
                            .debug_selector(|| "terminal-confirm-restart".into())
                            .danger()
                            .label("Restart terminal")
                            .disabled(!connected || tab_busy)
                            .on_click(cx.listener(move |app, _, _, cx| {
                                app.terminal_restart(&id, true, cx)
                            })),
                    )
                    .child(
                        Button::new("terminal-cancel-restart")
                            .ghost()
                            .label("Cancel")
                            .on_click(cx.listener(|app, _, window, cx| {
                                app.terminal_drawer.restart_confirm = None;
                                app.terminal_drawer.focus.focus(window, cx);
                                cx.notify();
                            })),
                    ),
            );
        }
        if let Some(error) = &self.terminal_drawer.local_error {
            drawer = drawer.child(div().text_color(cx.theme().danger).child(error.clone()));
        }
        if !connected {
            drawer = drawer
                .child(div().child("Disconnected — input disabled; terminal state is unknown."));
        }
        let Some(id) = tab_id else {
            return Some(
                drawer
                    .child(if self.terminal_drawer.pending_open.is_some() {
                        "Discovering this thread's terminal sessions…"
                    } else {
                        "Select a saved thread and connect to open a terminal, or choose New."
                    })
                    .into_any_element(),
            );
        };
        let mut ids = self
            .terminal_drawer
            .split
            .as_ref()
            .filter(|(scope, thread, ids)| {
                scope == &self.environment_scope
                    && Some(thread) == self.selected_thread.as_ref()
                    && ids.contains(&id)
            })
            .map(|(_, _, ids)| ids.clone())
            .unwrap_or_else(|| vec![id.clone()]);
        ids.retain(|id| {
            self.terminal_drawer.tabs.iter().any(|tab| {
                &tab.id == id
                    && tab.scope == self.environment_scope
                    && Some(&tab.thread) == self.selected_thread.as_ref()
            })
        });
        let panes = ids
            .into_iter()
            .map(|id| self.render_terminal_pane(id, cx))
            .collect::<Vec<_>>();
        Some(
            drawer
                .child(self.terminal_input_container(panes, cx))
                .into_any_element(),
        )
    }

    fn terminal_input_container(
        &self,
        panes: Vec<AnyElement>,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let input_owner = self
            .terminal_active()
            .map(|tab| {
                (
                    tab.scope.clone(),
                    tab.thread.clone(),
                    tab.id.clone(),
                    self.generation,
                    tab.epoch,
                )
            })
            .unwrap();
        div()
            .id("terminal-panes")
            .flex()
            .flex_1()
            .min_h_0()
            .min_w_0()
            .key_context("NativeTerminal")
            .track_focus(&self.terminal_drawer.focus)
            .on_action(
                cx.listener(|app, _: &gpui_kit::component::input::Copy, _, cx| {
                    if let Some(capture) = app.terminal_capture() {
                        cx.write_to_clipboard(ClipboardItem::new_string(capture.text));
                    }
                    cx.stop_propagation();
                }),
            )
            .on_action(
                cx.listener(|app, _: &gpui_kit::component::input::Paste, window, cx| {
                    app.terminal_paste(window, cx);
                    cx.stop_propagation();
                }),
            )
            .on_action(cx.listener(|app, key: &super::keys::TerminalKey, _, cx| {
                if let Some(tab) = app.terminal_active() {
                    let id = tab.id.clone();
                    if let Some(data) = tab.screen.key(&key.key, None, key.control, false) {
                        app.terminal_rpc(&id, "terminal.write", json!({"data":data}), cx);
                    }
                }
            }))
            .capture_key_down(cx.listener(move |app, event: &KeyDownEvent, _, cx| {
                if !app.terminal_pane_current(&input_owner) {
                    return;
                }
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
                let Some(tab) = app.terminal_active() else {
                    return;
                };
                let input_id = tab.id.clone();
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
            .children(panes)
    }

    fn render_terminal_pane(&mut self, id: String, cx: &mut Context<Self>) -> AnyElement {
        let connected =
            self.shell.synchronized && self.connection_phase.ready() && self.commands.is_some();
        let active = self.terminal_active().is_some_and(|tab| tab.id == id);
        let mut drawer = div()
            .flex()
            .flex_col()
            .flex_1()
            .min_w_0()
            .min_h_0()
            .border_l_1()
            .border_color(cx.theme().border);
        let tab = self
            .terminal_drawer
            .tabs
            .iter()
            .find(|tab| {
                tab.id == id
                    && tab.scope == self.environment_scope
                    && Some(&tab.thread) == self.selected_thread.as_ref()
            })
            .unwrap();
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
        let owner = (
            tab.scope.clone(),
            tab.thread.clone(),
            tab.id.clone(),
            self.generation,
            tab.epoch,
        );
        let geometry_owner = owner.clone();
        let mouse_owner = owner.clone();
        let menu_owner = owner.clone();
        let scroll_owner = owner;
        let cells = tab.screen.cells();
        let composition = self
            .terminal_composition()
            .filter(|_| active)
            .map(|mark| mark.text.clone());
        let input_entity = cx.entity();
        let input_focus = self.terminal_drawer.focus.clone();
        let font_size = cx.theme().mono_font_size;
        let line_height = (font_size / px(1.) * 1.5).ceil();
        // Use the actual monospace advance, not a guessed character width.
        let font = font(crate::settings_fonts::family(
            3,
            cx.theme().mono_font_family.clone(),
            cx,
        ));
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
                let owner = geometry_owner.clone();
                cx.defer(move |cx| {
                    let _ = weak.update(cx, |app, cx| {
                        if !app.terminal_pane_current(&owner) {
                            return;
                        }
                        if let Some(tab) = app.terminal_drawer.tabs.iter_mut().find(|tab| {
                            tab.id == id
                                && tab.scope == app.environment_scope
                                && Some(&tab.thread) == app.selected_thread.as_ref()
                        }) {
                            tab.bounds = Some(bounds);
                        }
                        if app.terminal_active().is_some_and(|tab| tab.id == id) {
                            app.terminal_drawer.bounds = Some(bounds);
                        }
                        app.terminal_drawer.cell_width = width;
                        app.terminal_drawer.line_height = line_height;
                        app.terminal_resize(&id, size, cx);
                    });
                });
                (width, font.clone(), theme.clone())
            },
            move |bounds, (width, font, theme), window, cx| {
                if active {
                    window.handle_input(
                        &input_focus,
                        ElementInputHandler::new(bounds, input_entity.clone()),
                        cx,
                    );
                }
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
        let menu_app = cx.entity().downgrade();
        let selector = if active {
            "terminal-grid".to_owned()
        } else {
            format!("terminal-grid-{id}")
        };
        let scroll_id = id.clone();
        let menu_id = id.clone();
        let surface = div()
            .id(SharedString::from(format!("terminal-pane-{id}")))
            .debug_selector(move || selector.clone())
            .flex_1()
            .min_h_0()
            .overflow_hidden()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |app, event: &MouseDownEvent, window, cx| {
                    if !app.terminal_pane_current(&mouse_owner) {
                        return;
                    }
                    app.terminal_drawer.active = Some(draw_id.clone());
                    app.terminal_drawer.focus.focus(window, cx);
                    let width = app.terminal_drawer.cell_width;
                    if let Some((row, col)) = app.terminal_point(event.position, width, line_height)
                        && let Some(tab) = app.terminal_drawer.tabs.iter_mut().find(|tab| {
                            tab.id == draw_id
                                && tab.scope == app.environment_scope
                                && Some(&tab.thread) == app.selected_thread.as_ref()
                        })
                    {
                        if (event.modifiers.platform || event.modifiers.control)
                            && let Some(url) = tab.screen.link_at(row, col)
                        {
                            cx.open_url(&url);
                        } else {
                            tab.screen.select(row, col, true);
                        }
                        cx.notify();
                    }
                }),
            )
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |app, _, window, cx| {
                    if !app.terminal_pane_current(&menu_owner) {
                        return;
                    }
                    app.terminal_drawer.active = Some(menu_id.clone());
                    app.terminal_drawer.focus.focus(window, cx);
                    cx.notify();
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
                    && let Some(tab) = app.terminal_drawer.tabs.iter_mut().find(|tab| {
                        tab.id == id
                            && tab.scope == app.environment_scope
                            && Some(&tab.thread) == app.selected_thread.as_ref()
                    })
                {
                    tab.screen.select(row, col, false);
                    cx.notify();
                }
            }))
            .on_scroll_wheel(cx.listener(move |app, event: &ScrollWheelEvent, _, cx| {
                if !app.terminal_pane_current(&scroll_owner) {
                    return;
                }
                let id = &scroll_id;
                if let Some(tab) = app.terminal_drawer.tabs.iter_mut().find(|tab| {
                    &tab.id == id
                        && tab.scope == app.environment_scope
                        && Some(&tab.thread) == app.selected_thread.as_ref()
                }) {
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
        let surface = surface.context_menu(move |menu, _, cx| {
            let has_selection = menu_app
                .upgrade()
                .is_some_and(|app| app.read(cx).terminal_capture().is_some());
            let can_paste = menu_app.upgrade().is_some_and(|app| {
                let app = app.read(cx);
                app.connection_phase.ready()
                    && app.shell.synchronized
                    && app.terminal_active().is_some_and(|tab| {
                        tab.stream_ready && !tab.busy && tab.attached == Some(app.generation)
                    })
            });
            let owner = menu_app
                .upgrade()
                .and_then(|app| app.read(cx).terminal_menu_owner());
            let capture = menu_app
                .upgrade()
                .and_then(|app| app.read(cx).terminal_capture());
            let context_owner = owner.clone();
            let context_app = menu_app.clone();
            let copy_owner = owner.clone();
            let paste_owner = owner.clone();
            let link_owner = owner;
            let clipboard_text = menu_app.upgrade().and_then(|app| {
                app.read(cx)
                    .terminal_active()
                    .and_then(|tab| tab.screen.selection())
            });
            let copy_app = menu_app.clone();
            let paste_app = menu_app.clone();
            let link_app = menu_app.clone();
            let link = menu_app
                .upgrade()
                .and_then(|app| app.read(cx).terminal_capture())
                .and_then(|capture| url::Url::parse(capture.text.trim()).ok())
                .filter(|url| matches!(url.scheme(), "http" | "https") && url.host_str().is_some());
            let menu = if let Some(url) = link {
                menu.item(
                    PopupMenuItem::element(|_, _| {
                        div()
                            .id("terminal-menu-link")
                            .debug_selector(|| "terminal-menu-link".into())
                            .child("Open selected link")
                    })
                    .on_click(move |_, _, cx| {
                        if link_app
                            .upgrade()
                            .is_some_and(|app| app.read(cx).terminal_menu_current(&link_owner))
                        {
                            cx.open_url(url.as_str());
                        }
                    }),
                )
            } else {
                menu
            };
            menu.item(
                PopupMenuItem::element(|_, _| {
                    div()
                        .id("terminal-menu-chat")
                        .debug_selector(|| "terminal-menu-chat".into())
                        .child("Add to chat")
                })
                .disabled(capture.is_none())
                .on_click(move |_, window, cx| {
                    let _ = context_app.update(cx, |app, cx| {
                        if !app.terminal_menu_current(&context_owner) {
                            app.report_error(
                                "Terminal selection belongs to a different session",
                                cx,
                            );
                            return;
                        }
                        if let Some(capture) = &capture {
                            app.terminal_intake_capture(capture, window, cx);
                        }
                    });
                }),
            )
            .item(
                PopupMenuItem::element(|_, _| {
                    div()
                        .id("terminal-menu-copy")
                        .debug_selector(|| "terminal-menu-copy".into())
                        .child("Copy")
                })
                .disabled(!has_selection)
                .on_click(move |_, window, cx| {
                    let _ = copy_app.update(cx, |app, cx| {
                        if !app.terminal_menu_current(&copy_owner) {
                            return;
                        }
                        if let Some(text) = &clipboard_text {
                            cx.write_to_clipboard(ClipboardItem::new_string(text.clone()));
                        }
                        app.terminal_drawer.focus.focus(window, cx);
                    });
                }),
            )
            .item(
                PopupMenuItem::element(|_, _| {
                    div()
                        .id("terminal-menu-paste")
                        .debug_selector(|| "terminal-menu-paste".into())
                        .child("Paste")
                })
                .disabled(!can_paste)
                .on_click(move |_, window, cx| {
                    let _ = paste_app.update(cx, |app, cx| {
                        if !app.terminal_menu_current(&paste_owner) {
                            return;
                        }
                        app.terminal_paste(window, cx);
                        app.terminal_drawer.focus.focus(window, cx);
                    });
                }),
            )
        });
        drawer
            .child(div().flex().flex_1().min_h_0().px_2().pb_2().child(surface))
            .into_any_element()
    }
}
