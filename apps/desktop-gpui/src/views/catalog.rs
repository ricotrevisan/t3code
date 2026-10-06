use gpui_kit::base::Selectable;
use gpui_kit::component::{
    ActiveTheme as _, Disableable, Sizable,
    button::{Button, ButtonVariants as _},
};
use gpui_kit::{Context, FontWeight, div, prelude::*, px};
use serde_json::{Value, json};

use crate::NativeApp;

impl NativeApp {
    pub(crate) fn render_catalog(
        &self,
        cx: &mut Context<Self>,
    ) -> Option<impl IntoElement + use<>> {
        let muted_foreground = cx.theme().muted_foreground;
        let border = cx.theme().border;
        let popover = cx.theme().popover;
        self.catalog_open.then(|| {
            div()
                .id("provider-model-catalog")
                .key_context("NativeCatalog")
                .absolute()
                .bottom(px(150.))
                .left(px(300.))
                .w(px(520.))
                .max_h(px(420.))
                .overflow_y_scroll()
                .flex()
                .flex_col()
                .gap_2()
                .p_3()
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
                                .text_sm()
                                .font_weight(FontWeight::SEMIBOLD)
                                .child("Model"),
                        )
                        .child(
                            Button::new("catalog-close")
                                .label("Close")
                                .ghost()
                                .small()
                                .on_click(cx.listener(|app, _, _, cx| {
                                    app.catalog_open = false;
                                    cx.notify();
                                })),
                        ),
                )
                .child(
                    Button::new("refresh-catalog")
                        .label("Reload server catalog")
                        .ghost()
                        .on_click(cx.listener(|app, _, _, cx| {
                            app.perform("server.getConfig", json!({}), None, cx)
                        })),
                )
                .children(self.provider_catalog.iter().map(|provider| {
                    div()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .child(div().text_color(muted_foreground).child(format!(
                            "{} · {} · {}",
                            provider.label(),
                            provider.instance_id,
                            provider.explanation()
                        )))
                        .children(provider.models.iter().enumerate().map(|(index, model)| {
                            let instance = provider.instance_id.clone();
                            let slug = model.slug.clone();
                            let enabled = provider.selectable();
                            Button::new(gpui_kit::SharedString::from(format!(
                                "catalog-{}-{index}",
                                provider.instance_id
                            )))
                            .debug_selector({
                                let id = format!("catalog-{}-{index}", provider.instance_id);
                                move || id.clone()
                            })
                            .ghost()
                            .selected(model.slug == self.model.read(cx).value())
                            .disabled(!enabled)
                            .label(model.name.clone())
                            .on_click(cx.listener(
                                move |app, _, window, cx| {
                                    app.pick_model(&instance, &slug, window, cx)
                                },
                            ))
                        }))
                }))
                .children(
                    self.provider_catalog
                        .iter()
                        .find(|provider| {
                            provider.instance_id == self.provider.read(cx).value().as_str()
                        })
                        .and_then(|provider| {
                            provider
                                .models
                                .iter()
                                .find(|model| model.slug == self.model.read(cx).value().as_str())
                        })
                        .and_then(|model| model.capabilities["optionDescriptors"].as_array())
                        .map(|options| {
                            div().flex().flex_col().gap_1().children(
                                options.iter().enumerate().map(|(index, option)| {
                                    let id = option["id"].as_str().unwrap_or_default().to_owned();
                                    let previous = self
                                        .selected_thread
                                        .as_ref()
                                        .and_then(|id| {
                                            self.shell
                                                .threads
                                                .iter()
                                                .chain(
                                                    self.pending_threads
                                                        .iter()
                                                        .map(|draft| &draft.thread),
                                                )
                                                .find(|thread| &thread.id == id)
                                        })
                                        .map(|thread| &thread.model_selection)
                                        .unwrap_or(&Value::Null);
                                    let selected = self
                                        .model_selection(previous, cx)
                                        .ok()
                                        .and_then(|selection| {
                                            selection["options"]
                                                .as_array()
                                                .and_then(|options| {
                                                    options.iter().find(|option| option["id"] == id)
                                                })
                                                .map(|option| option["value"].clone())
                                        })
                                        .or_else(|| option.get("currentValue").cloned())
                                        .or_else(|| {
                                            option["options"]
                                                .as_array()
                                                .and_then(|choices| {
                                                    choices
                                                        .iter()
                                                        .find(|choice| choice["isDefault"] == true)
                                                })
                                                .map(|choice| choice["id"].clone())
                                        });
                                    let choices = if option["type"] == "boolean" {
                                        vec![
                                            json!({"id":false,"label":"Off"}),
                                            json!({"id":true,"label":"On"}),
                                        ]
                                    } else {
                                        option["options"].as_array().cloned().unwrap_or_default()
                                    };
                                    div()
                                        .flex()
                                        .flex_col()
                                        .gap_1()
                                        .child(div().text_color(muted_foreground).child(
                                            option["label"].as_str().unwrap_or_default().to_owned(),
                                        ))
                                        .child(div().flex().flex_wrap().gap_1().children(
                                            choices.into_iter().enumerate().map(
                                                |(choice_index, choice)| {
                                                    let id = id.clone();
                                                    let value = choice["id"].clone();
                                                    Button::new(gpui_kit::SharedString::from(
                                                        format!(
                                                            "model-option-{index}-{choice_index}"
                                                        ),
                                                    ))
                                                    .debug_selector(move || {
                                                        format!(
                                                            "model-option-{index}-{choice_index}"
                                                        )
                                                    })
                                                    .ghost()
                                                    .selected(selected.as_ref() == Some(&value))
                                                    .label(format!(
                                                        "{}{}",
                                                        choice["label"]
                                                            .as_str()
                                                            .unwrap_or_default(),
                                                        if selected.as_ref() == Some(&value) {
                                                            " ✓"
                                                        } else {
                                                            ""
                                                        }
                                                    ))
                                                    .on_click(cx.listener(move |app, _, _, cx| {
                                                        app.choose_model_option(
                                                            &id,
                                                            value.clone(),
                                                            cx,
                                                        )
                                                    }))
                                                },
                                            ),
                                        ))
                                }),
                            )
                        }),
                )
        })
    }
}
