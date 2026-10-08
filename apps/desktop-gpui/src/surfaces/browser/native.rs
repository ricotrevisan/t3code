//! The only unsafe boundary: main-thread WebKit attached to GPUI's actual NSView.
use super::model::{Navigation, Profile};
use gpui_kit::{Bounds, Pixels, Window};
use objc2::{
    DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send,
    rc::Retained,
    runtime::{AnyObject, ProtocolObject},
};
use objc2_app_kit::{NSView, NSWindowDidChangeOcclusionStateNotification, NSWindowOcclusionState};
use objc2_foundation::{
    NSDictionary, NSError, NSKeyValueChangeKey, NSKeyValueObservingOptions, NSNotificationCenter,
    NSObject, NSObjectNSKeyValueObserverRegistration as _, NSObjectProtocol, NSOperationQueue,
    NSPoint, NSRect, NSSize, NSString, NSURL, NSURLRequest,
};
use objc2_web_kit::{
    WKNavigation, WKNavigationDelegate, WKWebView, WKWebViewConfiguration, WKWebsiteDataStore,
};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

const OBSERVED_KEYS: [&str; 5] = ["URL", "title", "canGoBack", "canGoForward", "loading"];

pub struct DelegateState {
    changed: Box<dyn Fn(Navigation)>,
    error: std::cell::RefCell<Option<String>>,
}

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "T3NativeBrowserNavigationDelegate"]
    #[ivars = DelegateState]
    struct NavigationDelegate;
    unsafe impl NSObjectProtocol for NavigationDelegate {}
    impl NavigationDelegate {
        #[unsafe(method(observeValueForKeyPath:ofObject:change:context:))]
        fn observed(&self, _key: Option<&NSString>, object: Option<&AnyObject>, _change: Option<&NSDictionary<NSKeyValueChangeKey, AnyObject>>, _context: *mut std::ffi::c_void) {
            if let Some(view) = object.and_then(|object| object.downcast_ref::<WKWebView>()) {
                (self.ivars().changed)(snapshot(view, self.ivars().error.borrow().clone()));
            }
        }
    }
    unsafe impl WKNavigationDelegate for NavigationDelegate {
        #[unsafe(method(webView:didStartProvisionalNavigation:))]
        fn started(&self, view: &WKWebView, _navigation: Option<&WKNavigation>) {
            self.publish(view, None);
        }
        #[unsafe(method(webView:didCommitNavigation:))]
        fn committed(&self, view: &WKWebView, _navigation: Option<&WKNavigation>) {
            self.publish(view, None);
        }
        #[unsafe(method(webView:didFinishNavigation:))]
        fn finished(&self, view: &WKWebView, _navigation: Option<&WKNavigation>) {
            self.publish(view, None);
        }
        #[unsafe(method(webView:didFailProvisionalNavigation:withError:))]
        fn failed_start(
            &self,
            view: &WKWebView,
            _navigation: Option<&WKNavigation>,
            error: &NSError,
        ) {
            // Cancellation is normal during a second navigation or tab close.
            if error.code() != -999 {
                self.publish(view, Some(error.localizedDescription().to_string()));
            }
        }
        #[unsafe(method(webView:didFailNavigation:withError:))]
        fn failed(&self, view: &WKWebView, _navigation: Option<&WKNavigation>, error: &NSError) {
            if error.code() != -999 {
                self.publish(view, Some(error.localizedDescription().to_string()));
            }
        }
        #[unsafe(method(webViewWebContentProcessDidTerminate:))]
        fn terminated(&self, view: &WKWebView) {
            self.publish(
                view,
                Some("Web content process stopped. Reload to recover.".into()),
            );
        }
    }
);

impl NavigationDelegate {
    fn publish(&self, view: &WKWebView, error: Option<String>) {
        self.ivars().error.replace(error.clone());
        (self.ivars().changed)(snapshot(view, error));
    }
}

fn snapshot(view: &WKWebView, error: Option<String>) -> Navigation {
    // SAFETY: WebKit delegate and all callers run on the AppKit main thread.
    unsafe {
        Navigation {
            url: view
                .URL()
                .and_then(|url| url.absoluteString())
                .map(|s| s.to_string())
                .unwrap_or_default(),
            title: view
                .title()
                .map(|s| s.to_string().chars().take(512).collect())
                .unwrap_or_default(),
            loading: error.is_none() && view.isLoading(),
            can_go_back: view.canGoBack(),
            can_go_forward: view.canGoForward(),
            error,
        }
    }
}

