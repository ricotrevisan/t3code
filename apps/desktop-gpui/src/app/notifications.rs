//! Status line, toasts, and desktop notifications.

use crate::NativeApp;
use gpui_kit::{Context, Window};
use t3code_desktop_gpui::{notifications, shell};

impl NativeApp {
    /// Plain status is neutral information, never inferred from message wording.
    pub(crate) fn set_status(&mut self, status: impl Into<String>, cx: &mut Context<Self>) {
        self.report_outcome(notifications::Outcome::Info, status, cx);
    }

    pub(crate) fn report_error(&mut self, status: impl Into<String>, cx: &mut Context<Self>) {
        self.report_outcome(notifications::Outcome::Error, status, cx);
    }

    /// Local missing-target validation is owned by its condition, not by the
    /// wording of an arbitrary server error.
    pub(crate) fn has_validation_thread(&self) -> bool {
        self.selected_thread.as_ref().is_some_and(|id| {
            self.shell
                .threads
                .iter()
                .chain(self.pending_threads.iter().map(|draft| &draft.thread))
                .any(|thread| &thread.id == id)
        })
    }

    pub(crate) fn report_composer_validation(&mut self, error: &str, cx: &mut Context<Self>) {
        if self.has_validation_thread() {
            self.report_error(error, cx);
        } else {
            self.report_thread_required(cx);
        }
    }

    pub(crate) fn report_thread_required(&mut self, cx: &mut Context<Self>) {
        self.thread_required_status = true;
        self.status = "Select a thread first".into();
        self.status_kind = notifications::Outcome::Error;
        self.toast_id += 1;
        if let Some(toast) = self
            .toasts
            .iter_mut()
            .find(|toast| Some(toast.id) == self.thread_required_toast)
        {
            toast.count = toast.count.saturating_add(1);
        } else {
            self.thread_required_toast = Some(self.toast_id);
            self.toasts.push(notifications::Toast {
                id: self.toast_id,
                kind: notifications::ToastKind::Error,
                count: 1,
                message: self.status.clone(),
                thread: None,
            });
        }
        cx.notify();
    }

    pub(crate) fn clear_thread_required(&mut self, cx: &mut Context<Self>) {
        if !self.has_validation_thread() {
            return;
        }
        if let Some(id) = self.thread_required_toast.take() {
            self.dismiss_toast(id, cx);
            if self.thread_required_status {
                self.set_status("Thread selected", cx);
            }
        }
    }

    pub(crate) fn report_progress(&mut self, status: impl Into<String>, cx: &mut Context<Self>) {
        self.report_outcome(notifications::Outcome::Progress, status, cx);
    }

    pub(crate) fn report_success(&mut self, status: &str, cx: &mut Context<Self>) {
        self.report_outcome(notifications::Outcome::Success, status, cx);
    }

