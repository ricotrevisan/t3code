//! Deduplicate menu and focused-view deliveries by AppKit event identity, not
//! elapsed time. A separate key-repeat event is a separate user action.
use crate::NativeApp;
use gpui_kit::{App, Context, Window};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct EventIdentity {
    timestamp: u64,
    key_code: u16,
    window: isize,
}

#[cfg(all(target_os = "macos", not(test)))]
fn current_event(_cx: &App) -> Option<EventIdentity> {
    use objc2::MainThreadMarker;
    use objc2_app_kit::{NSApplication, NSEventType};
    let application = NSApplication::sharedApplication(MainThreadMarker::new()?);
    let event = application.currentEvent()?;
    if event.r#type() != NSEventType::KeyDown {
        return None;
    }
    Some(EventIdentity {
        timestamp: event.timestamp().to_bits(),
        key_code: event.keyCode(),
        window: event.windowNumber(),
    })
}

#[cfg(all(not(target_os = "macos"), not(test)))]
fn current_event(_cx: &App) -> Option<EventIdentity> {
    None
}

#[cfg(test)]
#[derive(Default)]
struct TestEvent(Option<EventIdentity>);
#[cfg(test)]
impl gpui_kit::Global for TestEvent {}
#[cfg(test)]
fn current_event(cx: &App) -> Option<EventIdentity> {
    cx.try_global::<TestEvent>().and_then(|event| event.0)
}

/// Simulate AppKit's currentEvent boundary, not feature state or receipts.
#[cfg(test)]
pub(crate) fn simulate_native_event(cx: &mut App, timestamp: u64, key_code: u16) {
    cx.set_global(TestEvent(Some(EventIdentity {
        timestamp,
        key_code,
        window: 1,
    })));
}

#[cfg(test)]
pub(crate) fn simulate_menu_click(cx: &mut App) {
    cx.set_global(TestEvent(None));
}

/// Headless and non-AppKit dispatch lacks a native event identity. Start a new
/// batch on each captured keydown; duplicate programmatic actions in the same
/// dispatch batch share the set until the deferred queue drains. No clock gate.
pub(crate) fn begin_key(app: &mut NativeApp, cx: &App) {
    if current_event(cx).is_none() {
        app.keystroke_actions.clear();
    }
}

pub(crate) fn accept(
    app: &mut NativeApp,
    action: &'static str,
    _window: &mut Window,
    cx: &mut Context<NativeApp>,
) -> bool {
    if !app.keystroke_guard {
        return true;
    }
    let event = current_event(cx);
    if event != app.keystroke_event {
        app.keystroke_actions.clear();
        app.keystroke_event = event;
    }
    if !app.keystroke_actions.insert(action) {
        super::input_trace::action(action, event, false);
        return false;
    }
    super::input_trace::action(action, event, true);
    if event.is_none() {
        // Non-keyboard menu clicks must never inherit a prior keyboard event.
        // No timer: a second user interaction in the next dispatch may act.
        let app = cx.entity().downgrade();
        cx.defer(move |cx| {
            let _ = app.update(cx, |app, _| {
                if app.keystroke_event.is_none() {
                    app.keystroke_actions.clear();
                }
            });
        });
    }
    true
}
