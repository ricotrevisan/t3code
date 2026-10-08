//! Model and mode selection, and provider sign-in.

use crate::NativeApp;
use gpui_kit::{App, AppContext as _, Context, Window};
use serde_json::{Value, json};
use t3code_desktop_gpui::commands;

impl NativeApp {
    pub(crate) fn open_model_catalog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.catalog_open {
            self.close_model_catalog(window, cx);
            return;
        }
        self.prepare_search_overlay(window, cx);
        self.catalog_open = true;
        self.composer_state.catalog_favorites_only = false;
        self.composer_state.catalog_show_hidden = false;
        if self.catalog_open {
            self.composer_state.catalog_provider = None;
            self.composer_state.catalog_anchor = Some(gpui_kit::ScrollAnchor::for_handle(
                self.composer_state.catalog_scroll.clone(),
            ));
            if self.composer_state.catalog_search.is_none() {
                let input = cx.new(|cx| {
                    crate::single_line_input(window, cx)
                        .placeholder("Search models or provider instances…")
                });
                let subscription = cx.subscribe(&input, |app, input, event, cx| {
                    if matches!(event, gpui_kit::component::input::InputEvent::Change) {
                        app.composer_state.catalog_query = input.read(cx).value().to_lowercase();
                        app.composer_state.catalog_index = 0;
                        cx.notify();
                    }
                });
                self.composer_state.catalog_search = Some(input);
                self.composer_state.catalog_subscription = Some(subscription);
            }
            let input = self.composer_state.catalog_search.clone().unwrap();
            input.update(cx, |input, cx| {
                input.set_value("", window, cx);
                input.focus(window, cx);
            });
        }
        cx.notify();
    }

    pub(crate) fn close_model_catalog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.catalog_open = false;
        if let Some(focus) = self.previous_focus.take() {
            window.focus(&focus, cx);
        }
        cx.notify();
    }

    pub(crate) fn model_preferences(&self) -> Vec<t3code_desktop_gpui::drafts::ModelPreference> {
        self.draft_store
            .as_ref()
            .map(|store| store.model_preferences(&self.environment_scope))
            .unwrap_or_else(|| {
                self.composer_state
                    .model_preferences
                    .get(&self.environment_scope)
                    .cloned()
                    .unwrap_or_default()
            })
    }

    pub(crate) fn model_preference(
        &self,
        instance: &str,
        slug: &str,
    ) -> Option<t3code_desktop_gpui::drafts::ModelPreference> {
        self.model_preferences()
            .into_iter()
            .find(|entry| entry.instance_id == instance && entry.model == slug)
    }

    pub(crate) fn toggle_model_preference(
        &mut self,
        instance: &str,
        slug: &str,
        favorite: bool,
        cx: &mut Context<Self>,
    ) {
        let mut preferences = self.model_preferences().to_vec();
        let index = preferences
            .iter()
            .position(|entry| entry.instance_id == instance && entry.model == slug)
            .unwrap_or_else(|| {
                preferences.push(t3code_desktop_gpui::drafts::ModelPreference {
                    instance_id: instance.into(),
                    model: slug.into(),
                    ..Default::default()
                });
                preferences.len() - 1
            });
        if favorite {
            preferences[index].favorite = !preferences[index].favorite;
        } else {
            preferences[index].hidden = !preferences[index].hidden;
        }
        preferences.retain(|entry| entry.favorite || entry.hidden);
        if let Some(store) = &mut self.draft_store {
            store.set_model_preferences(&self.environment_scope, preferences);
            self.schedule_draft_save(cx);
        } else {
            self.composer_state
                .model_preferences
                .insert(self.environment_scope.clone(), preferences);
        }
        self.composer_state.catalog_index = 0;
        cx.notify();
    }

    /// The painted list and keyboard list share filtering and favorite ordering.
    pub(crate) fn catalog_models<'a>(
        &self,
        provider: &'a t3code_desktop_gpui::providers::Provider,
    ) -> Vec<(usize, &'a t3code_desktop_gpui::providers::Model)> {
        let mut models: Vec<_> = provider
            .models
            .iter()
            .enumerate()
            .filter(|(_, model)| {
                let preference = self.model_preference(&provider.instance_id, &model.slug);
                (!preference.as_ref().is_some_and(|entry| entry.hidden)
                    || self.composer_state.catalog_show_hidden)
                    && (!self.composer_state.catalog_favorites_only
                        || preference.as_ref().is_some_and(|entry| entry.favorite))
                    && format!(
                        "{} {} {} {}",
                        provider.label(),
                        provider.instance_id,
                        model.name,
                        model.slug
                    )
                    .to_lowercase()
                    .contains(&self.composer_state.catalog_query)
            })
            .collect();
        models.sort_by_key(|(_, model)| {
            !self
                .model_preference(&provider.instance_id, &model.slug)
                .is_some_and(|entry| entry.favorite)
        });
        models
    }

    pub(crate) fn catalog_rows(&self) -> Vec<(String, String)> {
        self.provider_catalog
            .iter()
            .filter(|provider| {
                provider.selectable()
                    && self
                        .composer_state
                        .catalog_provider
                        .as_ref()
                        .is_none_or(|id| id == &provider.instance_id)
            })
            .flat_map(|provider| {
                self.catalog_models(provider)
                    .into_iter()
                    .map(|(_, model)| (provider.instance_id.clone(), model.slug.clone()))
            })
            .collect()
    }

    pub(crate) fn unavailable_model_options(&self, cx: &App) -> bool {
        let Some(selection) = self.current_model_selection(cx) else {
            return false;
        };
        let descriptors = self
            .selected_provider(cx)
            .and_then(|provider| {
                provider
                    .models
                    .iter()
                    .find(|model| model.slug == selection["model"])
            })
            .and_then(|model| model.capabilities["optionDescriptors"].as_array());
        if descriptors.is_none() {
            return false;
        }
        selection["options"].as_array().is_some_and(|options| {
            options.iter().any(|saved| {
                descriptors
                    .and_then(|descriptors| {
                        descriptors
                            .iter()
                            .find(|descriptor| descriptor["id"] == saved["id"])
                    })
                    .is_none_or(|descriptor| {
                        if descriptor["type"] == "boolean" {
                            !saved["value"].is_boolean()
                        } else {
                            !descriptor["options"].as_array().is_some_and(|choices| {
                                choices.iter().any(|choice| choice["id"] == saved["value"])
                            })
                        }
                    })
            })
        })
    }

    pub(crate) fn reset_model_options(&mut self, cx: &mut Context<Self>) {
        let Some(mut selection) = self.current_model_selection(cx) else {
            return;
        };
        selection.as_object_mut().unwrap().remove("options");
        self.model_override = Some(selection.clone());
        if let Some(draft) = self
            .pending_threads
            .iter_mut()
            .find(|draft| Some(&draft.thread.id) == self.selected_thread.as_ref())
        {
            draft.thread.model_selection = selection;
            if let Some(store) = &mut self.draft_store {
                store.put_pending(&self.environment_scope, draft.clone());
            }
            self.schedule_draft_save(cx);
        }
        cx.notify();
    }

    pub(crate) fn current_model_selection(&self, cx: &App) -> Option<Value> {
        let previous = self
            .selected_thread
            .as_ref()
            .and_then(|id| {
                self.shell
                    .threads
                    .iter()
                    .chain(self.pending_threads.iter().map(|draft| &draft.thread))
                    .find(|thread| &thread.id == id)
            })
            .map(|thread| &thread.model_selection)
            .unwrap_or(&Value::Null);
        self.model_selection(previous, cx).ok()
    }

    pub(crate) fn model_selection(
        &self,
        previous: &Value,
        cx: &App,
    ) -> Result<Value, &'static str> {
        let selected = commands::select_model(
            previous,
            &self.provider.read(cx).value(),
            &self.model.read(cx).value(),
        )?;
        Ok(self
            .model_override
            .as_ref()
            .filter(|value| {
                value["instanceId"] == selected["instanceId"] && value["model"] == selected["model"]
            })
            .cloned()
            .unwrap_or(selected))
    }

    pub(crate) fn choose_composer_trait(
        &mut self,
        id: &str,
        value: Value,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let descriptor = self
            .selected_provider(cx)
            .and_then(|p| {
                p.models
                    .iter()
                    .find(|m| m.slug == self.model.read(cx).value().as_str())
            })
            .and_then(|m| m.capabilities["optionDescriptors"].as_array())
            .and_then(|options| options.iter().find(|o| o["id"] == id));
        let injected = descriptor
            .and_then(|d| d["promptInjectedValues"].as_array())
            .is_some_and(|values| values.contains(&value));
        let prompt = self.composer.read(cx).value().to_string();
        if injected {
            if value != "ultrathink" {
                self.report_error(
                    "This prompt-injected trait is not supported by the native composer",
                    cx,
                );
                return;
            }
            let stripped = prompt.trim_start();
            let next = if stripped.to_lowercase().starts_with("ultrathink:") {
                prompt
            } else {
                format!("Ultrathink:\n{prompt}")
            };
            self.composer.update(cx, |input, cx| {
                input.set_value(crate::composer_state::prompt_content(&next), window, cx)
            });
            self.remember_composer_draft(cx);
        } else {
            let supported = descriptor.is_some_and(|d| {
                (d["type"] == "boolean" && value.is_boolean())
                    || d["options"]
                        .as_array()
                        .is_some_and(|choices| choices.iter().any(|c| c["id"] == value))
            });
            if !supported {
                self.choose_model_option(id, value, cx);
                return;
            }
            if id == "effort" && prompt.to_lowercase().starts_with("ultrathink:") {
                let next = prompt[11..].trim_start().to_owned();
                self.composer.update(cx, |input, cx| {
                    input.set_value(crate::composer_state::prompt_content(&next), window, cx)
                });
                self.remember_composer_draft(cx);
            }
            self.choose_model_option(id, value, cx);
        }
        cx.notify();
    }

    pub(crate) fn choose_model_option(&mut self, id: &str, value: Value, cx: &mut Context<Self>) {
        let previous = self
            .selected_thread
            .as_ref()
            .and_then(|id| {
                self.shell
                    .threads
                    .iter()
                    .chain(self.pending_threads.iter().map(|draft| &draft.thread))
                    .find(|thread| &thread.id == id)
            })
            .map(|thread| &thread.model_selection)
            .unwrap_or(&Value::Null);
        let mut selection = match self.model_selection(previous, cx) {
            Ok(selection) => selection,
            Err(error) => {
                self.report_error(error, cx);
                cx.notify();
                return;
            }
        };
        let descriptor = self
            .provider_catalog
            .iter()
            .find(|provider| {
                provider.instance_id == selection["instanceId"] && provider.selectable()
            })
            .and_then(|provider| {
                provider
                    .models
                    .iter()
                    .find(|model| model.slug == selection["model"])
            })
            .and_then(|model| model.capabilities["optionDescriptors"].as_array())
            .and_then(|options| options.iter().find(|option| option["id"] == id));
        let valid = descriptor.is_some_and(|descriptor| {
            (descriptor["type"] == "boolean" && value.is_boolean())
                || (descriptor["type"] == "select"
                    && descriptor["options"]
                        .as_array()
                        .is_some_and(|choices| choices.iter().any(|choice| choice["id"] == value)))
        });
        if !valid {
            self.report_error("Model option is unavailable; refresh the catalog", cx);
            cx.notify();
            return;
        }
        let mut options = selection["options"].as_array().cloned().unwrap_or_default();
        options.retain(|option| option["id"] != id);
        options.push(json!({"id":id,"value":value}));
        selection["options"] = json!(options);
        self.model_override = Some(selection.clone());
        if let Some(draft) = self
            .pending_threads
            .iter_mut()
            .find(|draft| Some(&draft.thread.id) == self.selected_thread.as_ref())
        {
            draft.thread.model_selection = selection;
            if let Some(store) = &mut self.draft_store {
                store.put_pending(&self.environment_scope, draft.clone());
            }
            self.schedule_draft_save(cx);
        }
        cx.notify();
    }

    pub(crate) fn pick_model(
        &mut self,
        instance: &str,
        model: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(provider) = self
            .provider_catalog
            .iter()
            .find(|provider| provider.instance_id == instance)
        else {
            self.report_error("Provider instance no longer available", cx);
            cx.notify();
            return;
        };
        if !provider.selectable() || !provider.models.iter().any(|entry| entry.slug == model) {
            self.report_error("Provider or model is unavailable; refresh its catalog", cx);
            cx.notify();
            return;
        }
        if provider.requires_new_thread_for_model_change
            && self.detail.thread.as_ref().is_some_and(|thread| {
                thread["messages"]
                    .as_array()
                    .is_some_and(|messages| !messages.is_empty())
                    && (thread["modelSelection"]["instanceId"] != instance
                        || thread["modelSelection"]["model"] != model)
            })
        {
            self.report_error("This provider requires a new thread for a model change", cx);
            cx.notify();
            return;
        }
        self.provider.update(cx, |input, cx| {
            input.set_value(instance.to_owned(), window, cx)
        });
        self.model.update(cx, |input, cx| {
            input.set_value(model.to_owned(), window, cx)
        });
        self.close_model_catalog(window, cx);
        if self.command_trigger(cx).is_some() {
            self.refresh_composer_catalog(false, cx);
        }
        cx.notify();
    }

    pub(crate) fn selected_provider(
        &self,
        cx: &App,
    ) -> Option<&t3code_desktop_gpui::providers::Provider> {
        let instance = self.provider.read(cx).value();
        self.provider_catalog
            .iter()
            .find(|provider| provider.instance_id == instance.as_str())
    }

    /// The effective runtime and interaction modes for the selected thread.
    pub(crate) fn selected_modes(&self) -> (String, String) {
        if let Some(detail) = &self.detail.thread
            && let (Some(runtime), Some(interaction)) = (
                detail["runtimeMode"].as_str(),
                detail["interactionMode"].as_str(),
            )
        {
            return (runtime.to_owned(), interaction.to_owned());
        }
        if let Some(id) = &self.selected_thread {
            if let Some(draft) = self
                .pending_threads
                .iter()
                .find(|draft| &draft.thread.id == id)
            {
                return (
                    draft
                        .thread
                        .runtime_mode
                        .clone()
                        .unwrap_or_else(|| "full-access".into()),
                    draft
                        .thread
                        .interaction_mode
                        .clone()
                        .unwrap_or_else(|| "default".into()),
                );
            }
            if let Some(thread) = self.shell.threads.iter().find(|thread| &thread.id == id) {
                return (
                    thread
                        .runtime_mode
                        .clone()
                        .unwrap_or_else(|| "full-access".into()),
                    thread
                        .interaction_mode
                        .clone()
                        .unwrap_or_else(|| "default".into()),
                );
            }
        }
        ("full-access".into(), "default".into())
    }

    /// Change a thread's runtime or interaction mode. Pending drafts update
    /// locally because the server has no thread to mutate yet.
    pub(crate) fn change_thread_mode(
        &mut self,
        kind: &str,
        field: &str,
        value: &str,
        cx: &mut Context<Self>,
    ) {
        let allowed = self.selected_provider(cx).is_some_and(|provider| {
            provider.selectable()
                && match field {
                    "runtimeMode" => provider.runtime_modes().contains(&value),
                    "interactionMode" => {
                        provider.show_interaction_mode_toggle.unwrap_or(true)
                            && matches!(value, "default" | "plan")
                    }
                    _ => false,
                }
        });
        if !allowed || self.pending_actions > 0 || self.checkout_transition_pending() {
            self.report_error(
                "Mode change unavailable for this provider or while an operation is pending",
                cx,
            );
            cx.notify();
            return;
        }
        let Some(id) = self.selected_thread.clone() else {
            return;
        };
        if let Some(draft) = self
            .pending_threads
            .iter_mut()
            .find(|draft| draft.thread.id == id)
        {
            match field {
                "runtimeMode" => draft.thread.runtime_mode = Some(value.to_owned()),
                "interactionMode" => draft.thread.interaction_mode = Some(value.to_owned()),
                _ => {}
            }
            if let Some(store) = &mut self.draft_store {
                store.put_pending(&self.environment_scope, draft.clone());
            }
            self.schedule_draft_save(cx);
            cx.notify();
            return;
        }
        let payload = commands::set_thread_mode(kind, &id, field, value);
        self.dispatch(payload, cx);
    }

    /// Begin a provider sign-in flow and surface its authorization URL.
    pub(crate) fn start_provider_auth(&mut self, instance_id: &str, cx: &mut Context<Self>) {
        if let Some(state) = &self.auth_state
            && let Some(previous) = state["instanceId"].as_str()
        {
            self.enqueue_subscription(
                t3code_desktop_gpui::connection::Command::Unsubscribe(format!("auth:{previous}")),
                cx,
            );
        }
        self.auth_state = None;
        self.perform(
            "provider.auth.start",
            json!({"instanceId": instance_id, "callbackMode": "client"}),
            None,
            cx,
        );
    }

    /// Finish a provider sign-in flow with a pasted callback URL.
    pub(crate) fn complete_provider_auth(&mut self, cx: &mut Context<Self>) {
        let Some(state) = self.auth_state.clone() else {
            return;
        };
        if !crate::settings_provider::accepts_callback(&state) {
            self.report_error("This sign-in flow does not accept a callback", cx);
            return;
        }
        let instance = state["instanceId"].as_str().unwrap_or_default().to_owned();
        let flow = state["flowId"].as_str().unwrap_or_default().to_owned();
        let callback = self.auth_callback.read(cx).value().trim().to_owned();
        if callback.is_empty() || callback.encode_utf16().count() > 16_384 {
            self.report_error("Paste the callback URL first", cx);
            cx.notify();
            return;
        }
        self.perform(
            "provider.auth.complete",
            json!({"instanceId": instance, "flowId": flow, "callbackUrl": callback}),
            None,
            cx,
        );
    }
}
