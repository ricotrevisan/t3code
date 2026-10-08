use super::{BrowserPanel, model::Profile};
use gpui_kit::base::{Disableable as _, Selectable};
use gpui_kit::component::{
    ActiveTheme as _, Sizable,
    button::{Button, ButtonVariants as _},
    input::Input,
};
use gpui_kit::{
    AnyElement, Context, Focusable as _, IntoElement, Render, Window, canvas, div, prelude::*,
};

impl BrowserPanel {
    /// Insert this beside the built-in tab controls; those controls call
    /// `deactivate` before selecting Inspector/Changes/Agents.
    pub fn tab_strip(&self, cx: &mut Context<Self>) -> AnyElement {
        let mut strip = div()
            .id("browser-surface-tabs")
            .min_w_0()
            .flex()
            .gap_1()
            .overflow_x_scroll();
        for tab in &self.tabs.entries {
            let id = tab.id;
            strip = strip.child(
                div()
                    .flex()
                    .gap_1()
                    .child(
                        Button::new(("browser-tab", id))
                            .debug_selector(move || format!("browser-tab-{id}"))
                            .label(tab.label.clone())
                            .tooltip(if tab.profile == Profile::Incognito {
                                "Incognito · data discarded when closed"
                            } else {
                                "Browser · default profile"
                            })
                            .ghost()
                            .small()
                            .selected(self.tabs.active == Some(id))
                            .on_click(cx.listener(move |panel, _, _, cx| panel.activate(id, cx))),
                    )
                    .child(
                        Button::new(("browser-close", id))
                            .debug_selector(move || format!("browser-close-{id}"))
                            .label("×")
                            .tooltip("Close browser tab")
                            .ghost()
                            .small()
                            .on_click(cx.listener(move |panel, _, _, cx| panel.close(id, cx))),
                    ),
            );
        }
        strip = strip
            .child(
                Button::new("browser-add")
                    .debug_selector(|| "browser-add".into())
                    .label("+ Browser")
                    .ghost()
                    .small()
                    .on_click(cx.listener(|panel, _, window, cx| {
                        panel.add(Profile::Default, window, cx);
                    })),
            )
            .child(
                Button::new("browser-add-incognito")
                    .debug_selector(|| "browser-add-incognito".into())
                    .label("+ Incognito")
                    .ghost()
                    .small()
                    .on_click(cx.listener(|panel, _, window, cx| {
                        panel.add(Profile::Incognito, window, cx);
                    })),
            );
        strip.into_any_element()
    }

