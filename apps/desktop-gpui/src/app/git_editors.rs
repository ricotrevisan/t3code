use crate::NativeApp;
use gpui_kit::component::{
    ActiveTheme as _, Disableable, Sizable,
    button::{Button, ButtonVariants},
};
use gpui_kit::{Context, div, prelude::*};
#[derive(Default)]
pub(crate) struct EditorChoice {
    pub open: bool,
    pub preferred: Option<String>,
}
impl EditorChoice {
    pub(crate) fn restore() -> Self {
        #[cfg(test)]
        let preferred = None;
        #[cfg(not(test))]
        let preferred = std::fs::read_to_string(
            t3code_desktop_gpui::drafts::default_path().with_file_name("preferred-editor.txt"),
        )
        .ok();
        Self {
            preferred,
            open: false,
        }
    }
}
impl NativeApp {
    pub(crate) fn editor_unavailable_reason(&self) -> Option<&'static str> {
        let remote = url::Url::parse(&self.environment_scope)
            .ok()
            .and_then(|url| url.host_str().map(str::to_owned))
            .is_some_and(|host| {
                host != "localhost"
                    && host
                        .parse::<std::net::IpAddr>()
                        .ok()
                        .is_none_or(|ip| !ip.is_loopback())
            });
        if remote {
            Some(
                "Remote checkout: local SSH editor routes are not supported yet. Opening on the server is disabled.",
            )
        } else if self.available_editors.is_empty() {
            Some("No available editors or file managers reported by the server")
        } else {
            None
        }
    }
    pub(crate) fn preferred_editor(&self) -> Option<String> {
        self.editor_choice
            .preferred
            .as_ref()
            .filter(|editor| self.available_editors.contains(editor))
            .cloned()
            .or_else(|| {
                self.available_editors
                    .iter()
                    .find(|editor| editor.as_str() == "zed")
                    .cloned()
            })
            .or_else(|| self.available_editors.first().cloned())
    }
    pub(crate) fn select_editor(&mut self, editor: String, cx: &mut Context<Self>) {
        if !self.available_editors.contains(&editor) || self.editor_unavailable_reason().is_some() {
            return;
        }
        self.editor_choice.preferred = Some(editor.clone());
        self.editor_choice.open = false;
        #[cfg(not(test))]
        {
            let path =
                t3code_desktop_gpui::drafts::default_path().with_file_name("preferred-editor.txt");
            if let Err(error) = super::checkout_operations::persist(&path, editor.as_bytes()) {
                self.report_error(format!("Could not save preferred editor: {error}"), cx);
            }
        }
        self.open_in_editor(cx);
    }
    pub(crate) fn render_editor_choices(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let reason = self.editor_unavailable_reason();
        div()
            .flex()
            .flex_col()
            .gap_1()
            .p_2()
            .bg(cx.theme().popover)
            .when_some(reason, |view, reason| {
                view.child(div().text_xs().child(reason))
            })
            .children(self.available_editors.iter().map(|editor| {
                let editor = editor.clone();
                Button::new(gpui_kit::SharedString::from(format!("open-in-{editor}")))
                    .ghost()
                    .small()
                    .label(editor.clone())
                    .disabled(reason.is_some())
                    .on_click(
                        cx.listener(move |app, _, _, cx| app.select_editor(editor.clone(), cx)),
                    )
            }))
    }
}
