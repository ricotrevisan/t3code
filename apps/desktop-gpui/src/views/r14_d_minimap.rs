//! A painted, bounded overview. Pointer position still addresses every user anchor.
use super::*;
use gpui_kit::{
    Bounds, MouseButton, MouseDownEvent, MouseMoveEvent, Pixels, canvas, fill, point, size,
};
use std::cell::Cell;

pub(crate) fn pointer_index(y: f32, height: f32, count: usize) -> Option<usize> {
    if count == 0 || !y.is_finite() || !height.is_finite() || height <= 0. {
        return None;
    }
    Some(((y / height).clamp(0., 1.) * count as f32).floor() as usize).map(|i| i.min(count - 1))
}

impl NativeApp {
    pub(super) fn render_timeline_minimap(&mut self, cx: &mut Context<Self>) -> gpui_kit::Div {
        let items = self.conversation_presentation.navigation.clone();
        let count = items.len();
        if count < 2 {
            return div();
        }
        let focus = self
            .conversation_presentation
            .minimap_focus
            .get_or_insert_with(|| cx.focus_handle())
            .clone();
        let active = self
            .conversation_presentation
            .minimap_active
            .map(|i| i.min(count - 1));
        let top = self.conversation_scroll.logical_scroll_top().item_ix;
        let current = items
            .partition_point(|item| item.row <= top)
            .saturating_sub(1);
        let muted = cx.theme().muted_foreground;
        let foreground = cx.theme().foreground;
        let bounds: Rc<Cell<Option<Bounds<Pixels>>>> = Rc::new(Cell::new(None));
        let paint_bounds = bounds.clone();
        let click_bounds = bounds.clone();
        let move_bounds = bounds.clone();
        let scope = (
            self.environment_scope.clone(),
            self.selected_thread.clone(),
            self.generation,
        );
        let click_scope = scope.clone();
        let key_items = items.clone();
        let preview = active.map(|i| items[i].preview.clone());
        div()
            .flex()
            .flex_col()
            .justify_center()
            .flex_shrink_0()
            .w_8()
            .child(
                div()
                    .id("timeline-minimap")
                    .debug_selector(|| "timeline-minimap".into())
                    .track_focus(&focus)
                    .aria_label("Timeline message navigator · arrow keys to choose, Enter to jump")
                    .relative()
                    .h_48()
                    .w_6()
                    .child(
                        canvas(
                            move |rect, _, _| {
                                paint_bounds.set(Some(rect));
                                rect
                            },
                            move |_, rect, window, _| {
                                // At most 80 painted ticks, independent of history size. The hit map is not sampled.
                                let ticks = count.min(80);
                                for i in 0..ticks {
                                    let highlighted =
                                        i == active.unwrap_or(current) * ticks / count;
                                    let y = rect.origin.y
                                        + rect.size.height * ((i as f32 + 0.5) / ticks as f32);
                                    window.paint_quad(fill(
                                        Bounds::new(
                                            point(rect.origin.x + rect.size.width / 3., y),
                                            size(rect.size.width * if highlighted { 2. / 3. } else { 1. / 3. }, window.rem_size() / 8.),
                                        ),
                                        if highlighted {
                                            foreground
                                        } else {
                                            muted.opacity(0.45)
                                        },
                                    ));
                                }
                            },
                        )
                        .size_full(),
                    )
                    .when_some(preview, |view, preview| {
                        view.child(
                            gpui_kit::deferred(div()
                                .absolute()
                                .left_8()
                                .top_0()
                                .w_64()
                                .p_2()
                                .rounded_md()
                                .bg(cx.theme().popover)
                                .text_color(cx.theme().popover_foreground)
                                .text_xs()
                                .child(preview)),
                        )
                    })
                    .on_mouse_move(cx.listener(move |app, event: &MouseMoveEvent, _, cx| {
                        let Some(rect) = move_bounds.get() else {
                            return;
                        };
                        let next = pointer_index(
                            (event.position.y - rect.origin.y).into(),
                            rect.size.height.into(),
                            count,
                        );
                        if app.conversation_presentation.minimap_active != next {
                            app.conversation_presentation.minimap_active = next;
                            cx.notify();
                        }
                    }))
                    .on_hover(cx.listener(|app, hovered, window, cx| {
                        if !hovered && !app.conversation_presentation.minimap_focus.as_ref().is_some_and(|focus| focus.is_focused(window)) {
                            app.conversation_presentation.minimap_active = None;
                            cx.notify();
                        }
                    }))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |app, event: &MouseDownEvent, window, cx| {
                            if click_scope
                                != (
                                    app.environment_scope.clone(),
                                    app.selected_thread.clone(),
                                    app.generation,
                                )
                            {
                                return;
                            }
                            let Some(rect) = click_bounds.get() else {
                                return;
                            };
                            if let Some(index) = pointer_index(
                                (event.position.y - rect.origin.y).into(),
                                rect.size.height.into(),
                                count,
                            ) {
                                window.prevent_default();
                                gpui_kit::component::global_state::GlobalState::suppress_text_selection(cx);
                                focus.focus(window, cx);
                                app.conversation_presentation.minimap_active = Some(index);
                                app.scroll_to_message(&items[index].message);
                                cx.notify();
                                cx.stop_propagation();
                            }
                        }),
                    )
                    .on_key_down(
                        cx.listener(move |app, event: &gpui_kit::KeyDownEvent, _, cx| {
                            if scope
                                != (
                                    app.environment_scope.clone(),
                                    app.selected_thread.clone(),
                                    app.generation,
                                )
                            {
                                return;
                            }
                            let index = app
                                .conversation_presentation
                                .minimap_active
                                .unwrap_or(current)
                                .min(count - 1);
                            let next = match event.keystroke.key.as_str() {
                                "up" => index.saturating_sub(1),
                                "down" => (index + 1).min(count - 1),
                                "home" => 0,
                                "end" => count - 1,
                                "enter" | "space" => {
                                    app.scroll_to_message(&key_items[index].message);
                                    cx.notify();
                                    cx.stop_propagation();
                                    return;
                                }
                                _ => return,
                            };
                            app.conversation_presentation.minimap_active = Some(next);
                            cx.notify();
                            cx.stop_propagation();
                        }),
                    ),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn r14_d_minimap_hit_map_covers_unsampled_anchors_and_edges() {
        assert_eq!(pointer_index(90., 180., 1000), Some(500));
        assert_eq!(pointer_index(180., 180., 1000), Some(999));
        assert_eq!(pointer_index(-1., 180., 1000), Some(0));
        assert_eq!(pointer_index(1., 0., 1000), None);
        assert_eq!(pointer_index(f32::NAN, 180., 1000), None);
        assert_eq!(pointer_index(1., 180., 0), None);
    }
}
