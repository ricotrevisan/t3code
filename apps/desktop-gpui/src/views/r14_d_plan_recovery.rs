//! Recover an unchanged plan draft only from one exact, loaded source plan.
//! The restored text is not a mode receipt: the user must confirm again.
use super::*;

pub(crate) fn prompt(markdown: &str, implement: bool) -> String {
    format!(
        "{}\n\n{markdown}",
        if implement {
            "Implement the following plan:"
        } else {
            "Refine the following plan:"
        }
    )
}

pub(crate) fn recover(
    detail: &Value,
    text: &str,
) -> Result<Option<(String, String, bool)>, &'static str> {
    let Some((heading, markdown)) = text.split_once("\n\n") else {
        return Ok(None);
    };
    let implement = match heading {
        "Implement the following plan:" => true,
        "Refine the following plan:" => false,
        _ => return Ok(None),
    };
    let matches = detail["proposedPlans"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|plan| {
            plan["planMarkdown"].as_str() == Some(markdown) && plan["id"].as_str().is_some()
        })
        .collect::<Vec<_>>();
    match matches.as_slice() {
        [] => Ok(None),
        [plan] => Ok(Some((
            plan["id"].as_str().unwrap().to_owned(),
            markdown.to_owned(),
            implement,
        ))),
        _ => Err(
            "This draft matches multiple plans; clear it and select the intended source plan again",
        ),
    }
}

impl NativeApp {
    pub(super) fn render_plan_draft_recovery(&self, cx: &mut Context<Self>) -> gpui_kit::Div {
        let text = self.composer.read(cx).value();
        let Some(detail) = self.detail.thread.as_ref() else {
            return div();
        };
        if self
            .conversation_presentation
            .plan_followup
            .as_ref()
            .is_some_and(|record| {
                record.generation == self.generation
                    && record.environment == self.environment_scope
                    && Some(record.thread.as_str()) == self.selected_thread.as_deref()
                    && record.prompt == text.as_str()
                    && record.accepted == Some(true)
            })
        {
            return div();
        }
        let Ok(Some((id, markdown, implement))) = recover(detail, &text) else {
            return div();
        };
        let owner = (
            self.generation,
            self.environment_scope.clone(),
            self.selected_thread.clone(),
        );
        div()
            .debug_selector(|| "plan-draft-recovery".into())
            .flex()
            .items_center()
            .gap_2()
            .p_2()
            .text_xs()
            .child("Plan follow-up draft · confirm mode and source before sending")
            .child(
                Button::new("plan-draft-reconfirm")
                    .debug_selector(|| "plan-draft-reconfirm".into())
                    .ghost()
                    .xsmall()
                    .label(if implement {
                        "Confirm Implement"
                    } else {
                        "Confirm Refine"
                    })
                    .disabled(!self.connection_ready() || self.pending_actions > 0)
                    .on_click(cx.listener(move |app, _, window, cx| {
                        if owner
                            != (
                                app.generation,
                                app.environment_scope.clone(),
                                app.selected_thread.clone(),
                            )
                        {
                            return;
                        }
                        app.prepare_plan_followup(
                            id.clone(),
                            markdown.clone(),
                            implement,
                            window,
                            cx,
                        )
                    })),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn r14_d_plan_recovery_requires_exact_unique_loaded_source() {
        let mut detail =
            serde_json::json!({"proposedPlans":[{"id":"p","planMarkdown":"# Plan\n\nDo it"}]});
        let original = prompt("# Plan\n\nDo it", true);
        assert_eq!(
            recover(&detail, &original).unwrap().unwrap(),
            ("p".into(), "# Plan\n\nDo it".into(), true)
        );
        assert!(
            recover(&detail, &(original.clone() + " edited"))
                .unwrap()
                .is_none()
        );
        assert!(recover(&detail, "Ordinary draft").unwrap().is_none());
        detail["proposedPlans"]
            .as_array_mut()
            .unwrap()
            .push(serde_json::json!({"id":"other","planMarkdown":"# Plan\n\nDo it"}));
        assert!(recover(&detail, &original).is_err());
    }
}
