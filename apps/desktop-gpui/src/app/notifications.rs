//! Status line, toasts, and desktop notifications.

use crate::NativeApp;
use gpui_kit::{Context, Window};
use t3code_desktop_gpui::{notifications, shell};

impl NativeApp {
    /// Shows `status` in the status bar; a failure also raises an error toast.
    pub(crate) fn set_status(&mut self, status: impl Into<String>, cx: &mut Context<Self>) {
        self.status = status.into();
        if notifications::is_error(&self.status) {
            let message = self.status.clone();
            self.push_toast(notifications::ToastKind::Error, message, None, cx);
        }
    }

    /// Reports a finished destructive or long operation in the status bar and a toast.
    pub(crate) fn report_success(&mut self, status: &str, cx: &mut Context<Self>) {
        self.set_status(status, cx);
        self.push_toast(notifications::ToastKind::Success, status.into(), None, cx);
    }

    /// Adds a toast unless the same one is showing. Errors stay until closed; the
    /// rest leave when a timer task fires, with nothing repainting meanwhile.
    pub(crate) fn push_toast(
        &mut self,
        kind: notifications::ToastKind,
        message: String,
        thread: Option<String>,
        cx: &mut Context<Self>,
    ) {
        if self
            .toasts
            .iter()
            .any(|toast| toast.kind == kind && toast.message == message && toast.thread == thread)
        {
            return;
        }
        self.toast_id += 1;
        let id = self.toast_id;
        self.toasts.push(notifications::Toast {
            id,
            kind,
            message,
            thread,
        });
        if self.toasts.len() > notifications::MAX_TOASTS {
            self.toasts.remove(0);
        }
        if kind != notifications::ToastKind::Error {
            cx.spawn(async move |app, cx| {
                cx.background_executor()
                    .timer(notifications::TOAST_LIFETIME)
                    .await;
                let _ = app.update(cx, |app, cx| app.dismiss_toast(id, cx));
            })
            .detach();
        }
        cx.notify();
    }

    pub(crate) fn dismiss_toast(&mut self, id: u64, cx: &mut Context<Self>) {
        self.toasts.retain(|toast| toast.id != id);
        cx.notify();
    }

    pub(crate) fn clear_notification_badge(&mut self, cx: &mut Context<Self>) {
        self.notification_badge.clear();
        for tag in self.notification_platform.invalidate() {
            cx.dismiss_system_notification(&tag);
        }
        self.notification_platform.badge(0);
    }

    pub(crate) fn clear_notifications(&mut self, cx: &mut Context<Self>) {
        self.clear_notification_badge(cx);
        self.notification_tracker.clear();
        self.toasts.retain(|toast| toast.thread.is_none());
    }

    pub(crate) fn set_notification_preferences(
        &mut self,
        preferences: notifications::Preferences,
        cx: &mut Context<Self>,
    ) {
        let mode_changed = self.notification_preferences.mode != preferences.mode;
        self.notification_preferences = preferences;
        if mode_changed || !preferences.mode.desktop() {
            self.clear_notification_badge(cx);
        }
        if !preferences.in_app {
            self.toasts.retain(|toast| toast.thread.is_none());
        }
        if let Some(state) = &mut self.saved_window {
            state.notifications = preferences;
            // Tiny device-local settings write; ordered so rapid toggles cannot
            // persist an older preference after a newer one.
            t3code_desktop_gpui::window_state::save(*state);
        }
        cx.notify();
    }

    /// Applies a live shell event and routes visual alerts separately from sound.
    pub(crate) fn apply_shell_item(
        &mut self,
        item: shell::ShellItem,
        cx: &mut Context<Self>,
    ) -> Result<(), &'static str> {
        let id = match &item {
            shell::ShellItem::ThreadUpserted { thread, .. } => Some(thread.id.clone()),
            _ => None,
        };
        let removed = matches!(&item, shell::ShellItem::ThreadRemoved { .. });
        let baseline =
            matches!(&item, shell::ShellItem::Snapshot { .. }) || !self.shell.synchronized;
        let checkout = self.checkout_context();
        let branch = self.selected_thread_branch();
        self.shell.apply(item)?;
        if self.checkout_context() != checkout || self.selected_thread_branch() != branch {
            self.invalidate_checkout_context(cx);
            self.set_status(
                "Checkout changed on the server; reload files in the selected checkout",
                cx,
            );
        }
        if baseline {
            self.notification_tracker
                .baseline(self.shell.threads.iter());
            return Ok(());
        }
        if removed {
            self.notification_tracker.retain(self.shell.threads.iter());
        }
        let Some(next) = id.and_then(|id| self.shell.threads.iter().find(|thread| thread.id == id))
        else {
            return Ok(());
        };
        let Some(event) = self.notification_tracker.observe(next) else {
            return Ok(());
        };
        let selected = self.selected_thread.as_deref() == Some(next.id.as_str());
        let mut preferences = self.notification_preferences;
        preferences.mode = self.notification_platform.effect_mode(preferences.mode);
        let route = notifications::decision(preferences, selected, self.window_active);
        let (id, title) = (next.id.clone(), next.title.clone());
        if route.sound {
            self.notification_platform
                .play(event, self.notification_preferences.mode);
        }
        if route.desktop {
            let scope = self.environment_scope.clone();
            let thread_id = id.clone();
            let body = title.clone();
            let delivery = self.notification_platform.begin_delivery(&scope, &id);
            cx.spawn(async move |app, cx| {
                use t3code_desktop_gpui::notification_platform::DeliveryOutcome;
                let outcome = delivery.send(event.title().into(), body).await;
                let _ = app.update(cx, |app, cx| {
                    app.notification_platform
                        .record_receipt(&delivery.tag, outcome);
                    let valid = delivery.is_current()
                        && app.environment_scope == scope
                        && app.shell.synchronized
                        && app.notification_preferences.mode.desktop()
                        && app
                            .shell
                            .threads
                            .iter()
                            .any(|thread| thread.id == thread_id);
                    if outcome == DeliveryOutcome::Accepted && valid {
                        let tag = notifications::tag(&scope, &thread_id);
                        if app.notification_badge.insert(tag) {
                            app.notification_platform
                                .badge(app.notification_badge.len());
                        }
                    } else {
                        cx.dismiss_system_notification(&delivery.tag);
                    }
                });
            })
            .detach();
        }
        if route.toast {
            let kind = match event {
                notifications::Event::TurnCompleted => notifications::ToastKind::Success,
                notifications::Event::ThreadFailed => notifications::ToastKind::Error,
                _ => notifications::ToastKind::Info,
            };
            self.push_toast(kind, format!("{} · {title}", event.title()), Some(id), cx);
        }
        Ok(())
    }

    /// Selects the thread a clicked desktop notification or toast names.
    pub(crate) fn open_notified_thread(
        &mut self,
        tag: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(id) = notifications::thread_from_tag(tag, &self.environment_scope) else {
            return;
        };
        if self.shell.synchronized && self.shell.threads.iter().any(|thread| thread.id == id) {
            let id = id.to_owned();
            self.select_thread(&id, window, cx);
            self.clear_notification_badge(cx);
            window.activate_window();
            cx.notify();
        }
    }
}