/// Retained by one browser tab, never by the delegate callback. Drop removes the
/// child view and navigation delegate before releasing WebKit and its profile.
pub struct EmbeddedBrowser {
    view: Retained<WKWebView>,
    clip: Retained<NSView>,
    delegate: Retained<NavigationDelegate>,
    parent: Retained<NSView>,
    occlusion_observer: Option<Retained<ProtocolObject<dyn NSObjectProtocol>>>,
}

/// GPUI has a top-left origin; AppKit views may instead be bottom-left based.
pub fn appkit_rect(bounds: Bounds<Pixels>, parent_height: f64, flipped: bool) -> NSRect {
    let x = f64::from(bounds.origin.x);
    let y = f64::from(bounds.origin.y);
    let width = f64::from(bounds.size.width);
    let height = f64::from(bounds.size.height);
    NSRect::new(
        NSPoint::new(
            x,
            if flipped {
                y
            } else {
                parent_height - y - height
            },
        ),
        NSSize::new(width, height),
    )
}

impl EmbeddedBrowser {
    pub fn attach(
        window: &Window,
        profile: Profile,
        changed: impl Fn(Navigation) + 'static,
    ) -> Result<Self, String> {
        let mtm = MainThreadMarker::new().ok_or("WebKit must attach on the main thread")?;
        let handle = HasWindowHandle::window_handle(window)
            .map_err(|error| format!("Native window handle unavailable: {error}"))?;
        let RawWindowHandle::AppKit(handle) = handle.as_raw() else {
            return Err("Embedded browser requires an AppKit window".into());
        };
        // SAFETY: GPUI's borrowed raw handle points to its live NSView. Retaining
        // the parent keeps it valid until our child has been removed in Drop.
        let parent = unsafe { Retained::retain(handle.ns_view.as_ptr().cast::<NSView>()) }
            .ok_or("GPUI window has no NSView")?;
        let clip = NSView::initWithFrame(NSView::alloc(mtm), NSRect::ZERO);
        // Sonoma exposes explicit view clipping; older AppKit clips implicitly.
        if clip.respondsToSelector(objc2::sel!(setClipsToBounds:)) {
            clip.setClipsToBounds(true);
        }
        clip.setWantsLayer(true);
        if let Some(layer) = clip.layer() {
            layer.setMasksToBounds(true);
        }
        clip.setHidden(true);
        // SAFETY: objects are initialized/owned here on the main thread. The
        // profile is selected BEFORE WKWebView construction, never after attach.
        let (view, delegate) = unsafe {
            let config = WKWebViewConfiguration::new(mtm);
            let store = match profile {
                Profile::Default => WKWebsiteDataStore::defaultDataStore(mtm),
                Profile::Incognito => WKWebsiteDataStore::nonPersistentDataStore(mtm),
            };
            config.setWebsiteDataStore(&store);
            let view = WKWebView::initWithFrame_configuration(
                WKWebView::alloc(mtm),
                NSRect::ZERO,
                &config,
            );
            let delegate = NavigationDelegate::alloc(mtm).set_ivars(DelegateState {
                changed: Box::new(changed),
                error: Default::default(),
            });
            let delegate: Retained<NavigationDelegate> = msg_send![super(delegate), init];
            view.setNavigationDelegate(Some(ProtocolObject::from_ref(&*delegate)));
            (view, delegate)
        };
        // URL/history also change via pushState and same-document navigation,
        // which do not necessarily emit navigation-delegate completion events.
        for key in OBSERVED_KEYS {
            unsafe {
                view.addObserver_forKeyPath_options_context(
                    &delegate,
                    &NSString::from_str(key),
                    NSKeyValueObservingOptions::New,
                    std::ptr::null_mut(),
                );
            }
        }
        clip.addSubview(&view);
        parent.addSubview(&clip);
        let occlusion_observer = parent.window().map(|window| {
            let clip = clip.clone();
            let view = view.clone();
            let delegate = delegate.clone();
            let observed_window = window.clone();
            let callback = block2::RcBlock::new(move |_| {
                if !observed_window
                    .occlusionState()
                    .contains(NSWindowOcclusionState::Visible)
                {
                    clip.setHidden(true);
                }
                // Invalidate once on reveal as well: a hidden native child
                // must be laid out again even when GPUI's scene did not change.
                (delegate.ivars().changed)(snapshot(
                    &view,
                    delegate.ivars().error.borrow().clone(),
                ));
            });
            // SAFETY: the observer is scoped to this NSWindow and dispatched on
            // the main operation queue; its AppKit captures never leave it.
            unsafe {
                NSNotificationCenter::defaultCenter().addObserverForName_object_queue_usingBlock(
                    Some(NSWindowDidChangeOcclusionStateNotification),
                    Some(&window),
                    Some(&NSOperationQueue::mainQueue()),
                    &callback,
                )
            }
        });
        Ok(Self {
            view,
            clip,
            delegate,
            parent,
            occlusion_observer,
        })
    }

