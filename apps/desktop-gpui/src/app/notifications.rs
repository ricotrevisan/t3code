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

    /// Applies a shell item, then sends a desktop notification for the thread
    /// event it raises on a thread the user is not looking at. With the window
    /// focused, a toast offers the same thread too.
    pub(crate) fn apply_shell_item(
        &mut self,
        item: shell::ShellItem,
        cx: &mut Context<Self>,
    ) -> Result<(), &'static str> {
        let prior = match &item {
            shell::ShellItem::ThreadUpserted { thread, .. } => self
                .shell
                .threads
                .iter()
                .find(|known| known.id == thread.id)
                .cloned(),
            _ => None,
        };
        let checkout = self.checkout_context();
        let branch = self.selected_thread_branch();
        self.shell.apply(item)?;
        if self.checkout_context() != checkout || self.selected_thread_branch() != branch {
            self.invalidate_checkout_context(cx);
            self.set_status("Checkout changed on the server; reload files in the selected checkout", cx);
        }
        let Some(prior) = prior else {
            return Ok(());
        };
        let Some(next) = self
            .shell
            .threads
            .iter()
            .find(|thread| thread.id == prior.id)
        else {
            return Ok(());
        };
        let selected = self.selected_thread.as_deref() == Some(next.id.as_str());
        let Some(event) = notifications::notification(&prior, next, selected, self.window_active)
        else {
            return Ok(());
        };
        let (id, title) = (next.id.clone(), next.title.clone());
        cx.show_system_notification(gpui_kit::SystemNotification {
            tag: notifications::tag(&id).into(),
            title: event.title().into(),
            body: title.clone().into(),
            actions: Vec::new(),
        });
        if self.window_active {
            let kind = if event == notifications::Event::TurnCompleted {
                notifications::ToastKind::Success
            } else {
                notifications::ToastKind::Info
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
        let Some(id) = notifications::thread_from_tag(tag) else {
            return;
        };
        if self.shell.threads.iter().any(|thread| thread.id == id) {
            let id = id.to_owned();
            self.select_thread(&id, window, cx);
            window.activate_window();
            cx.notify();
        }
    }
}
