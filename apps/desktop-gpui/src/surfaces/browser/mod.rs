//! Native browser surfaces. WebKit owns document input/history; GPUI owns chrome.
//! Call `sync_visibility` on EVERY shell render, even when this entity is omitted
//! from the tree. Native children are outside GPUI's normal clipping/overlay tree.
pub mod model;
#[cfg(target_os = "macos")]
mod native;
mod view;

use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::{AppContext as _, Context, Entity, EventEmitter, Subscription, Window};
use model::{Navigation, Profile, Tabs};
use std::collections::HashMap;

pub struct BrowserTab {
    pub address: Entity<InputState>,
    pub navigation: Navigation,
    pub pending_url: Option<url::Url>,
    #[cfg(target_os = "macos")]
    native: Option<native::EmbeddedBrowser>,
    _subscription: Subscription,
}

/// The parent listens to this event to notify its built-in-body selection.
pub struct SurfaceSelectionChanged;

pub struct BrowserPanel {
    pub tabs: Tabs,
    browsers: HashMap<u64, BrowserTab>,
    pub error: Option<String>,
    visible: bool,
    scope: Option<String>,
    window_handle: gpui_kit::AnyWindowHandle,
    rename: Entity<InputState>,
    renaming: Option<u64>,
    _subscriptions: Vec<Subscription>,
}
impl EventEmitter<SurfaceSelectionChanged> for BrowserPanel {}

