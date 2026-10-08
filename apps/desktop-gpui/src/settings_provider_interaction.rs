//! Authentication interaction replies are bound to the captured environment/flow/interaction.
use crate::{NativeApp, settings_provider};
use gpui_kit::component::{
    ActiveTheme, Disableable, Sizable,
    button::{Button, ButtonVariants},
    input::{Input, InputState},
};
use gpui_kit::{AppContext, Context, Entity, IntoElement, Task, Window, div, prelude::*};
use serde_json::{Value, json};
use t3code_desktop_gpui::connection;

#[derive(Clone, Debug, PartialEq, Eq)]
struct Owner {
    generation: u64,
    instance: String,
    flow: String,
    interaction: String,
}
fn owner(state: &Value, generation: u64) -> Option<Owner> {
    if !settings_provider::auth_active(state) {
        return None;
    }
    let nonempty = |value: &Value| {
        value
            .as_str()
            .filter(|s| !s.is_empty() && s.len() <= 128)
            .map(str::to_owned)
    };
    Some(Owner {
        generation,
        instance: nonempty(&state["instanceId"])?,
        flow: nonempty(&state["flowId"])?,
        interaction: nonempty(&state["interaction"]["id"])?,
    })
}
pub(crate) struct Editor {
    fields: [Entity<InputState>; 16],
    terminal: Entity<InputState>,
    owner: Option<Owner>,
    pending: Option<Owner>,
    task: Option<Task<()>>,
    error: Option<String>,
}
impl Editor {
    pub fn new(window: &mut Window, cx: &mut gpui_kit::App) -> Self {
        Self {
            fields: std::array::from_fn(|_| cx.new(|cx| InputState::new(window, cx).masked(true))),
            terminal: cx.new(|cx| {
                InputState::new(window, cx).placeholder("Input for this sign-in process")
            }),
            owner: None,
            pending: None,
            task: None,
            error: None,
        }
    }
}
fn payload(state: &Value, captured: &Owner, response: Value) -> Result<Value, String> {
    if owner(state, captured.generation).as_ref() != Some(captured) {
        return Err("Authentication interaction changed; review the new prompt".into());
    }
    let interaction = &state["interaction"];
    if interaction["type"] != response["type"] {
        return Err("Response does not match the authentication prompt".into());
    }
    match response["type"].as_str() {
        Some("browser")
            if interaction["requiresConsent"] == true
                && matches!(response["action"].as_str(), Some("accept" | "decline")) => {}
        Some("terminal")
            if response["data"]
                .as_str()
                .is_some_and(|s| s.encode_utf16().count() <= 4096) => {}
        Some("credentials") => {
            let fields = interaction["fields"]
                .as_array()
                .filter(|fields| !fields.is_empty() && fields.len() <= 16)
                .ok_or("Invalid credential descriptor")?;
            let values = response["values"]
                .as_object()
                .ok_or("Invalid credential values")?;
            if fields.len() != values.len()
                || fields.iter().any(|field| {
                    field["name"].as_str().is_none_or(|name| {
                        values
                            .get(name)
                            .and_then(Value::as_str)
                            .is_none_or(|value| value.encode_utf16().count() > 16_384)
                    })
                })
            {
                return Err("Credential values do not match the requested fields".into());
            }
        }
        _ => return Err("Unsupported authentication interaction".into()),
    }
    Ok(
        json!({"instanceId":captured.instance,"flowId":captured.flow,"interactionId":captured.interaction,"response":response}),
    )
}
impl NativeApp {
    fn prepare_auth_inputs(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(captured) = self
            .auth_state
            .as_ref()
            .and_then(|state| owner(state, self.generation))
        else {
            return;
        };
        for field in &self.provider_auth_editor.fields {
            field.update(cx, |input, cx| input.set_value("", window, cx));
        }
        self.provider_auth_editor
            .terminal
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.provider_auth_editor.owner = Some(captured);
        self.provider_auth_editor.error = None;
        if self
            .auth_state
            .as_ref()
            .is_some_and(|state| state["interaction"]["type"] == "credentials")
        {
            self.provider_auth_editor.fields[0].update(cx, |input, cx| input.focus(window, cx));
        } else {
            self.provider_auth_editor
                .terminal
                .update(cx, |input, cx| input.focus(window, cx));
        }
        cx.notify();
    }
    pub(crate) fn respond_provider_interaction(&mut self, response: Value, cx: &mut Context<Self>) {
        if !self.connection_ready() {
            self.provider_auth_editor.error = Some("Reconnect before answering sign-in".into());
            cx.notify();
            return;
        }
        let Some(state) = self.auth_state.clone() else {
            return;
        };
        let Some(captured) = owner(&state, self.generation) else {
            return;
        };
        if self.provider_auth_editor.pending.is_some() {
            return;
        }
        let data = match payload(&state, &captured, response) {
            Ok(data) => data,
            Err(error) => {
                self.provider_auth_editor.error = Some(error);
                cx.notify();
                return;
            }
        };
        let Some(sender) = self.commands.clone() else {
            return;
        };
        self.provider_auth_editor.owner = Some(captured.clone());
        self.provider_auth_editor.pending = Some(captured.clone());
        self.provider_auth_editor.error = None;
        self.provider_auth_editor.task = Some(cx.spawn(async move |entity, cx| {
            let result = connection::call(&sender, "provider.auth.respond", data).await;
            let _ =
                entity.update_in(cx, |app, window, cx| {
                    if app.provider_auth_editor.pending.as_ref() == Some(&captured) {
                        app.provider_auth_editor.pending = None;
                    }
                    if app.generation != captured.generation
                        || app
                            .auth_state
                            .as_ref()
                            .and_then(|state| owner(state, app.generation))
                            .as_ref()
                            != Some(&captured)
                    {
                        cx.notify();
                        return;
                    }
                    match result {
                        Ok(state)
                            if state["instanceId"] == captured.instance
                                && state["flowId"] == captured.flow =>
                        {
                            for field in &app.provider_auth_editor.fields {
                                field.update(cx, |input, cx| input.set_value("", window, cx));
                            }
                            app.provider_auth_editor
                                .terminal
                                .update(cx, |input, cx| input.set_value("", window, cx));
                            app.provider_auth_editor.owner = None;
                            app.accept_provider_auth(state, false, cx);
                        }
                        Ok(_) => {
                            app.provider_auth_editor.error = Some(
                                "Server returned a different authentication flow; refresh sign-in"
                                    .into(),
                            )
                        }
                        Err(_) => app.provider_auth_editor.error = Some(
                            "Sign-in response failed. Review the current prompt before retrying."
                                .into(),
                        ),
                    }
                    cx.notify();
                });
        }));
        cx.notify();
    }
    fn submit_auth_inputs(&mut self, cx: &mut Context<Self>) {
        let Some(state) = self.auth_state.as_ref() else {
            return;
        };
        if self.provider_auth_editor.owner != owner(state, self.generation) {
            return;
        }
        let response = match state["interaction"]["type"].as_str() {
            Some("terminal") => {
                json!({"type":"terminal","data":format!("{}\n", self.provider_auth_editor.terminal.read(cx).value())})
            }
            Some("credentials") => {
                let Some(fields) = state["interaction"]["fields"]
                    .as_array()
                    .filter(|fields| fields.len() <= 16)
                else {
                    return;
                };
                let mut values = serde_json::Map::new();
                for (index, field) in fields.iter().enumerate() {
                    let Some(name) = field["name"].as_str() else {
                        return;
                    };
                    values.insert(
                        name.into(),
                        self.provider_auth_editor.fields[index]
                            .read(cx)
                            .value()
                            .to_string()
                            .into(),
                    );
                }
                json!({"type":"credentials","values":values})
            }
            _ => return,
        };
        self.respond_provider_interaction(response, cx);
    }
    pub(crate) fn render_provider_interaction(
        &self,
        state: &Value,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let captured = owner(state, self.generation);
        let active = captured.is_some();
        let ready = self.connection_ready() && self.provider_auth_editor.pending.is_none();
        let prepared = active && self.provider_auth_editor.owner == captured;
        let credentials = state["interaction"]["type"] == "credentials";
        let terminal = state["interaction"]["type"] == "terminal";
        let fields = state["interaction"]["fields"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        let valid_fields = !fields.is_empty()
            && fields.len() <= 16
            && fields.iter().all(|field| {
                field["name"].as_str().is_some_and(|name| !name.is_empty())
                    && field["label"].is_string()
            });
        div().flex().flex_col().gap_2()
            .when(active && state["interaction"]["type"] == "browser" && state["interaction"]["requiresConsent"] == true, |view| view
                .child(div().text_sm().child(format!("Authorize using {}", state["interaction"]["url"].as_str().unwrap_or("the provider page"))))
                .child(div().flex().gap_1().children([("accept", "Authorize"), ("decline", "Decline")].into_iter().map(|(action, label)| Button::new(action).debug_selector(move || format!("provider-consent-{action}")).label(label).ghost().small().disabled(!ready).on_click(cx.listener(move |app, _, _, cx| app.respond_provider_interaction(json!({"type":"browser","action":action}), cx)))))))
            .when(active && (terminal || credentials), |view| view
                .when(credentials && !valid_fields, |view| view.child(div().text_sm().child("This credential descriptor cannot be rendered safely. Cancel sign-in or use the web client.")))
                .when(!prepared && (terminal || valid_fields), |view| view.child(Button::new("auth-prepare-inputs").debug_selector(|| "auth-prepare-inputs".into()).label(if terminal { "Enter sign-in input" } else { "Enter credentials" }).ghost().small().disabled(!ready).on_click(cx.listener(|app, _, window, cx| app.prepare_auth_inputs(window, cx)))))
                .when(prepared && terminal, |view| view.child(div().debug_selector(|| "auth-terminal-input".into()).child(Input::new(&self.provider_auth_editor.terminal).small())))
                .when(prepared && credentials && valid_fields, |view| view.children(fields.into_iter().enumerate().map(|(index, field)| div().flex().flex_col().gap_1().child(div().text_sm().child(field["label"].as_str().unwrap_or("Credential").to_string())).child(div().debug_selector(move || format!("auth-credential-{index}")).child(Input::new(&self.provider_auth_editor.fields[index]).small())))))
                .when(prepared && (terminal || valid_fields), |view| view.child(Button::new("auth-submit-inputs").debug_selector(|| "auth-submit-inputs".into()).label(if terminal { "Send input line" } else { "Submit credentials" }).ghost().small().disabled(!ready).on_click(cx.listener(|app, _, _, cx| app.submit_auth_inputs(cx))))))
            .when_some(self.provider_auth_editor.error.clone().filter(|_| self.provider_auth_editor.owner == captured), |view, error| view.child(div().text_sm().text_color(cx.theme().danger).child(error)))
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn r13_f_auth_response_validates_utf16_descriptor_and_identity() {
        let state = json!({"instanceId":"personal","flowId":"flow","phase":"waiting","interaction":{"id":"challenge","type":"terminal"}});
        let captured = owner(&state, 3).unwrap();
        assert_eq!(
            payload(&state, &captured, json!({"type":"terminal","data":"ok\n"})).unwrap()["interactionId"],
            "challenge"
        );
        assert!(
            payload(
                &state,
                &captured,
                json!({"type":"terminal","data":"😀".repeat(2049)})
            )
            .is_err()
        );
        let mut changed = state.clone();
        changed["interaction"]["id"] = json!("new");
        assert!(payload(&changed, &captured, json!({"type":"terminal","data":"old"})).is_err());
        assert!(payload(&state, &captured, json!({"type":"credentials","values":{}})).is_err());
    }
}
