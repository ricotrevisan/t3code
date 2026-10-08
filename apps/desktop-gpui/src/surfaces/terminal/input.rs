//! The PTY is a stream, not an editable document. Only IME preedit is local;
//! committed text goes through the same attachment/generation guards as keys.
use crate::NativeApp;
use gpui_kit::*;
use serde_json::json;
use std::ops::Range;

pub(super) struct Composition {
    pub id: String,
    pub scope: String,
    pub thread: String,
    pub epoch: u64,
    pub generation: u64,
    pub text: String,
    pub selection: Range<usize>,
}

// Platform ranges are UTF-16, while Rust strings must be sliced at UTF-8 boundaries.
fn byte_offset(text: &str, offset: usize) -> usize {
    let mut units = 0;
    for (byte, ch) in text.char_indices() {
        if units >= offset {
            return byte;
        }
        units += ch.len_utf16();
    }
    text.len()
}

impl NativeApp {
    pub(super) fn terminal_observe_focus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.terminal_drawer.focus_subscription.is_none() {
            let owner = cx.entity().downgrade();
            self.terminal_drawer.focus_subscription =
                Some(
                    window.on_focus_out(&self.terminal_drawer.focus, cx, move |_, _, cx| {
                        // Focus can change inside an app callback. Defer the entity
                        // update rather than trying to reborrow that live callback.
                        let owner = owner.clone();
                        cx.defer(move |cx| {
                            let _ = owner.update(cx, |app, cx| {
                                app.terminal_drawer.composition = None;
                                cx.notify();
                            });
                        });
                    }),
                );
        }
    }
    pub(crate) fn clamp_terminal_height(&mut self, window: &Window) {
        self.terminal_drawer.height = crate::window_state::clamp_width(
            self.terminal_drawer.height,
            crate::window_state::TERMINAL_HEIGHT,
        )
        .min((window.viewport_size().height / px(1.) * 0.75).max(180.));
    }

    pub(super) fn terminal_composition(&self) -> Option<&Composition> {
        self.terminal_drawer.composition.as_ref().filter(|mark| {
            mark.scope == self.environment_scope
                && mark.generation == self.generation
                && self.terminal_active().is_some_and(|tab| {
                    tab.id == mark.id && tab.thread == mark.thread && tab.epoch == mark.epoch
                })
        })
    }

    fn terminal_text_target(&self) -> Option<String> {
        (self.terminal_drawer.open
            && self.shell.synchronized
            && self.connection_phase.ready()
            && self.commands.is_some())
        .then(|| self.terminal_active())
        .flatten()
        .filter(|tab| {
            tab.stream_ready && tab.exists && !tab.busy && tab.attached == Some(self.generation)
        })
        .map(|tab| tab.id.clone())
    }

    pub(crate) fn remember_terminal_height(&mut self) {
        if let Some(state) = &mut self.saved_window {
            state.terminal_height = crate::window_state::clamp_width(
                self.terminal_drawer.height,
                crate::window_state::TERMINAL_HEIGHT,
            );
            crate::window_state::save(*state);
        }
    }
}

impl EntityInputHandler for NativeApp {
    fn paste(&mut self, item: ClipboardItem, window: &mut Window, cx: &mut Context<Self>) {
        if self.terminal_drawer.focus.is_focused(window)
            && let Some(text) = item.text()
            && let Some(id) = self.terminal_text_target()
            && let Some(tab) = self.terminal_active()
        {
            let data = tab.screen.paste(&text);
            self.terminal_rpc(&id, "terminal.write", json!({"data":data}), cx);
        }
    }