impl BrowserPanel {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let rename = cx.new(|cx| InputState::new(window, cx).placeholder("Tab name"));
        let subscription = cx.subscribe_in(&rename, window, |panel, _, event, _, cx| {
            if matches!(event, InputEvent::PressEnter { .. }) {
                panel.finish_rename(cx);
            }
        });
        Self {
            tabs: Tabs::default(),
            browsers: HashMap::new(),
            error: None,
            visible: false,
            scope: None,
            window_handle: window.window_handle(),
            rename,
            renaming: None,
            _subscriptions: vec![subscription],
        }
    }
    pub fn browser_active(&self) -> bool {
        self.tabs.active.is_some()
    }
    pub fn deactivate(&mut self, cx: &mut Context<Self>) {
        self.tabs.active = None;
        self.hide_all();
        cx.emit(SurfaceSelectionChanged);
        cx.notify();
    }
    /// A replaced environment must not inherit tabs bound to its predecessor.
    pub fn sync_scope(&mut self, scope: &str, cx: &mut Context<Self>) {
        if self.scope.as_deref() == Some(scope) {
            return;
        }
        self.browsers.clear();
        // Do not reuse IDs: queued WebKit callbacks from the retired scope
        // must not update a new environment's tab with the same numeric ID.
        self.tabs.entries.clear();
        self.tabs.active = None;
        self.renaming = None;
        self.error = None;
        self.scope = Some(scope.into());
        cx.emit(SurfaceSelectionChanged);
        cx.notify();
    }
    pub fn sync_visibility(&mut self, panel_open: bool, overlay_covers_panel: bool) {
        self.visible = panel_open && !overlay_covers_panel;
        self.hide_inactive();
    }
    fn hide_all(&self) {
        #[cfg(target_os = "macos")]
        for tab in self.browsers.values() {
            if let Some(native) = &tab.native {
                native.hide();
            }
        }
    }
    fn hide_inactive(&self) {
        #[cfg(target_os = "macos")]
        for (id, tab) in &self.browsers {
            if (!self.visible || self.tabs.active != Some(*id))
                && let Some(native) = &tab.native
            {
                native.hide();
            }
        }
    }
    pub fn add(
        &mut self,
        profile: Profile,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<u64> {
        #[cfg(not(target_os = "macos"))]
        {
            let _ = (profile, window);
            self.error = Some("Embedded browsing is supported on macOS only".into());
            cx.notify();
            return None;
        }
        #[cfg(target_os = "macos")]
        {
            let id = match self.tabs.add(profile) {
                Ok(id) => id,
                Err(error) => {
                    self.error = Some(error);
                    cx.notify();
                    return None;
                }
            };
            let address = cx.new(|cx| InputState::new(window, cx).placeholder("Preview URL"));
            let subscription = cx.subscribe_in(&address, window, move |panel, _, event, _, cx| {
                if matches!(event, InputEvent::PressEnter { .. }) {
                    panel.navigate(id, cx);
                }
            });
            self.browsers.insert(
                id,
                BrowserTab {
                    address,
                    navigation: Navigation::default(),
                    pending_url: None,
                    native: None,
                    _subscription: subscription,
                },
            );
            self.error = None;
            self.hide_inactive();
            cx.emit(SurfaceSelectionChanged);
            cx.notify();
            Some(id)
        }
    }
    /// Use the saved action's `previewUrl`, captured in its accepted run owner,
    /// NOT the mutable settings form. Relay/public port previews fail explicitly.
    pub fn open_preview(
        &mut self,
        preview_url: &str,
        environment_http_url: &str,
        profile: Profile,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<u64> {
        let url = match model::preview_address(preview_url, environment_http_url) {
            Ok(url) => url,
            Err(error) => {
                self.error = Some(error);
                cx.notify();
                return None;
            }
        };
        let id = self.add(profile, window, cx)?;
        self.browsers
            .get_mut(&id)?
            .address
            .update(cx, |input, cx| input.set_value(url.as_str(), window, cx));
        self.navigate(id, cx);
        Some(id)
    }
    /// Receipt handlers have no Window borrow. Defer opening to the owning
    /// GPUI window, and fence a scope switch that races this deferred callback.
    pub fn queue_preview(
        entity: &Entity<Self>,
        scope: &str,
        preview_url: &str,
        environment_http_url: &str,
        cx: &mut gpui_kit::App,
    ) {
        let handle = entity.read(cx).window_handle;
        let entity = entity.downgrade();
        let scope = scope.to_owned();
        let preview_url = preview_url.to_owned();
        let environment_http_url = environment_http_url.to_owned();
        cx.defer(move |cx| {
            let _ = handle.update(cx, |_, window, cx| {
                let _ = entity.update(cx, |panel, cx| {
                    if panel.scope.as_deref() == Some(&scope) {
                        panel.open_preview(
                            &preview_url,
                            &environment_http_url,
                            Profile::Default,
                            window,
                            cx,
                        );
                    }
                });
            });
        });
    }

    fn navigate(&mut self, id: u64, cx: &mut Context<Self>) {
        let Some(tab) = self.browsers.get_mut(&id) else {
            return;
        };
        let url = match model::address(&tab.address.read(cx).value()) {
            Ok(url) => url,
            Err(error) => {
                tab.navigation.error = Some(error);
                cx.notify();
                return;
            }
        };
        tab.navigation.error = None;
        #[cfg(target_os = "macos")]
        if let Some(native) = &tab.native {
            if let Err(error) = native.navigate(&url) {
                tab.navigation.error = Some(error);
            }
        } else {
            tab.pending_url = Some(url);
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = url;
            tab.navigation.error = Some("Embedded browsing is supported on macOS only".into());
        }
        cx.notify();
    }
    fn close(&mut self, id: u64, cx: &mut Context<Self>) {
        self.browsers.remove(&id); // Drop stops navigation and removes the NSView.
        self.tabs.close(id);
        if self.renaming == Some(id) {
            self.renaming = None;
        }
        self.hide_inactive();
        cx.emit(SurfaceSelectionChanged);
        cx.notify();
    }
    fn activate(&mut self, id: u64, cx: &mut Context<Self>) {
        if self.tabs.activate(id) {
            self.hide_inactive();
            cx.emit(SurfaceSelectionChanged);
            cx.notify();
        }
    }
    fn finish_rename(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.renaming else {
            return;
        };
        match self.tabs.rename(id, &self.rename.read(cx).value()) {
            Ok(()) => {
                self.renaming = None;
                self.error = None;
            }
            Err(error) => self.error = Some(error),
        }
        cx.notify();
    }
}
