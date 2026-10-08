//! AppKit work areas. GPUI's macOS display bounds are local to each screen;
//! persisted rectangles instead use global top-left coordinates.
use gpui_kit::{App, DisplayId, Task, Window};
use objc2::{MainThreadMarker, msg_send, rc::Retained, runtime::ProtocolObject};
use objc2_app_kit::{
    NSApplicationDidChangeScreenParametersNotification, NSScreen, NSView, NSWindow,
    NSWindowDidChangeScreenNotification, NSWindowDidEndLiveResizeNotification,
    NSWindowDidExitFullScreenNotification, NSWindowDidResizeNotification, NSWindowStyleMask,
};
use objc2_foundation::{
    NSNotificationCenter, NSObjectProtocol, NSOperationQueue, NSPoint, NSRect, NSSize, NSString,
};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use std::{cell::Cell, rc::Rc, time::Duration};
use t3code_desktop_gpui::window_state::{WindowState, clamp_rect};

struct Screen {
    id: DisplayId,
    frame: NSRect,
    visible: NSRect,
}

fn screens() -> Vec<Screen> {
    let Some(mtm) = MainThreadMarker::new() else {
        return Vec::new();
    };
    NSScreen::screens(mtm)
        .iter()
        .map(|screen| {
            let number = screen
                .deviceDescription()
                .objectForKey(&NSString::from_str("NSScreenNumber"))
                .expect("NSScreen has a display number");
            // SAFETY: AppKit's NSScreenNumber is an NSNumber containing CGDirectDisplayID.
            let id: u32 = unsafe { msg_send![&*number, unsignedIntValue] };
            Screen {
                id: DisplayId::new(id.into()),
                frame: screen.frame(),
                visible: screen.visibleFrame(),
            }
        })
        .collect()
}

fn rect(rect: NSRect) -> [f32; 4] {
    [
        rect.origin.x as f32,
        rect.origin.y as f32,
        rect.size.width as f32,
        rect.size.height as f32,
    ]
}

fn top_rect(rect: NSRect, primary_top: f64) -> [f32; 4] {
    [
        rect.origin.x as f32,
        (primary_top - rect.origin.y - rect.size.height) as f32,
        rect.size.width as f32,
        rect.size.height as f32,
    ]
}

pub(super) fn initial(state: WindowState) -> Option<([f32; 4], DisplayId)> {
    initial_for_screens(state, &screens())
}

fn initial_for_screens(state: WindowState, screens: &[Screen]) -> Option<([f32; 4], DisplayId)> {
    let top = screens.first()?.frame.size.height;
    let areas: Vec<_> = screens.iter().map(|s| top_rect(s.visible, top)).collect();
    let restored = state.restore_rect(&areas)?;
    let index = areas
        .iter()
        .position(|a| clamp_rect(restored, &[*a]) == Some(restored))?;
    let screen = &screens[index];
    // Creation takes display-local top-left coordinates, unlike saved state.
    let mut local = restored;
    local[0] -= screen.frame.origin.x as f32;
    local[1] -= (top - screen.frame.origin.y - screen.frame.size.height) as f32;
    Some((local, screen.id))
}

fn native_window(window: &Window) -> Option<Retained<NSWindow>> {
    let RawWindowHandle::AppKit(handle) = HasWindowHandle::window_handle(window).ok()?.as_raw()
    else {
        return None;
    };
    // SAFETY: the borrowed handle is GPUI's live NSView, accessed on the UI thread.
    let view = unsafe { Retained::retain(handle.ns_view.as_ptr().cast::<NSView>()) }?;
    view.window()
}

pub(super) fn remember(state: &mut WindowState, window: &Window) -> bool {
    let Some(native) = native_window(window) else {
        return false;
    };
    let screens = screens();
    let Some(primary) = screens.first() else {
        return false;
    };
    state.maximized = window.is_maximized();
    if !state.maximized && !window.is_fullscreen() {
        let areas: Vec<_> = screens
            .iter()
            .map(|s| top_rect(s.visible, primary.frame.size.height))
            .collect();
        if let Some([x, y, w, h]) =
            clamp_rect(top_rect(native.frame(), primary.frame.size.height), &areas)
        {
            state.x = Some(x);
            state.y = Some(y);
            state.width = w;
            state.height = h;
        }
    }
    true
}

struct Placement {
    window: Retained<NSWindow>,
    adjusting: Cell<bool>,
    minimum_content_size: NSSize,
}

