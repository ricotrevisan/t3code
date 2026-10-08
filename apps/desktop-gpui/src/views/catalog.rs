use gpui_kit::base::Selectable;
use gpui_kit::component::{
    ActiveTheme as _, Sizable,
    button::{Button, ButtonVariants as _},
    input::Input,
};
use gpui_kit::{Context, div, prelude::*, px};
use serde_json::{Value, json};

use super::list_row;
use crate::NativeApp;
use gpui_kit::base::FocusTrapElement;

impl NativeApp {
    pub(crate) fn render_catalog(
        &self,
        cx: &mut Context<Self>,
    ) -> Option<impl IntoElement + use<>> {
        let muted_foreground = cx.theme().muted_foreground;
        let border = cx.theme().border;
        let popover = cx.theme().popover;
        self.catalog_open.then(|| {
            let keyboard_selection = self
                .catalog_rows()
                .get(self.composer_state.catalog_index)
                .cloned();
            div()
                .id("provider-model-catalog")
                .debug_selector(|| "provider-model-catalog".into())
                .key_context("NativeCatalog")
                .capture_action(cx.listener(
                    |app, _: &gpui_kit::component::input::MoveUp, window, cx| {
                        app.composer_state.catalog_index = t3code_desktop_gpui::palette::step(
                            app.composer_state.catalog_index,
                            app.catalog_rows().len(),
                            false,
                        );
                        if let Some(anchor) = &app.composer_state.catalog_anchor { anchor.scroll_to(window, cx); }
                        cx.stop_propagation();
                        cx.notify();
                    },
                ))
                .capture_action(cx.listener(
                    |app, _: &gpui_kit::component::input::MoveDown, window, cx| {
                        app.composer_state.catalog_index = t3code_desktop_gpui::palette::step(
                            app.composer_state.catalog_index,
                            app.catalog_rows().len(),
                            true,
                        );
                        if let Some(anchor) = &app.composer_state.catalog_anchor { anchor.scroll_to(window, cx); }
                        cx.stop_propagation();
                        cx.notify();
                    },
                ))
                .capture_action(cx.listener(
                    |app, _: &gpui_kit::component::input::Enter, window, cx| {
                        if let Some((instance, model)) = app
                            .catalog_rows()
                            .get(app.composer_state.catalog_index)
                            .cloned()
                        {
                            app.pick_model(&instance, &model, window, cx);
                        }
                        cx.stop_propagation();
                        cx.notify();
                    },
                ))
                .capture_key_down(
                    cx.listener(|app, event: &gpui_kit::KeyDownEvent, window, cx| {
                        match event.keystroke.key.as_str() {
                            "up" | "down" => {
                                app.composer_state.catalog_index =
                                    t3code_desktop_gpui::palette::step(
                                        app.composer_state.catalog_index,
                                        app.catalog_rows().len(),
                                        event.keystroke.key == "down",
                                    );
                                if let Some(anchor) = &app.composer_state.catalog_anchor { anchor.scroll_to(window, cx); }
                                cx.stop_propagation();
                                cx.notify();
                            }
                            "enter" => {
                                if let Some((instance, model)) = app
                                    .catalog_rows()
                                    .get(app.composer_state.catalog_index)
                                    .cloned()
                                {
                                    app.pick_model(&instance, &model, window, cx);
                                }
                                cx.stop_propagation();
                            }
                            "escape" => {
                                app.close_model_catalog(window, cx);
                                cx.stop_propagation();
                                cx.notify();
                            }
                            _ => {}
                        }
                    }),
                )
                .occlude()
                .mb_1()
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
                .when_some(
                    self.composer_state.catalog_search.as_ref(),
                    |view, input| view.child(Input::new(input).small()),
                )
                .when(self.catalog_rows().is_empty(), |view| {
                    view.child(div().text_color(muted_foreground).child(
                        "No available models match. Unavailable providers are explained below.",
                    ))
                })
                .child(
                    list_row("refresh-catalog", None, "Reload server catalog", false).on_click(
                        cx.listener(|app, _, _, cx| {
                            app.perform("server.getConfig", json!({}), None, cx)
                        }),
                    ),
                )
                .when(self.unavailable_model_options(cx), |view| {
                    view.child(
                        Button::new("reset-unavailable-options")
                            .debug_selector(|| "reset-unavailable-options".into())
                            .ghost()
                            .small()
                            .label("Saved traits unavailable · reset options")
                            .on_click(cx.listener(|app, _, _, cx| app.reset_model_options(cx))),
                    )
                })
                .child(div().flex().gap_2()
                    .child(div().flex().flex_col().gap_1().w(px(128.))
                        .child(Button::new("catalog-all-providers").debug_selector(|| "catalog-all-providers".into()).ghost().small().label("All providers").selected(self.composer_state.catalog_provider.is_none()).on_click(cx.listener(|app, _, _, cx| { app.composer_state.catalog_provider = None; app.composer_state.catalog_index = 0; cx.notify(); })))
                        .children(self.provider_catalog.iter().map(|provider| {
                            let instance = provider.instance_id.clone();
                            Button::new(gpui_kit::SharedString::from(format!("catalog-provider-{instance}")))
                                .debug_selector({ let instance = instance.clone(); move || format!("catalog-provider-{instance}") })
                                .ghost().small().label(provider.label().to_owned()).tooltip(provider.explanation().to_owned())
                                .selected(self.composer_state.catalog_provider.as_ref() == Some(&instance))
                                .on_click(cx.listener(move |app, _, _, cx| { app.composer_state.catalog_provider = Some(instance.clone()); app.composer_state.catalog_index = 0; app.composer_state.catalog_scroll.set_offset(gpui_kit::point(px(0.),px(0.))); cx.notify(); }))
                        })))
                    .child(div().id("catalog-model-scroll").debug_selector(|| "catalog-model-scroll".into()).flex_1().min_w_0().max_h(px(260.)).overflow_y_scroll().track_scroll(&self.composer_state.catalog_scroll)
                .children(self.provider_catalog.iter().filter(|provider| self.composer_state.catalog_provider.as_ref().is_none_or(|id| id == &provider.instance_id)).map(|provider| {
                    div()
                        .flex()
                        .flex_col()
                        .child(
                            div()
                                .px_2()
                                .py_1()
                                .text_xs()
                                .text_color(muted_foreground)
                                .child(format!(
                                    "{} · {} · {}",
                                    provider.label(),
                                    provider.instance_id,
                                    provider.explanation()
                                )),
                        )
                        .children(
                            provider
                                .models
                                .iter()
                                .enumerate()
                                .filter(|(_, model)| {
                                    format!(
                                        "{} {} {} {}",
                                        provider.label(),
                                        provider.instance_id,
                                        model.name,
                                        model.slug
                                    )
                                    .to_lowercase()
                                    .contains(&self.composer_state.catalog_query)
                                })
                                .map(|(index, model)| {
                                    let instance = provider.instance_id.clone();
                                    let slug = model.slug.clone();
                                    let enabled = provider.selectable();
                                    div().id(gpui_kit::SharedString::from(format!("catalog-anchor-{}-{index}", provider.instance_id))).anchor_scroll((keyboard_selection.as_ref() == Some(&(instance.clone(), slug.clone()))).then(|| self.composer_state.catalog_anchor.clone()).flatten()).child(list_row(
                                        format!("catalog-{}-{index}", provider.instance_id),
                                        (keyboard_selection.as_ref()
                                            == Some(&(instance.clone(), slug.clone())))
                                        .then_some(gpui_kit::assets::IconName::ChevronRight),
                                        model.name.clone(),
                                        provider.instance_id
                                            == self.provider.read(cx).value().as_str()
                                            && model.slug == self.model.read(cx).value(),
                                    )
                                    .disabled(!enabled)
                                    .on_click(cx.listener(
                                        move |app, _, window, cx| {
                                            app.pick_model(&instance, &slug, window, cx)
                                        },
                                    )))
                                }),
                        )
                }))))
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
                                    let unavailable = selected.as_ref().is_some_and(|selected| {
                                        !choices.iter().any(|choice| &choice["id"] == selected)
                                    });
                                    div()
                                        .flex()
                                        .flex_col()
                                        .gap_1()
                                        .when(unavailable, |view| {
                                            view.child(
                                                Button::new(gpui_kit::SharedString::from(format!(
                                                    "reset-option-{index}"
                                                )))
                                                .debug_selector(move || {
                                                    format!("reset-option-{index}")
                                                })
                                                .ghost()
                                                .small()
                                                .label("Saved option unavailable · reset options")
                                                .on_click(cx.listener(|app, _, _, cx| {
                                                    app.reset_model_options(cx)
                                                })),
                                            )
                                        })
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
                                                    .label(
                                                        choice["label"]
                                                            .as_str()
                                                            .unwrap_or_default()
                                                            .to_owned(),
                                                    )
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
                .focus_trap("catalog-focus-trap", &self.settings_shortcut_focus)
        })
    }
}