    fn text_for_range(
        &mut self,
        range: Range<usize>,
        adjusted: &mut Option<Range<usize>>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<String> {
        let text = self
            .terminal_composition()
            .map_or("", |mark| mark.text.as_str());
        let start = byte_offset(text, range.start);
        let end = byte_offset(text, range.end).max(start);
        *adjusted = Some(text[..start].encode_utf16().count()..text[..end].encode_utf16().count());
        Some(text[start..end].into())
    }

    fn selected_text_range(
        &mut self,
        ignore_disabled: bool,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        if !ignore_disabled && self.terminal_text_target().is_none() {
            return None;
        }
        Some(UTF16Selection {
            range: self
                .terminal_composition()
                .map_or(0..0, |mark| mark.selection.clone()),
            reversed: false,
        })
    }

    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        self.terminal_composition()
            .map(|mark| 0..mark.text.encode_utf16().count())
    }

    fn unmark_text(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let target = self
            .terminal_drawer
            .focus
            .is_focused(window)
            .then(|| self.terminal_text_target())
            .flatten();
        let current = self.terminal_composition().is_some();
        if let Some(mark) = self.terminal_drawer.composition.take()
            && current
            && target.as_ref() == Some(&mark.id)
            && !mark.text.is_empty()
        {
            self.terminal_rpc(&mark.id, "terminal.write", json!({"data":mark.text}), cx);
        }
        cx.notify();
    }

    fn replace_text_in_range(
        &mut self,
        _: Option<Range<usize>>,
        text: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // A committed insert replaces the local preedit, never edits shell history.
        let stale =
            self.terminal_drawer.composition.is_some() && self.terminal_composition().is_none();
        self.terminal_drawer.composition = None;
        if !stale
            && self.terminal_drawer.focus.is_focused(window)
            && !text.is_empty()
            && let Some(id) = self.terminal_text_target()
        {
            self.terminal_rpc(&id, "terminal.write", json!({"data":text}), cx);
        }
        cx.notify();
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
        selection: Option<Range<usize>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(id) = self
            .terminal_drawer
            .focus
            .is_focused(window)
            .then(|| self.terminal_text_target())
            .flatten()
        else {
            self.terminal_drawer.composition = None;
            return;
        };
        let old = self
            .terminal_composition()
            .map_or("", |mark| mark.text.as_str());
        let range = range.unwrap_or(0..old.encode_utf16().count());
        let start = byte_offset(old, range.start);
        let end = byte_offset(old, range.end).max(start);
        let prefix_len = old[..start].encode_utf16().count();
        let value = format!("{}{text}{}", &old[..start], &old[end..]);
        let length = value.encode_utf16().count();
        let selected = selection.map_or(length..length, |r| {
            (prefix_len + r.start).min(length)..(prefix_len + r.end).min(length)
        });
        let tab = self.terminal_active().unwrap();
        let thread = tab.thread.clone();
        let epoch = tab.epoch;
        self.terminal_drawer.composition = (!value.is_empty()).then_some(Composition {
            id,
            scope: self.environment_scope.clone(),
            thread,
            epoch,
            generation: self.generation,
            text: value,
            selection: selected,
        });
        cx.notify();
    }

    fn bounds_for_range(
        &mut self,
        _: Range<usize>,
        bounds: Bounds<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let cell = self
            .terminal_active()?
            .screen
            .cells()
            .into_iter()
            .find(|cell| cell.cursor)?;
        Some(Bounds::new(
            point(
                bounds.origin.x + px(cell.col as f32 * self.terminal_drawer.cell_width),
                bounds.origin.y + px(cell.row as f32 * self.terminal_drawer.line_height),
            ),
            size(
                px(self.terminal_drawer.cell_width),
                px(self.terminal_drawer.line_height),
            ),
        ))
    }

    fn character_index_for_point(
        &mut self,
        _: Point<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<usize> {
        Some(
            self.terminal_composition()
                .map_or(0, |mark| mark.selection.end),
        )
    }

    fn accepts_text_input(&self, window: &mut Window, _: &mut Context<Self>) -> bool {
        self.terminal_drawer.focus.is_focused(window) && self.terminal_text_target().is_some()
    }
}