    pub fn browser_body(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        self.hide_inactive();
        let id = self.tabs.active?;
        let tab = self.browsers.get(&id)?;
        let address = tab.address.clone();
        let navigation = tab.navigation.clone();
        if !navigation.url.is_empty()
            && !address.read(cx).focus_handle(cx).is_focused(window)
            && address.read(cx).value().as_ref() != navigation.url
        {
            address.update(cx, |input, cx| {
                input.set_value(navigation.url.clone(), window, cx)
            });
        }
        let position = self.tabs.entries.iter().position(|tab| tab.id == id)?;
        let label = self.tabs.entries[position].label.clone();
        let controls = div()
            .flex()
            .gap_1()
            .child(
                Button::new("browser-back")
                    .debug_selector(|| "browser-back".into())
                    .label("Back")
                    .ghost()
                    .small()
                    .disabled(!navigation.can_go_back)
                    .on_click(cx.listener(move |panel, _, _, _| {
                        #[cfg(target_os = "macos")]
                        if let Some(native) =
                            panel.browsers.get(&id).and_then(|tab| tab.native.as_ref())
                        {
                            native.back();
                        }
                    })),
            )
            .child(
                Button::new("browser-forward")
                    .label("Forward")
                    .ghost()
                    .small()
                    .disabled(!navigation.can_go_forward)
                    .on_click(cx.listener(move |panel, _, _, _| {
                        #[cfg(target_os = "macos")]
                        if let Some(native) =
                            panel.browsers.get(&id).and_then(|tab| tab.native.as_ref())
                        {
                            native.forward();
                        }
                    })),
            )
            .child(
                Button::new("browser-reload")
                    .label("Reload")
                    .ghost()
                    .small()
                    .disabled(navigation.url.is_empty())
                    .on_click(cx.listener(move |panel, _, _, _| {
                        #[cfg(target_os = "macos")]
                        if let Some(native) =
                            panel.browsers.get(&id).and_then(|tab| tab.native.as_ref())
                        {
                            native.reload();
                        }
                    })),
            )
            .child(
                Button::new("browser-external")
                    .label("External")
                    .tooltip("Open current page in the system browser")
                    .ghost()
                    .small()
                    .disabled(super::model::address(&navigation.url).is_err())
                    .on_click(cx.listener(move |panel, _, _, cx| {
                        if let Some(tab) = panel.browsers.get(&id)
                            && let Ok(url) = super::model::address(&tab.navigation.url)
                        {
                            cx.open_url(url.as_str());
                        }
                    })),
            );
        let management = div()
            .flex()
            .gap_1()
            .child(
                Button::new("browser-rename")
                    .debug_selector(|| "browser-rename".into())
                    .label("Rename…")
                    .ghost()
                    .small()
                    .on_click(cx.listener(move |panel, _, window, cx| {
                        panel.renaming = Some(id);
                        panel.rename.update(cx, |input, cx| {
                            input.set_value(label.clone(), window, cx);
                            input.focus(window, cx);
                        });
                        cx.notify();
                    })),
            )
            .child(
                Button::new("browser-move-left")
                    .debug_selector(|| "browser-move-left".into())
                    .label("←")
                    .tooltip("Move tab left")
                    .ghost()
                    .small()
                    .disabled(position == 0)
                    .on_click(cx.listener(move |panel, _, _, cx| {
                        panel.tabs.move_by(id, -1);
                        cx.notify();
                    })),
            )
            .child(
                Button::new("browser-move-right")
                    .debug_selector(|| "browser-move-right".into())
                    .label("→")
                    .tooltip("Move tab right")
                    .ghost()
                    .small()
                    .disabled(position + 1 == self.tabs.entries.len())
                    .on_click(cx.listener(move |panel, _, _, cx| {
                        panel.tabs.move_by(id, 1);
                        cx.notify();
                    })),
            )
            .when(navigation.loading, |view| {
                view.child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child("Loading"),
                )
            });
        let mut body = div()
            .id("browser-surface-body")
            .debug_selector(|| "browser-surface-body".into())
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .min_w_0()
            .gap_1()
            .child(controls)
            .child(
                div()
                    .flex()
                    .gap_1()
                    .child(div().min_w_0().flex_1().child(Input::new(&address).small()))
                    .child(
                        Button::new("browser-go")
                            .debug_selector(|| "browser-go".into())
                            .label("Go")
                            .primary()
                            .small()
                            .on_click(cx.listener(move |panel, _, _, cx| panel.navigate(id, cx))),
                    ),
            )
            .child(management);
        if self.renaming == Some(id) {
            body = body.child(
                div()
                    .flex()
                    .gap_1()
                    .child(
                        div()
                            .min_w_0()
                            .flex_1()
                            .child(Input::new(&self.rename).small()),
                    )
                    .child(
                        Button::new("browser-rename-save")
                            .label("Save")
                            .ghost()
                            .small()
                            .on_click(cx.listener(|panel, _, _, cx| panel.finish_rename(cx))),
                    )
                    .child(
                        Button::new("browser-rename-cancel")
                            .label("Cancel")
                            .ghost()
                            .small()
                            .on_click(cx.listener(|panel, _, _, cx| {
                                panel.renaming = None;
                                cx.notify();
                            })),
                    ),
            );
        }
        body = body.children(
            navigation
                .error
                .map(|error| div().text_xs().text_color(cx.theme().danger).child(error)),
        );
        if self.visible {
            #[cfg(target_os = "macos")]
            {
                let panel = cx.entity().downgrade();
                body = body.child(
                    canvas(
                        |bounds, window, _| bounds.intersect(&window.content_mask().bounds),
                        move |bounds, mask, window, cx| {
                            let async_cx = cx.to_async();
                            let weak = panel.clone();
                            let _ = panel.update(cx, |panel, cx| {
                                // Stale paint closures cannot remount a closed/inactive tab.
                                if !panel.visible || panel.tabs.active != Some(id) {
                                    return;
                                }
                                let Some(profile) = panel
                                    .tabs
                                    .entries
                                    .iter()
                                    .find(|tab| tab.id == id)
                                    .map(|tab| tab.profile)
                                else {
                                    return;
                                };
                                let Some(tab) = panel.browsers.get_mut(&id) else {
                                    return;
                                };
                                if tab.native.is_none() {
                                    let callback = move |navigation: super::model::Navigation| {
                                        let weak = weak.clone();
                                        // Queue callbacks: loadRequest may invoke a delegate
                                        // synchronously while GPUI already borrows this entity.
                                        async_cx
                                            .spawn(async move |cx| {
                                                let _ = weak.update(cx, |panel, cx| {
                                                    if let Some(tab) = panel.browsers.get_mut(&id) {
                                                        panel.tabs.navigation_title(
                                                            id,
                                                            &navigation.title,
                                                        );
                                                        tab.navigation = navigation;
                                                        cx.notify();
                                                    }
                                                });
                                            })
                                            .detach();
                                    };
                                    match super::native::EmbeddedBrowser::attach(
                                        window, profile, callback,
                                    ) {
                                        Ok(native) => tab.native = Some(native),
                                        Err(error) => {
                                            if tab.navigation.error.as_ref() != Some(&error) {
                                                tab.navigation.error = Some(error);
                                                cx.notify();
                                            }
                                            return;
                                        }
                                    }
                                }
                                if let Some(native) = &tab.native {
                                    if let Some(url) = tab.pending_url.take()
                                        && let Err(error) = native.navigate(&url)
                                    {
                                        tab.navigation.error = Some(error);
                                        cx.notify();
                                    }
                                    native.layout(bounds, mask);
                                }
                            });
                        },
                    )
                    .flex_1()
                    .min_h_0()
                    .w_full(),
                );
            }
        }
        Some(body.into_any_element())
    }
}

impl Render for BrowserPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .flex_col()
            .min_w_0()
            .gap_1()
            .when(self.browser_active(), |view| view.flex_1().min_h_0())
            .child(self.tab_strip(cx))
            .children(
                self.error
                    .clone()
                    .map(|error| div().text_xs().text_color(cx.theme().danger).child(error)),
            )
            .children(self.browser_body(window, cx))
    }
}
