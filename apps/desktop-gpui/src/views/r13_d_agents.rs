//! Timeline identity is read from the same stamped, lifecycle-aware roster as Agents.
use super::*;
impl NativeApp {
    pub(super) fn render_timeline_agent(
        &self,
        activity: &Value,
        cx: &mut Context<Self>,
    ) -> Option<gpui_kit::Div> {
        let id = activity["payload"]["taskId"].as_str()?;
        let agent = self
            .conversation_presentation
            .agents
            .iter()
            .find(|agent| agent.id == id && activity["id"] == agent.activity_id)?;
        let color = match agent.status {
            t3code_desktop_gpui::surfaces::agents::Status::Failed => cx.theme().danger,
            t3code_desktop_gpui::surfaces::agents::Status::Completed => cx.theme().success,
            _ => cx.theme().muted_foreground,
        };
        let status = match agent.status {
            t3code_desktop_gpui::surfaces::agents::Status::Pending => "Pending",
            t3code_desktop_gpui::surfaces::agents::Status::Running => "Running",
            t3code_desktop_gpui::surfaces::agents::Status::Waiting => "Waiting for input",
            _ => agent.status.label(),
        };
        let detail = [
            agent.role.as_deref(),
            agent.model.as_deref(),
            agent.effort.as_deref(),
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" · ");
        let key = format!("agent-detail:{}", agent.id);
        let expanded = self.activity_expanded.get(&key) == Some(&true);
        let selector = format!("timeline-agent-{}", agent.id);
        Some(
            div()
                .flex()
                .flex_col()
                .gap_1()
                .p_2()
                .border_l_1()
                .border_color(cx.theme().border)
                .child(
                    Button::new(gpui_kit::SharedString::from(selector.clone()))
                        .debug_selector(move || selector.clone())
                        .ghost()
                        .small()
                        .icon(IconName::User)
                        .label(format!("{} · {}", agent.title, status))
                        .on_click(cx.listener(move |app, _, _, cx| {
                            app.conversation_scroll.pause_following_tail();
                            app.activity_expanded.insert(key.clone(), !expanded);
                            cx.notify();
                        })),
                )
                .when(!detail.is_empty(), |view| {
                    view.child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(detail),
                    )
                })
                .children(
                    agent
                        .progress
                        .clone()
                        .map(|progress| div().text_xs().child(progress)),
                )
                .when(expanded, |view| {
                    view.child(
                        div()
                            .id(gpui_kit::SharedString::from(format!(
                                "agent-detail-scroll-{}",
                                agent.id
                            )))
                            .max_h(px(180.))
                            .overflow_y_scroll()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .text_xs()
                            .text_color(color)
                            .children(agent.tool.clone())
                            .children(agent.result.clone())
                            .children(agent.error.clone())
                            .child(format!(
                                "Activation {}{}",
                                agent.activations,
                                agent.attempt.map_or(String::new(), |attempt| format!(
                                    " · attempt {attempt}"
                                ))
                            ))
                            .child(
                                Button::new(gpui_kit::SharedString::from(format!(
                                    "timeline-agent-inspector-{}",
                                    agent.id
                                )))
                                .ghost()
                                .xsmall()
                                .label("Open Agents")
                                .on_click(cx.listener(
                                    |app, _, _, cx| {
                                        app.inspector_open = true;
                                        app.right_tab = crate::RightTab::Agents;
                                        cx.notify();
                                    },
                                )),
                            ),
                    )
                }),
        )
    }
}