    /// Typed seam for helpers that carry outcomes from multiple asynchronous branches.
    pub(crate) fn report_outcome(
        &mut self,
        outcome: notifications::Outcome,
        status: impl Into<String>,
        cx: &mut Context<Self>,
    ) {
        self.thread_required_status = false;
        self.status = notifications::cap_message(status.into());
        self.status_kind = outcome;
        let kind = match outcome {
            notifications::Outcome::Error => notifications::ToastKind::Error,
            notifications::Outcome::Success => notifications::ToastKind::Success,
            notifications::Outcome::Info | notifications::Outcome::Progress => return,
        };
        self.push_toast(kind, self.status.clone(), None, cx);
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
        self.toast_id += 1;
        let id = self.toast_id;
        // Condition-owned validation never coalesces with durable RPC failures,
        // even when a server happens to use exactly the same wording.
        let condition = self.thread_required_toast.and_then(|id| {
            self.toasts
                .iter()
                .position(|toast| toast.id == id)
                .map(|index| self.toasts.remove(index))
        });
        notifications::retain_toast(
            &mut self.toasts,
            notifications::Toast {
                id,
                kind,
                count: 1,
                message,
                thread,
            },
        );
        if let Some(condition) = condition {
            self.toasts.push(condition);
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
        self.sync_environment_badge();
        self.clear_environment_alerts(cx);
    }

    pub(crate) fn clear_notifications(&mut self, cx: &mut Context<Self>) {
        self.clear_notification_badge(cx);
        self.notification_tracker.clear();
        self.toasts.retain(|toast| toast.thread.is_none());
    }

    pub(crate) fn refresh_notification_authorization(&mut self, cx: &mut Context<Self>) {
        if self.notification_platform.authorization_request_pending() {
            return;
        }
        let receiver = self.notification_platform.probe_authorization();
        let authorization_epoch = self.notification_platform.authorization_epoch();
        let generation = self.generation;
        // A later delivery must win over a probe begun before its permission prompt.
        let deliveries = self.notification_platform.delivery_epoch();
        cx.spawn(async move |app, cx| {
            if let Ok(authorization) = receiver.recv().await {
                let _ = app.update(cx, |app, cx| {
                    if app.generation == generation
                        && app.notification_platform.authorization_epoch() == authorization_epoch
                        && app.notification_platform.delivery_epoch() == deliveries
                    {
                        app.notification_platform
                            .record_authorization(authorization);
                        cx.notify();
                    }
                });
            }
        })
        .detach();
    }

    pub(crate) fn set_notification_preferences(
        &mut self,
        preferences: notifications::Preferences,
        cx: &mut Context<Self>,
    ) {
        if preferences.mode.desktop() && preferences.mode != self.notification_preferences.mode {
            self.clear_notification_badge(cx);
            let receiver = self.notification_platform.request_authorization();
            let epoch = self.notification_platform.authorization_epoch();
            cx.spawn(async move |app, cx| {
                let authorization = receiver
                    .recv()
                    .await
                    .unwrap_or(t3code_desktop_gpui::notification_platform::Authorization::Failed);
                let _ = app.update(cx, |app, cx| {
                    if app.notification_platform.authorization_epoch() != epoch {
                        return;
                    }
                    app.notification_platform
                        .record_authorization(authorization);
                    if authorization
                        == t3code_desktop_gpui::notification_platform::Authorization::Authorized
                    {
                        app.commit_notification_preferences(preferences, cx);
                    }
                    cx.notify();
                });
            })
            .detach();
        } else {
            // A newer choice invalidates an outstanding permission request.
            self.notification_platform.cancel_authorization_request();
            self.refresh_notification_authorization(cx);
            self.commit_notification_preferences(preferences, cx);
        }
        cx.notify();
    }

    fn commit_notification_preferences(
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
        let synchronized = matches!(&item, shell::ShellItem::Synchronized);
        let baseline =
            matches!(&item, shell::ShellItem::Snapshot { .. }) || !self.shell.synchronized;
        let checkout = self.checkout_context();
        let branch = self.selected_thread_branch();
        self.shell.apply(item)?;
        if synchronized {
            self.connection_phase = if self.sessions.selected().is_none() || self.environment_identity_verified {
                t3code_desktop_gpui::connection::Phase::Ready
            } else {
                t3code_desktop_gpui::connection::Phase::Syncing
            };
        }
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
        let selected = self.environment_is_selected()
            && self.selected_thread.as_deref() == Some(next.id.as_str());
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
                            app.sync_environment_badge();
                        }
                    } else {
                        cx.dismiss_system_notification(&delivery.tag);
                        if delivery.is_current()
                            && app.environment_scope == scope
                            && !matches!(
                                outcome,
                                DeliveryOutcome::Accepted | DeliveryOutcome::Invalidated
                            )
                        {
                            app.report_error(app.notification_state_text(), cx);
                        }
                    }
                    cx.notify();
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
            if self.environment_is_selected() {
                self.push_toast(kind, format!("{} · {title}", event.title()), Some(id), cx);
            } else {
                self.background_environment_alert(&id, format!("{} · {title}", event.title()), cx);
            }
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
