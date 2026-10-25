use gpui_kit::base::FocusTrapElement;
use gpui_kit::component::{
    ActiveTheme as _, Disableable, Sizable,
    button::{Button, ButtonVariants as _},
    input::Input,
    radio::{Radio, RadioGroup},
    switch::Switch,
};
use gpui_kit::{Context, FontWeight, div, prelude::*, rems};
use t3code_desktop_gpui::{credentials, notification_platform, notifications};

use crate::NativeApp;

impl NativeApp {
    pub(crate) fn render_settings(
        &self,
        cx: &mut Context<Self>,
    ) -> Option<impl IntoElement + use<>> {
        let muted_foreground = cx.theme().muted_foreground;
        let border = cx.theme().border;
        let group_box = cx.theme().group_box;
        let green = cx.theme().green;
        let popover = cx.theme().popover;
        let section = |label: &'static str| {
            div()
                .text_xs()
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(muted_foreground)
                .child(label)
        };
        let search = self.settings_query.read(cx).value().to_lowercase();
        let matches =
            |terms: &str| search.is_empty() || terms.to_lowercase().contains(search.trim());
        self.settings_open.then(|| {
            div()
                .id("settings")
                .debug_selector(|| "settings".into())
                .key_context("NativeSettings")
                .track_focus(&self.settings_shortcut_focus)
                .occlude()
                .w(rems(640. / 14.))
                .max_w_full()
                .max_h(gpui_kit::relative(0.85))
                .overflow_y_scroll()
                .flex()
                .flex_col()
                .gap_3()
                .p_4()
                .rounded_lg()
                .bg(popover)
                .border_1()
                .border_color(border)
                .shadow_lg()
                .child(div().debug_selector(|| "settings-search".into()).child(Input::new(&self.settings_query)))
                .child(
                    div()
                        .flex()
                        .justify_between()
                        .items_center()
                        .child(
                            div()
                                .text_lg()
                                .font_weight(FontWeight::SEMIBOLD)
                                .child("Settings"),
                        )
                        .child(
                            Button::new("settings-close")
                                .debug_selector(|| "settings-close".into())
                                .label("Close")
                                .ghost()
                                .small()
                                .on_click(cx.listener(|app, _, window, cx| app.close_settings(window, cx))),
                        ),
                )
                .when(matches("connection pairing credentials follow up queue general send key confirmations archive delete unpin timestamp skills slash"), |view| view.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_2()
                        .child(section("GENERAL"))
                        .child(self.render_follow_up_settings(cx))
                        .child(self.render_general_preferences(cx))
                        .child(section("CONNECTION"))
                        .child(div().text_xs().child(self.connection_state_text()))
                        .child(Input::new(&self.server))
                        .child(self.render_saved_environments(cx))
                        .child(
                            div()
                                .debug_selector(|| "pairing-input".into())
                                .child(Input::new(&self.pairing)),
                        )
                        .child(
                            div()
                                .flex()
                                .flex_wrap()
                                .gap_2()
                                .child(
                                    Button::new("pair")
                                        .debug_selector(|| "pair".into())
                                        .label("Pair and connect")
                                        .primary()
                                        .on_click(
                                            cx.listener(|app, _, window, cx| app.pair(window, cx)),
                                        ),
                                )
                                .child(
                                    Button::new("settings-connect")
                                        .label("Connect / retry")
                                        .secondary()
                                        .on_click(cx.listener(|app, _, _, cx| app.retry_connection_now(cx))),
                                )
                                .child(
                                    Button::new("settings-disconnect")
                                        .label("Disconnect")
                                        .ghost()
                                        .on_click(cx.listener(|app, _, _, cx| app.disconnect(cx))),
                                ),
                        )
                        .child(
                            div()
                                .flex()
                                .flex_wrap()
                                .gap_2()
                                .child(
                                    Button::new("remember-credential")
                                        .debug_selector(|| "remember-credential".into())
                                        .label("Remember in Keychain")
                                        .ghost()
                                        .on_click(
                                            cx.listener(|app, _, _, cx| {
                                                app.remember_credential(cx)
                                            }),
                                        ),
                                )
                                .child(
                                    Button::new("load-credential")
                                        .debug_selector(|| "load-credential".into())
                                        .label("Connect remembered")
                                        .ghost()
                                        .on_click(cx.listener(|app, _, _, cx| {
                                            app.credential_operation(
                                                credentials::Operation::Load,
                                                cx,
                                            )
                                        })),
                                )
                                .child(
                                    Button::new("forget-credential")
                                        .debug_selector(|| "forget-credential".into())
                                        .label("Forget credential")
                                        .ghost()
                                        .on_click(
                                            cx.listener(|app, _, _, cx| app.forget_credential(cx)),
                                        ),
                                ),
                        ),
                ))
                .when(matches("providers instances models authentication version capabilities binary executable API endpoint transport credentials configuration sign-in"), |view| view.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .child(section("PROVIDERS"))
                        .when(self.provider_catalog.is_empty(), |view| {
                            view.child(
                                div()
                                    .text_xs()
                                    .text_color(muted_foreground)
                                    .child("No provider catalog loaded"),
                            )
                        })
                        .children(self.provider_catalog.iter().map(|provider| {
                            let instance_id = provider.instance_id.clone();
                            let details = self.settings_provider_details.iter().find(|value| value["__nativeGeneration"].as_u64() == Some(self.generation) && value["instanceId"].as_str() == Some(&instance_id));
                            div()
                                .flex()
                                .flex_col()
                                .gap_1()
                                .child(
                                    div()
                                        .flex()
                                        .justify_between()
                                        .gap_2()
                                        .text_sm()
                                        .child(provider.label().to_owned())
                                        .child(
                                            div()
                                                .text_color(if provider.selectable() {
                                                    green
                                                } else {
                                                    muted_foreground
                                                })
                                                .child(provider.explanation().to_owned()),
                                        ),
                                )
                                .when_some(details, |view, details| view.child(div().text_xs().text_color(muted_foreground).child(crate::settings_provider::details(details))))
                                .when_some(details.filter(|details| details["__nativeConfiguration"].is_object()), |view, _| {
                                    let id = instance_id.clone();
                                    let enabled = provider.enabled;
                                    view.child(Switch::new(gpui_kit::SharedString::from(format!("provider-enabled-{id}")))
                                        .label("Enabled on this environment").checked(enabled).small()
                                        .disabled(self.settings_provider_busy == Some(self.generation) || !self.connection_ready())
                                        .on_click(cx.listener(move |app, enabled: &bool, _, cx| app.set_provider_enabled(&id, *enabled, cx))))
                                })
                                .when_some(details.filter(|details| details["__nativeConfiguration"].is_object()), |view, _| {
                                    let edit_id = instance_id.clone();
                                    let copy_id = instance_id.clone();
                                    view.child(div().flex().flex_wrap().gap_2()
                                        .child(Button::new(gpui_kit::SharedString::from(format!("provider-edit-{edit_id}")))
                                            .debug_selector({ let id = edit_id.clone(); move || format!("provider-edit-{id}") })
                                            .label("Edit instance").ghost().small()
                                            .disabled(self.provider_instance_editor.is_open() || self.settings_provider_busy == Some(self.generation) || !self.connection_ready())
                                            .on_click(cx.listener(move |app, _, window, cx| app.open_provider_instance(&edit_id, false, window, cx))))
                                        .child(Button::new(gpui_kit::SharedString::from(format!("provider-copy-{copy_id}")))
                                            .debug_selector({ let id = copy_id.clone(); move || format!("provider-copy-{id}") })
                                            .label("Add similar instance").ghost().small()
                                            .disabled(self.provider_instance_editor.is_open() || self.settings_provider_busy == Some(self.generation) || !self.connection_ready())
                                            .on_click(cx.listener(move |app, _, window, cx| app.open_provider_instance(&copy_id, true, window, cx)))))
                                })
                                .when(provider.enabled && provider.installed && provider.status != "ready" && details.is_some_and(crate::settings_provider::can_sign_in)
                                    && self.auth_state.as_ref().is_none_or(|state| state["instanceId"] != instance_id || !crate::settings_provider::auth_active(state)), |view| view.child(
                                    Button::new(gpui_kit::SharedString::from(format!(
                                        "provider-signin-{instance_id}"
                                    )))
                                    .debug_selector({
                                        let id = instance_id.clone();
                                        move || format!("provider-signin-{id}")
                                    })
                                    .label("Sign in")
                                    .disabled(self.settings_provider_busy == Some(self.generation) || !self.connection_ready())
                                    .ghost()
                                    .small()
                                    .on_click(cx.listener(move |app, _, _, cx| {
                                        app.start_provider_auth(&instance_id, cx)
                                    })),
                                ))
                        }))
                        .child(
                            Button::new("settings-refresh-providers")
                                .label("Reload providers")
                                .disabled((self.settings_provider_busy == Some(self.generation)) || !self.connection_ready())
                                .secondary()
                                .on_click(cx.listener(|app, _, _, cx| {
                                    app.refresh_settings_providers(true, cx)
                                })),
                        )
                        .child(self.render_provider_instance_editor(cx))
                        .when_some(self.auth_state.clone(), |view, state| {
                            view.child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .gap_2()
                                    .p_2()
                                    .rounded_md()
                                    .bg(group_box)
                                    .border_1()
                                    .border_color(border)
                                    .child(section("SIGN IN"))
                                    .child(div().text_sm().child(format!(
                                        "{} · {}",
                                        state["phase"].as_str().unwrap_or("idle"),
                                        state["message"].as_str().unwrap_or("")
                                    )))
                                    .when_some(
                                        crate::settings_provider::browser_url(&state).map(str::to_owned),
                                        |view, url| {
                                            view.child(
                                                Button::new("provider-auth-open")
                                                    .debug_selector(|| "provider-auth-open".into())
                                                    .label("Open authorization page")
                                                    .secondary()
                                                    .on_click(cx.listener(move |_, _, _, cx| {
                                                        cx.open_url(&url)
                                                    })),
                                            )
                                        },
                                    )
                                    .when(state["interaction"]["type"] == "deviceCode", |view| {
                                        let code = state["interaction"]["userCode"].as_str().unwrap_or_default().to_owned();
                                        view.child(div().text_sm().child(format!("Verification code: {code}")))
                                            .child(Button::new("provider-auth-copy-code").debug_selector(|| "provider-auth-copy-code".into())
                                                .label("Copy verification code").secondary().on_click(cx.listener(move |_, _, _, cx| {
                                                    cx.write_to_clipboard(gpui_kit::ClipboardItem::new_string(code.clone()));
                                                })))
                                    })
                                    .when(state["interaction"]["type"] == "terminal", |view| view.child(div().text_sm().child(state["interaction"]["output"].as_str().unwrap_or_default().to_owned())))
                                    .child(self.render_provider_interaction(&state, cx))
                                    .when(crate::settings_provider::auth_active(&state), |view| view.child(
                                        Button::new("provider-auth-cancel").label("Cancel sign-in").ghost().disabled(!self.connection_ready())
                                            .on_click(cx.listener(|app, _, _, cx| app.cancel_provider_auth(cx)))))
                                    .when(crate::settings_provider::accepts_callback(&state), |view| view.child(Input::new(&self.auth_callback))
                                    .child(
                                        Button::new("provider-auth-complete").disabled(!self.connection_ready())
                                            .debug_selector(|| "provider-auth-complete".into())
                                            .label("Complete sign-in")
                                            .primary()
                                            .on_click(cx.listener(|app, _, _, cx| {
                                                app.complete_provider_auth(cx)
                                            })),
                                    )),
                            )
                        }),
                ))
                .when(matches("usage limits quotas credits subscription providers"), |view| view.child(
                    t3code_desktop_gpui::surfaces::usage::render(
                        self.server_config.config(self.generation).unwrap_or(&serde_json::Value::Null), cx)))
                .when(matches("theme library themes import export"), |view| view.child(self.render_theme_library(cx)))
                .when(matches("font families typography prose code"), |view| view.child(self.render_font_families(cx)))
                .when(matches("notifications appearance light dark system font typography context meter theme contrast prose code"), |view| view.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .child(section("NOTIFICATIONS"))
                        .child(div().debug_selector(|| "settings-notification-status".into()).text_xs().text_color(muted_foreground).child(self.notification_state_text()))
                        .child(RadioGroup::vertical("settings-notification-mode")
                            .selected_index(notifications::NotificationMode::ALL.iter().position(|mode| *mode == self.notification_preferences.mode))
                            .disabled(!notification_platform::Platform::supported())
                            .children(notifications::NotificationMode::ALL.into_iter().enumerate().map(|(index, mode)| Radio::new(("notification-mode", index)).label(mode.label())))
                            .on_click(cx.listener(|app, index: &usize, _, cx| {
                                let mut preferences = app.notification_preferences;
                                preferences.mode = notifications::NotificationMode::ALL[*index];
                                app.set_notification_preferences(preferences, cx);
                            })))
                        .child(Switch::new("settings-in-app-notifications")
                            .label("In-app notifications for other threads")
                            .checked(self.notification_preferences.in_app)
                            .small()
                            .on_click(cx.listener(|app, enabled: &bool, _, cx| {
                                let mut preferences = app.notification_preferences;
                                preferences.in_app = *enabled;
                                app.set_notification_preferences(preferences, cx);
                            })))
                        .child(div().text_xs().text_color(muted_foreground).child(
                            if notification_platform::Platform::supported() {
                                "Device-local preferences. Desktop alerts require the bundled app and macOS notification permission (System Settings → Notifications). Sounds use distinct system tones."
                            } else {
                                "Device-local preferences. In-app notifications remain available for other threads."
                            }))
                        .child(section("APPEARANCE"))
                        .child(self.render_typography_preferences(cx))
                        .child(RadioGroup::horizontal("settings-appearance")
                            .selected_index(crate::settings_appearance::Appearance::ALL.iter().position(|mode| *mode == crate::settings_appearance::current(cx).appearance))
                            .children(crate::settings_appearance::Appearance::ALL.into_iter().enumerate().map(|(index, mode)| Radio::new(("appearance-mode", index)).label(mode.label())))
                            .on_click(cx.listener(|app, index: &usize, window, cx| {
                                if let Err(error) = crate::theme::update(window, cx, |preferences| preferences.appearance = crate::settings_appearance::Appearance::ALL[*index]) {
                                    app.report_error(format!("Could not save appearance: {error}"), cx);
                                }
                                cx.notify();
                            })))
                        .child(div().text_xs().text_color(muted_foreground).child("Zed One Light / One Dark · device-local"))
                        .child(div().flex().items_center().gap_2()
                            .child(div().text_sm().child(format!("UI font: {} px", crate::settings_appearance::current(cx).font_size)))
                            .children([(-1., "Smaller"), (1., "Larger")].into_iter().enumerate().map(|(index, (delta, label))| {
                                Button::new(("settings-font", index)).label(label).ghost().small()
                                    .on_click(cx.listener(move |app, _, window, cx| {
                                        if let Err(error) = crate::theme::update(window, cx, |preferences| preferences.font_size += delta) {
                                            app.report_error(format!("Could not save appearance: {error}"), cx);
                                        }
                                        cx.notify();
                                    }))
                            }))
                            .child(Button::new("settings-zoom-reset").label("Reset zoom").ghost().small()
                                .on_click(cx.listener(|app, _, window, cx| app.change_scale(None, window, cx)))))
                        .child(
                            div()
                                .debug_selector(|| "settings-context-meter".into())
                                .pt_2()
                                .child(
                                    Switch::new("settings-context-meter")
                                        .label("Show context window meter in the composer")
                                        .checked(!self.context_meter_hidden)
                                        .small()
                                        .on_click(cx.listener(|app, shown: &bool, _, cx| {
                                            app.context_meter_hidden = !shown;
                                            cx.notify();
                                        })),
                                ),
                        ),
                ))
                .when(matches("keybindings keyboard shortcuts"), |view| view.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .child(section("KEYBINDINGS"))
                        .child(self.render_keybinding_preferences(cx))
                        .children(cx.key_bindings().borrow().bindings().filter(|binding| {
                            binding.predicate().is_some_and(|predicate| predicate.to_string().contains("Native"))
                        }).map(|binding| {
                            let keys = binding.keystrokes().iter().map(|stroke| stroke.to_string()).collect::<Vec<_>>().join(" ");
                            let label = binding.action().name().to_owned();
                            let context = binding.predicate().map(|predicate| predicate.to_string()).unwrap_or_default();
                            div().flex().flex_col().gap_1()
                                .child(div().flex().justify_between().text_sm().child(label).child(div().text_color(muted_foreground).child(keys)))
                                .child(div().text_xs().text_color(muted_foreground).child(context))
                        }))
                ))
                .when(matches("project defaults storage scopes environment preferences"), |view| view.child(self.render_scoped_pages(false, cx)))
                .when(matches("diagnostics processes traces logs"), |view| view.child(self.render_scoped_pages(true, cx)))
                .when(matches("licenses"), |view| view.child(div().flex().flex_col().gap_1()
                    .child(section("LICENSES"))
                    .child(div().text_sm().child(include_str!("../../../../LICENSE")))
                    .child("Third-party notices are not bundled in this experimental client.")))
                .when(matches("about version"), |view| view.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .child(section("ABOUT"))
                        .child(div().text_sm().child(concat!("T3 Code — desktop GPUI client ", env!("CARGO_PKG_VERSION"))))
                        .child(div().text_xs().child(format!("Platform: {} / {}", std::env::consts::OS, std::env::consts::ARCH)))
                        .when_some(self.server_config.config(self.generation), |view, config| view.child(format!("Environment: {} · server {}", config["environment"]["label"], config["environment"]["serverVersion"])))
                        .child(div().text_xs().text_color(muted_foreground).child(
                            "Experimental native slice; feature parity is still being evaluated.",
                        )),
                ))
                .focus_trap("settings", &self.settings_shortcut_focus)
        })
    }
}
