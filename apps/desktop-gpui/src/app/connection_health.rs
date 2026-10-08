//! Truthful readiness and presentation shared by settings and mutation controls.
use crate::NativeApp;
use gpui_kit::Context;
use serde_json::Value;
use t3code_desktop_gpui::{connection, notifications};

impl NativeApp {
    pub(crate) fn connection_ready(&self) -> bool {
        self.connection_phase.ready() && self.shell.synchronized && self.commands.is_some()
            && (self.sessions.selected().is_none() || self.environment_identity_verified)
    }

    pub(crate) fn connection_state_text(&self) -> String {
        if matches!(self.connection_phase, connection::Phase::Ready) && !self.connection_ready() {
            if self.commands.is_none() {
                connection::Phase::Offline.label()
            } else {
                connection::Phase::Syncing.label()
            }
        } else {
            self.connection_phase.label()
        }
    }

    pub(crate) fn retry_connection_now(&mut self, cx: &mut Context<Self>) {
        if matches!(
            self.connection_phase,
            connection::Phase::Reconnecting { .. }
        ) {
            let Some(sender) = &self.commands else {
                self.connect(cx);
                return;
            };
            if sender.try_send(connection::Command::RetryNow).is_err() {
                self.report_error("Retry could not be queued; reconnect from Settings", cx);
            }
        } else if matches!(
            self.connection_phase,
            connection::Phase::Offline | connection::Phase::Error(_)
        ) {
            self.connect(cx);
        }
    }

    pub(crate) fn apply_connection_phase(
        &mut self,
        phase: connection::Phase,
        cx: &mut Context<Self>,
    ) {
        if let connection::Phase::Error(reason) = &phase {
            self.report_error(reason.clone(), cx);
        } else {
            self.report_progress(phase.label(), cx);
        }
        self.connection_phase = if phase.ready() && self.sessions.selected().is_some() && !self.environment_identity_verified {
            connection::Phase::Syncing
        } else { phase };
        self.publish_environment_phase(cx);
        cx.notify();
    }

    pub(crate) fn update_environment_identity(&mut self, config: &Value) {
        self.environment_name = config["environment"]["label"]
            .as_str()
            .filter(|name| !name.trim().is_empty())
            .map(str::to_owned);
    }

    pub(crate) fn notification_state_text(&self) -> &'static str {
        self.notification_platform.state_text()
    }

    pub(crate) fn status_text(&self) -> String {
        if self.connection_ready() {
            self.status.clone()
        } else if self.status.is_empty() || self.status_kind == notifications::Outcome::Progress {
            self.connection_state_text()
        } else {
            format!("{} · {}", self.connection_state_text(), self.status)
        }
    }
}