impl Placement {
    fn reconcile(&self) {
        if self.adjusting.replace(true) {
            return;
        }
        // Fullscreen belongs to AppKit, not the desktop work area. On exit the
        // notification below clamps the restored normal rectangle again.
        if !self.window.inLiveResize()
            && !self
                .window
                .styleMask()
                .contains(NSWindowStyleMask::FullScreen)
        {
            let screens = screens();
            let areas: Vec<_> = screens.iter().map(|s| rect(s.visible)).collect();
            let current = rect(self.window.frame());
            if let Some([x, y, w, h]) = clamp_rect(current, &areas) {
                let target = NSRect::new(
                    NSPoint::new(x.into(), y.into()),
                    NSSize::new(w.into(), h.into()),
                );
                let content = self.window.contentRectForFrameRect(target).size;
                // AppKit's content minimum must not defeat an outer-frame clamp
                // on a small work area. Restore it when sufficient space returns.
                let area = areas
                    .iter()
                    .find(|area| clamp_rect([x, y, w, h], &[**area]) == Some([x, y, w, h]));
                if let Some(area) = area {
                    let available = self
                        .window
                        .contentRectForFrameRect(NSRect::new(
                            NSPoint::new(area[0].into(), area[1].into()),
                            NSSize::new(area[2].into(), area[3].into()),
                        ))
                        .size;
                    let minimum = NSSize::new(
                        self.minimum_content_size.width.min(available.width),
                        self.minimum_content_size.height.min(available.height),
                    );
                    if self.window.contentMinSize() != minimum {
                        self.window.setContentMinSize(minimum);
                    }
                }
                if current != [x, y, w, h] && content.width > 0. && content.height > 0. {
                    self.window.setFrame_display(target, true);
                }
            }
        }
        self.adjusting.set(false);
    }
}

pub(crate) struct Guard {
    observers: Vec<Retained<ProtocolObject<dyn NSObjectProtocol>>>,
    _work_area_watch: Task<()>,
}

impl Guard {
    pub(crate) fn install(window: &Window, cx: &App) -> Option<Self> {
        let native = native_window(window)?;
        let placement = Rc::new(Placement {
            minimum_content_size: native.contentMinSize(),
            window: native,
            adjusting: Cell::new(false),
        });
        placement.reconcile();
        let center = NSNotificationCenter::defaultCenter();
        let mut observers = Vec::new();
        for (name, scoped) in [
            (
                unsafe { NSApplicationDidChangeScreenParametersNotification },
                false,
            ),
            (unsafe { NSWindowDidChangeScreenNotification }, true),
            (unsafe { NSWindowDidEndLiveResizeNotification }, true),
            (unsafe { NSWindowDidResizeNotification }, true),
            (unsafe { NSWindowDidExitFullScreenNotification }, true),
        ] {
            let placement = placement.clone();
            let observed = scoped.then(|| placement.window.clone());
            let callback = block2::RcBlock::new(move |_| placement.reconcile());
            // SAFETY: AppKit objects are retained and the callback only runs on
            // the main queue. Guard removes every observer when the view drops.
            observers.push(unsafe {
                center.addObserverForName_object_queue_usingBlock(
                    Some(name),
                    observed.as_deref().map(|w| w.as_ref()),
                    Some(&NSOperationQueue::mainQueue()),
                    &callback,
                )
            });
        }
        // Dock animation/preferences do not reliably emit screen-parameter
        // notifications on every macOS version. Check only work-area changes;
        // this timer never invalidates a GPUI frame or fights an in-bounds resize.
        let executor = cx.background_executor().clone();
        let work_area_watch = cx.spawn(async move |cx| {
            let mut previous: Vec<_> = screens().iter().map(|s| (s.id, rect(s.visible))).collect();
            loop {
                executor.timer(Duration::from_millis(500)).await;
                cx.update(|_| {
                    let current: Vec<_> =
                        screens().iter().map(|s| (s.id, rect(s.visible))).collect();
                    if current != previous {
                        previous = current;
                        placement.reconcile();
                    }
                });
            }
        });
        Some(Self {
            observers,
            _work_area_watch: work_area_watch,
        })
    }
}

impl Drop for Guard {
    fn drop(&mut self) {
        let center = NSNotificationCenter::defaultCenter();
        for observer in &self.observers {
            unsafe { center.removeObserver(observer.as_ref()) };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn screen(id: u64, frame: [f64; 4], visible: [f64; 4]) -> Screen {
        let ns = |[x, y, w, h]: [f64; 4]| NSRect::new(NSPoint::new(x, y), NSSize::new(w, h));
        Screen {
            id: DisplayId::new(id),
            frame: ns(frame),
            visible: ns(visible),
        }
    }

    #[test]
    fn r16_win_appkit_work_area_to_global_top_left() {
        let primary = screen(1, [0., 0., 1920., 1080.], [0., 80., 1920., 975.]);
        assert_eq!(top_rect(primary.visible, 1080.), [0., 25., 1920., 975.]);
        let above = screen(
            2,
            [-1600., 1080., 1600., 900.],
            [-1520., 1080., 1520., 875.],
        );
        assert_eq!(top_rect(above.visible, 1080.), [-1520., -875., 1520., 875.]);
    }

    #[test]
    fn r16_win_appkit_restore_passes_target_display_and_local_origin() {
        let screens = [
            screen(1, [0., 0., 1920., 1080.], [0., 80., 1920., 975.]),
            screen(
                2,
                [-1600., 1080., 1600., 900.],
                [-1520., 1080., 1520., 875.],
            ),
        ];
        for maximized in [false, true] {
            let state = WindowState {
                x: Some(-1500.),
                y: Some(-850.),
                width: 1280.,
                height: 800.,
                maximized,
                ..WindowState::default()
            };
            assert_eq!(
                initial_for_screens(state, &screens),
                Some(([100., 50., 1280., 800.], DisplayId::new(2)))
            );
            assert_eq!(
                initial_for_screens(state, &screens[..1]),
                Some(([320., 112.5, 1280., 800.], DisplayId::new(1)))
            );
        }
    }
}
