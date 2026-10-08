//! Durable source attribution, separate from generation-owned mode receipts.
use crate::NativeApp;
use gpui_kit::Context;
use t3code_desktop_gpui::{commands, connection, drafts::PlanIntent};

impl NativeApp {
    pub(crate) fn durable_plan_intent(&self) -> Option<PlanIntent> {
        let thread = self.selected_thread.as_ref()?;
        // The store is authoritative after accepted/background draft clearing.
        // A cached record must not resurrect an already consumed attribution.
        if let Some(store) = &self.draft_store {
            return store.plan_intent(&self.environment_scope, thread);
        }
        self.composer_state
            .plan_intents
            .get(&(self.environment_scope.clone(), thread.clone()))
            .filter(|intent| self.drafts.get(thread) == Some(&intent.prompt))
            .cloned()
    }

    /// Accepted receipts consume only the matching owned prompt, never a newer draft.
    pub(crate) fn consume_plan_intent(&mut self, environment: &str, thread: &str, prompt: &str) {
        let key = (environment.to_owned(), thread.to_owned());
        if self
            .composer_state
            .plan_intents
            .get(&key)
            .is_some_and(|intent| intent.prompt == prompt)
        {
            self.composer_state.plan_intents.remove(&key);
        }
        if let Some(store) = &mut self.draft_store
            && store
                .plan_intent(environment, thread)
                .is_some_and(|intent| intent.prompt == prompt)
        {
            store.set_plan_intent(environment, thread, None);
        }
    }

    pub(crate) fn capture_plan_intent(&mut self, intent: Option<PlanIntent>) {
        let Some(thread) = self.selected_thread.clone() else {
            return;
        };
        let key = (self.environment_scope.clone(), thread.clone());
        if let Some(intent) = &intent {
            self.drafts.insert(thread.clone(), intent.prompt.clone());
            self.composer_state.plan_intents.insert(key, intent.clone());
        } else {
            self.composer_state.plan_intents.remove(&key);
        }
        if let Some(store) = &mut self.draft_store {
            store.set_plan_intent(&self.environment_scope, &thread, intent);
        }
    }

    pub(crate) fn durable_plan_mode(&self, text: &str) -> Result<Option<String>, String> {
        let Some(intent) = self.durable_plan_intent() else {
            return self.confirmed_plan_followup_mode(text);
        };
        if !matches!(intent.mode.as_str(), "default" | "plan")
            || intent.plan.trim().is_empty()
            || intent.source_thread.trim().is_empty()
        {
            return Err(
                "Invalid stored plan intent; discard attribution and select its source again"
                    .into(),
            );
        }
        if intent.environment != self.environment_scope || intent.prompt != text {
            return Err(
                "Plan attribution does not match this environment or prompt; confirm or discard it"
                    .into(),
            );
        }
        let fresh = self
            .composer_state
            .plan_confirmations
            .get(&intent.id)
            .is_some_and(|(generation, thread, accepted)| {
                *generation == self.generation
                    && Some(thread) == self.selected_thread.as_ref()
                    && *accepted
            });
        let live = self
            .conversation_presentation
            .plan_followup
            .as_ref()
            .is_some_and(|record| {
                record.generation == self.generation
                    && record.environment == intent.environment
                    && Some(&record.thread) == self.selected_thread.as_ref()
                    && record.thread == intent.source_thread
                    && record.plan == intent.plan
                    && record.mode == intent.mode
                    && record.accepted == Some(true)
            });
        if !fresh && !live {
            return Err(
                "Confirm the durable plan intent with a fresh mode receipt before sending".into(),
            );
        }
        Ok(Some(intent.mode))
    }

    pub(crate) fn durable_plan_source(&self, text: &str) -> Option<serde_json::Value> {
        if let Some(intent) = self.durable_plan_intent() {
            self.durable_plan_mode(text).ok().flatten()?;
            return (intent.mode == "default").then(|| intent.source());
        }
        self.plan_followup_source(text)
    }

    pub(crate) fn confirm_durable_plan(&mut self, cx: &mut Context<Self>) {
        let Some(mut intent) = self.durable_plan_intent() else {
            return;
        };
        if intent.environment != self.environment_scope
            || !self.connection_ready()
            || self.pending_actions > 0
        {
            return;
        }
        intent.prompt = self.composer.read(cx).value().to_string();
        if intent.prompt.trim().is_empty() {
            return;
        }
        self.capture_plan_intent(Some(intent.clone()));
        self.schedule_draft_save(cx);
        let Some(sender) = self.commands.clone() else {
            return;
        };
        let Some(thread) = self.selected_thread.clone() else {
            return;
        };
        let generation = self.generation;
        let environment = self.environment_scope.clone();
        let payload = commands::set_thread_mode(
            "thread.interaction-mode.set",
            &thread,
            "interactionMode",
            &intent.mode,
        );
        self.pending_actions += 1;
        cx.spawn(async move |entity, cx| {
            let result = connection::call(&sender, "orchestration.dispatchCommand", payload).await;
            let _ = entity.update(cx, |app, cx| {
                if app.generation != generation || app.environment_scope != environment {
                    return;
                }
                app.pending_actions = app.pending_actions.saturating_sub(1);
                app.composer_state
                    .plan_confirmations
                    .insert(intent.id, (generation, thread, result.is_ok()));
                match result {
                    Ok(_) => app.report_success("Plan intent confirmed · review and Send", cx),
                    Err(error) => app.report_error(format!("Plan mode rejected: {error}"), cx),
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(crate) fn render_durable_plan(&self, cx: &mut Context<Self>) -> gpui_kit::Div {
        use gpui_kit::component::{
            Disableable, Sizable,
            button::{Button, ButtonVariants},
        };
        use gpui_kit::{div, prelude::*};
        let Some(intent) = self.durable_plan_intent() else {
            return div();
        };
        let pending = self
            .durable_plan_mode(&self.composer.read(cx).value())
            .is_err();
        // Preserve the existing reconfirm target when a live preparation fails;
        // restored/cross-source intents have their independent composer target.
        let confirm_id = if self
            .conversation_presentation
            .plan_followup
            .as_ref()
            .is_some_and(|record| {
                record.plan == intent.plan
                    && record.thread == intent.source_thread
                    && record.accepted == Some(false)
            }) {
            "plan-draft-reconfirm"
        } else {
            "durable-plan-confirm"
        };
        div()
            .debug_selector(|| "durable-plan-intent".into())
            .flex()
            .items_center()
            .gap_2()
            .text_xs()
            .child(format!(
                "Plan {} · source {} · {}",
                intent.plan, intent.source_thread, intent.mode
            ))
            .when(pending, |view| {
                view.child(
                    Button::new(confirm_id)
                        .debug_selector(move || confirm_id.into())
                        .ghost()
                        .small()
                        .label("Confirm intent and mode")
                        .disabled(!self.connection_ready() || self.pending_actions > 0)
                        .on_click(cx.listener(|app, _, _, cx| app.confirm_durable_plan(cx))),
                )
            })
            .child(
                Button::new("durable-plan-discard")
                    .debug_selector(|| "durable-plan-discard".into())
                    .ghost()
                    .small()
                    .label("Discard attribution")
                    .on_click(cx.listener(|app, _, _, cx| {
                        app.capture_plan_intent(None);
                        app.conversation_presentation.plan_followup = None;
                        app.schedule_draft_save(cx);
                        cx.notify();
                    })),
            )
    }
}
