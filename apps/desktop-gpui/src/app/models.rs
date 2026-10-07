//! Model and mode selection, and provider sign-in.

use crate::NativeApp;
use gpui_kit::{App, Context, Window};
use serde_json::{Value, json};
use t3code_desktop_gpui::commands;

impl NativeApp {
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
                self.set_status(error, cx);
                cx.notify();
                return;
            }
        };
        let descriptor = self
            .provider_catalog
            .iter()
            .find(|provider| provider.instance_id == selection["instanceId"])
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
            self.set_status("Model option is unavailable; refresh the catalog", cx);
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
            self.set_status("Provider instance no longer available", cx);
            cx.notify();
            return;
        };
        if !provider.selectable() || !provider.models.iter().any(|entry| entry.slug == model) {
            self.set_status("Provider or model is unavailable; refresh its catalog", cx);
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
            self.set_status("This provider requires a new thread for a model change", cx);
            cx.notify();
            return;
        }
        self.provider.update(cx, |input, cx| {
            input.set_value(instance.to_owned(), window, cx)
        });
        self.model.update(cx, |input, cx| {
            input.set_value(model.to_owned(), window, cx)
        });
        self.catalog_open = false;
        cx.notify();
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
        let instance = state["instanceId"].as_str().unwrap_or_default().to_owned();
        let flow = state["flowId"].as_str().unwrap_or_default().to_owned();
        let callback = self.auth_callback.read(cx).value().trim().to_owned();
        if callback.is_empty() {
            self.set_status("Paste the callback URL first", cx);
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
