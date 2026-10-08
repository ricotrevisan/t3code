//! Opt-in event-boundary evidence for real-window failures that the mock
//! platform cannot reproduce. Never log prompts, URLs, credentials or payloads.
use crate::NativeApp;
use gpui_kit::{
    App, Context, DispatchPhase, IntoElement, MouseDownEvent, MouseUpEvent, Window, canvas,
    prelude::*,
};
use std::sync::OnceLock;

pub(crate) fn enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var_os("T3_NATIVE_TRACE_INPUT").is_some())
}

#[cfg(all(target_os = "macos", not(test)))]
fn responder() -> String {
    use objc2::MainThreadMarker;
    use objc2_app_kit::NSApplication;
    MainThreadMarker::new()
        .and_then(|mtm| NSApplication::sharedApplication(mtm).keyWindow())
        .and_then(|window| window.firstResponder())
        .map(|responder| responder.class().name().to_string_lossy().into_owned())
        .unwrap_or_else(|| "none".into())
}
#[cfg(any(not(target_os = "macos"), test))]
fn responder() -> String {
    "mock/non-AppKit".into()
}

pub(crate) fn mouse(
    layer: &'static str,
    app: &NativeApp,
    event: &MouseDownEvent,
    window: &Window,
    cx: &App,
) {
    if enabled() {
        eprintln!(
            "[t3-input] {layer} down position={:?} first_mouse={} button={:?} gpui_focus={:?} native_responder={} selected={} terminal={} overlays={}/{}/{}",
            event.position,
            event.first_mouse,
            event.button,
            window.focused(cx),
            responder(),
            app.selected_thread.is_some(),
            app.terminal_drawer.open,
            app.palette_open,
            app.settings_open,
            app.branch_picker.is_some(),
        );
    }
}

pub(crate) fn root_mouse(
    app: &NativeApp,
    event: &MouseDownEvent,
    window: &Window,
    cx: &mut Context<NativeApp>,
) {
    mouse("root", app, event, window, cx);
    if enabled() {
        let entity = cx.entity().downgrade();
        cx.defer(move |cx| {
            let _ = entity.update(cx, |app, _| {
                eprintln!(
                    "[t3-input] after-down selected={:?} pending_actions={}",
                    app.selected_thread, app.pending_actions
                );
            });
        });
    }
}

/// This canvas paints no hitbox and its window listener deliberately does not
/// check hover. If it logs a press but root/status do not, GPUI occlusion (not a
/// native child swallowing the event) is implicated.
pub(crate) fn listener(cx: &Context<NativeApp>) -> Option<impl IntoElement + use<>> {
    if !enabled() {
        return None;
    }
    let app = cx.entity().downgrade();
    Some(
        canvas(
            |_, _, _| (),
            move |_, _, window, _| {
                let app = app.clone();
                let up = app.clone();
                window.on_mouse_event(move |event: &MouseDownEvent, phase, window, cx| {
                    if phase == DispatchPhase::Capture {
                        let _ = app.update(cx, |app, cx| mouse("window", app, event, window, cx));
                    }
                });
                window.on_mouse_event(move |_: &MouseUpEvent, phase, _, cx| {
                    if phase == DispatchPhase::Capture {
                        let _ = up.update(cx, |_, cx| root_mouse_up(cx));
                    }
                });
            },
        )
        .absolute()
        .size_full(),
    )
}

pub(crate) fn root_mouse_up(cx: &mut Context<NativeApp>) {
    if enabled() {
        let entity = cx.entity().downgrade();
        cx.defer(move |cx| {
            let _ = entity.update(cx, |app, _| {
                eprintln!(
                    "[t3-input] after-up selected={:?} pending_actions={}",
                    app.selected_thread, app.pending_actions
                );
            });
        });
    }
}

pub(crate) fn action(
    action: &'static str,
    event: Option<super::keystrokes::EventIdentity>,
    accepted: bool,
) {
    if enabled() {
        eprintln!(
            "[t3-input] action={action} event={event:?} accepted={accepted} native_responder={}",
            responder()
        );
    }
}
