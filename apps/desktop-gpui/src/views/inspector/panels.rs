//! The Changes and Agents tab bodies of the right panel.
use gpui_kit::component::ActiveTheme as _;
use gpui_kit::{Context, div, prelude::*};

use crate::NativeApp;

impl NativeApp {
    pub(super) fn render_changes_panel(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let muted_foreground = cx.theme().muted_foreground;
        let files = self
            .detail
            .thread
            .as_ref()
            .and_then(|detail| {
                detail["checkpoints"]
                    .as_array()
                    .and_then(|list| list.last())
            })
            .and_then(|checkpoint| checkpoint["files"].as_array())
            .cloned()
            .unwrap_or_default();
        div()
            .id("changes-panel")
            .debug_selector(|| "changes-panel".into())
            .flex()
            .flex_col()
            .gap_2()
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .when(files.is_empty(), |view| {
                view.child(
                    div()
                        .text_sm()
                        .text_color(muted_foreground)
                        .child("No checkpointed changes yet"),
                )
            })
            .children(files.iter().map(|file| {
                div()
                    .flex()
                    .justify_between()
                    .gap_2()
                    .text_sm()
                    .child(
                        div()
                            .min_w_0()
                            .truncate()
                            .child(file["path"].as_str().unwrap_or("").to_owned()),
                    )
                    .child(div().text_xs().child(format!(
                        "+{} -{}",
                        file["additions"].as_u64().unwrap_or(0),
                        file["deletions"].as_u64().unwrap_or(0)
                    )))
            }))
    }

    pub(super) fn render_agents_panel(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let muted_foreground = cx.theme().muted_foreground;
        div()
            .id("agents-panel")
            .debug_selector(|| "agents-panel".into())
            .flex()
            .flex_col()
            .gap_2()
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .when(self.provider_catalog.is_empty(), |view| {
                view.child(
                    div()
                        .text_sm()
                        .text_color(muted_foreground)
                        .child("No provider catalog loaded"),
                )
            })
            .children(self.provider_catalog.iter().map(|provider| {
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(div().text_sm().child(provider.label().to_owned()))
                    .child(div().text_xs().text_color(muted_foreground).child(format!(
                        "{} · {}",
                        provider.instance_id,
                        provider.explanation()
                    )))
            }))
    }
}
