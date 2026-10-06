use gpui_kit::component::{
    ActiveTheme as _, Sizable,
    button::{Button, ButtonVariants as _},
    input::Input,
};
use gpui_kit::{Context, FontWeight, div, prelude::*, px};
use serde_json::json;

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
        self.settings_open.then(|| {
            div()
                .id("settings")
                .key_context("NativeSettings")
                .absolute()
                .top(px(72.))
                .left(px(320.))
                .w(px(640.))
                .max_h(px(620.))
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
                                .on_click(cx.listener(|app, _, _, cx| {
                                    app.settings_open = false;
                                    cx.notify();
                                })),
                        ),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_2()
                        .child(section("CONNECTION"))
                        .child(Input::new(&self.server))
                        .child(
                            div()
                                .flex()
                                .flex_wrap()
                                .gap_2()
                                .child(
                                    Button::new("settings-pair")
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
                                        .on_click(cx.listener(|app, _, _, cx| app.connect(cx))),
                                )
                                .child(
                                    Button::new("settings-disconnect")
                                        .label("Disconnect")
                                        .ghost()
                                        .on_click(cx.listener(|app, _, _, cx| app.disconnect(cx))),
                                ),
                        ),
                )
                .child(
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
                                .child(
                                    Button::new(gpui_kit::SharedString::from(format!(
                                        "provider-signin-{instance_id}"
                                    )))
                                    .debug_selector({
                                        let id = instance_id.clone();
                                        move || format!("provider-signin-{id}")
                                    })
                                    .label("Sign in")
                                    .ghost()
                                    .small()
                                    .on_click(cx.listener(move |app, _, _, cx| {
                                        app.start_provider_auth(&instance_id, cx)
                                    })),
                                )
                        }))
                        .child(
                            Button::new("settings-refresh-providers")
                                .label("Reload providers")
                                .secondary()
                                .on_click(cx.listener(|app, _, _, cx| {
                                    app.perform("server.getConfig", json!({}), None, cx)
                                })),
                        )
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
                                        state["authorizationUrl"].as_str().map(str::to_owned),
                                        |view, url| {
                                            view.child(
                                                Button::new("provider-auth-open")
                                                    .label("Open authorization page")
                                                    .secondary()
                                                    .on_click(cx.listener(move |_, _, _, cx| {
                                                        cx.open_url(&url)
                                                    })),
                                            )
                                        },
                                    )
                                    .child(Input::new(&self.auth_callback))
                                    .child(
                                        Button::new("provider-auth-complete")
                                            .debug_selector(|| "provider-auth-complete".into())
                                            .label("Complete sign-in")
                                            .primary()
                                            .on_click(cx.listener(|app, _, _, cx| {
                                                app.complete_provider_auth(cx)
                                            })),
                                    ),
                            )
                        }),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .child(section("APPEARANCE"))
                        .child(div().text_sm().child("One Dark (Zed)"))
                        .child(div().text_xs().text_color(muted_foreground).child(
                            "The native client ships Zed's One Dark palette with semantic tokens.",
                        )),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .child(section("KEYBINDINGS"))
                        .children(
                            [
                                ("⌘K", "Command palette"),
                                ("⌘N", "New thread"),
                                ("↩", "Send prompt (⇧↩ for a newline)"),
                                ("⌘⇧[ / ⌘⇧]", "Previous / next thread"),
                                ("⌘⌥B", "Toggle right panel"),
                                ("⌘,", "Settings"),
                                ("⌘P", "Workspace files"),
                                ("Esc", "Close the open menu or panel"),
                                ("⌘S", "Stash prompt"),
                                ("⌘⇧F", "Search workspace contents"),
                                ("⌘⇧S", "Toggle settlement"),
                                ("⌘⇧P", "Toggle pin"),
                            ]
                            .into_iter()
                            .map(|(keys, label)| {
                                div()
                                    .flex()
                                    .justify_between()
                                    .text_sm()
                                    .child(label)
                                    .child(div().text_color(muted_foreground).child(keys))
                            }),
                        ),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .child(section("ABOUT"))
                        .child(div().text_sm().child("T3 Code — desktop GPUI client"))
                        .child(div().text_xs().text_color(muted_foreground).child(
                            "Experimental native slice; feature parity is still being evaluated.",
                        )),
                )
        })
    }
}