    /// Called by the actual canvas paint, with the current GPUI content mask.
    /// Clip the native host, not the page viewport: scrolling the panel must not
    /// reflow the document or expose native pixels outside the panel.
    pub fn layout(&self, bounds: Bounds<Pixels>, mask: Bounds<Pixels>) {
        let clipped = bounds.intersect(&mask);
        let visible = !clipped.is_empty()
            && self.parent.window().is_some_and(|window| {
                window
                    .occlusionState()
                    .contains(NSWindowOcclusionState::Visible)
            });
        if !visible {
            self.hide();
            return;
        }
        self.clip.setFrame(appkit_rect(
            clipped,
            self.parent.bounds().size.height,
            self.parent.isFlipped(),
        ));
        let origin_x = f64::from(bounds.origin.x - clipped.origin.x);
        let origin_y = f64::from(
            clipped.origin.y + clipped.size.height - bounds.origin.y - bounds.size.height,
        );
        self.view.setFrame(NSRect::new(
            NSPoint::new(origin_x, origin_y),
            NSSize::new(f64::from(bounds.size.width), f64::from(bounds.size.height)),
        ));
        self.clip.setHidden(false);
    }
    /// GPUI focus handles do not change AppKit's first responder. Return it
    /// only when this page (or its WebKit content descendant) currently owns it;
    /// normal page clicks remain responsible for taking native focus again.
    pub fn release_focus(&self) {
        if let Some(window) = self.parent.window()
            && let Some(responder) = window.firstResponder()
            && let Some(view) = responder.downcast_ref::<NSView>()
            && view.isDescendantOf(&self.clip)
        {
            // A GPUI input's focus ring alone cannot route insertText/IME or
            // shortcuts away from the native WebKit child.
            window.makeFirstResponder(Some(&self.parent));
        }
    }
    pub fn hide(&self) {
        self.release_focus();
        self.clip.setHidden(true);
    }
    pub fn navigate(&self, url: &url::Url) -> Result<(), String> {
        let url = NSURL::URLWithString(&NSString::from_str(url.as_str()))
            .ok_or("WebKit rejected the URL")?;
        let request = NSURLRequest::requestWithURL(&url);
        // SAFETY: the request is a live NSURLRequest on the main thread.
        unsafe { self.view.loadRequest(&request) }.ok_or("WebKit could not start navigation")?;
        Ok(())
    }
    pub fn back(&self) {
        unsafe {
            self.view.goBack();
        }
    }
    pub fn forward(&self) {
        unsafe {
            self.view.goForward();
        }
    }
    pub fn reload(&self) {
        unsafe {
            self.view.reload();
        }
    }
}
impl Drop for EmbeddedBrowser {
    fn drop(&mut self) {
        // SAFETY: EmbeddedBrowser is !Send (main-thread AppKit views).
        if let Some(observer) = self.occlusion_observer.take() {
            unsafe {
                NSNotificationCenter::defaultCenter().removeObserver((*observer).as_ref());
            }
        }
        self.hide();
        unsafe {
            for key in OBSERVED_KEYS {
                self.view
                    .removeObserver_forKeyPath(&self.delegate, &NSString::from_str(key));
            }
            self.view.setNavigationDelegate(None);
            self.view.stopLoading();
        }
        self.clip.removeFromSuperview();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::{point, px, size};
    #[test]
    fn browser_native_geometry_converts_both_origins_without_display_scale_multiplication() {
        let bounds = Bounds::new(point(px(600.), px(100.)), size(px(320.), px(450.)));
        assert_eq!(
            appkit_rect(bounds, 800., false),
            NSRect::new(NSPoint::new(600., 250.), NSSize::new(320., 450.))
        );
        assert_eq!(appkit_rect(bounds, 800., true).origin.y, 100.);
    }
}
