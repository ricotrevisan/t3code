//! Scoped selection and PTY lifecycle operations shared by terminal entry points.
use super::{Screen, Size, Tab};
use crate::NativeApp;
use gpui_kit::*;
use serde_json::{Value, json};
use t3code_desktop_gpui::connection;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TerminalCapture {
    pub scope: String,
    pub thread: String,
    pub terminal_id: String,
    pub terminal_label: String,
    pub line_start: usize,
    pub line_end: usize,
    pub text: String,
}
impl NativeApp {
    pub(super) fn terminal_visible_ids(&self) -> Vec<String> {
        let Some(tab) = self.terminal_active() else {
            return Vec::new();
        };
        self.terminal_drawer
            .split
            .as_ref()
            .filter(|(scope, thread, ids)| {
                scope == &tab.scope && thread == &tab.thread && ids.contains(&tab.id)
            })
            .map(|(_, _, ids)| ids.clone())
            .unwrap_or_else(|| vec![tab.id.clone()])
    }
    fn terminal_discovered_title(&self, thread: &str, id: &str) -> String {
        let normalized = id.to_ascii_lowercase();
        if let Some(suffix) = normalized
            .strip_prefix("term-")
            .or_else(|| normalized.strip_prefix("terminal-"))
            && !suffix.is_empty()
            && suffix.bytes().all(|byte| byte.is_ascii_digit())
        {
            return format!("Terminal {suffix}");
        }
        self.terminal_default_title(thread)
    }
    pub(super) fn terminal_default_title(&self, thread: &str) -> String {
        (1..=17)
            .map(|n| format!("Terminal {n}"))
            .find(|title| {
                !self.terminal_drawer.tabs.iter().any(|tab| {
                    tab.scope == self.environment_scope
                        && tab.thread == thread
                        && &tab.title == title
                })
            })
            .unwrap()
    }
    pub(super) fn terminal_make_room(&mut self) -> bool {
        if self.terminal_drawer.tabs.len() < 16 {
            return true;
        }
        if let Some(index) = self.terminal_drawer.tabs.iter().position(|tab| {
            !tab.busy
                && tab.attached.is_none()
                && (tab.scope != self.environment_scope
                    || Some(&tab.thread) != self.selected_thread.as_ref())
        }) {
            self.terminal_drawer.tabs.remove(index);
            true
        } else {
            false
        }
    }
    /// Retired keys keep their original command channel until cancellation is queued.
    /// At most 16 recent screens are retained; only the visible thread receives output.
    pub(super) fn terminal_retire(&mut self, _cx: &mut Context<Self>) {
        if self.terminal_drawer.confirm.as_ref().is_some_and(|owner| {
            self.terminal_active().is_none_or(|tab| {
                owner
                    != &(
                        tab.scope.clone(),
                        tab.thread.clone(),
                        tab.id.clone(),
                        self.generation,
                        tab.epoch,
                    )
            })
        }) {
            self.terminal_drawer.confirm = None;
        }
        if self
            .terminal_drawer
            .restart_confirm
            .as_ref()
            .is_some_and(|owner| {
                self.terminal_active().is_none_or(|tab| {
                    owner
                        != &(
                            tab.scope.clone(),
                            tab.thread.clone(),
                            tab.id.clone(),
                            self.generation,
                            tab.epoch,
                        )
                })
            })
        {
            self.terminal_drawer.restart_confirm = None;
        }
        if self
            .terminal_drawer
            .metadata
            .as_ref()
            .is_some_and(|(_, scope, generation)| {
                scope != &self.environment_scope
                    || generation != &self.generation
                    || !self.terminal_drawer.open
            })
            && let Some((key, _, _)) = self.terminal_drawer.metadata.take()
            && let Some(sender) = self.terminal_drawer.metadata_sender.take()
        {
            self.terminal_drawer.retired.push((sender, key));
        }
        let visible_ids = self.terminal_visible_ids();
        let mut retained = Vec::new();
        for mut tab in self.terminal_drawer.tabs.drain(..) {
            let visible = self.terminal_drawer.open
                && tab.scope == self.environment_scope
                && Some(&tab.thread) == self.selected_thread.as_ref()
                && visible_ids.contains(&tab.id);
            let keep = (tab.exists || tab.busy) && retained.len() < 16;
            if (!visible
                || !keep
                || tab
                    .attached
                    .is_some_and(|generation| generation != self.generation))
                && tab.attached.take().is_some()
                && let Some(sender) = &tab.subscription_sender
            {
                self.terminal_drawer
                    .retired
                    .push((sender.clone(), tab.stream.clone()));
                tab.stream_ready = false;
            }
            if keep {
                retained.push(tab);
            }
        }
        self.terminal_drawer.tabs = retained;
        if let Some((scope, thread, ids)) = &mut self.terminal_drawer.split {
            ids.retain(|id| {
                self.terminal_drawer
                    .tabs
                    .iter()
                    .any(|tab| &tab.scope == scope && &tab.thread == thread && &tab.id == id)
            });
            if ids.len() < 2 {
                self.terminal_drawer.split = None;
            }
        }
        self.terminal_drawer.retired.retain(|(sender, key)| {
            matches!(
                sender.try_send(connection::Command::Unsubscribe(key.clone())),
                Err(async_channel::TrySendError::Full(_))
            )
        });
    }
    pub(super) fn terminal_split(&mut self, cx: &mut Context<Self>) {
        let Some(tab) = self.terminal_active() else {
            return;
        };
        let scope = tab.scope.clone();
        let thread = tab.thread.clone();
        let mut ids = self
            .terminal_drawer
            .split
            .as_ref()
            .filter(|(s, t, ids)| s == &scope && t == &thread && ids.contains(&tab.id))
            .map(|(_, _, ids)| ids.clone())
            .unwrap_or_else(|| vec![tab.id.clone()]);
        ids.retain(|id| {
            self.terminal_drawer
                .tabs
                .iter()
                .any(|tab| tab.scope == scope && tab.thread == thread && &tab.id == id)
        });
        if ids.len() >= 4 {
            return;
        }
        let previous = self.terminal_drawer.active.clone();
        self.new_terminal(cx);
        if self.terminal_drawer.active != previous
            && let Some(id) = &self.terminal_drawer.active
        {
            ids.push(id.clone());
            self.terminal_drawer.split = Some((scope, thread, ids));
        }
        cx.notify();
    }
    pub(super) fn terminal_pane_current(&self, owner: &(String, String, String, u64, u64)) -> bool {
        owner.0 == self.environment_scope
            && Some(&owner.1) == self.selected_thread.as_ref()
            && owner.3 == self.generation
            && self.terminal_drawer.tabs.iter().any(|tab| {
                tab.scope == owner.0
                    && tab.thread == owner.1
                    && tab.id == owner.2
                    && tab.epoch == owner.4
            })
    }
    pub(super) fn terminal_menu_owner(&self) -> Option<(String, String, String, u64)> {
        let tab = self.terminal_active()?;
        Some((
            tab.scope.clone(),
            tab.thread.clone(),
            tab.id.clone(),
            self.generation,
        ))
    }
    pub(super) fn terminal_menu_current(
        &self,
        owner: &Option<(String, String, String, u64)>,
    ) -> bool {
        owner.is_some() && &self.terminal_menu_owner() == owner
    }
    /// A shell block can run only in the currently selected, already attached PTY.
    /// Returning its id is readiness, not an execution receipt.
    pub(crate) fn terminal_run_readiness(
        &self,
        scope: &str,
        thread: &str,
    ) -> Result<String, String> {
        if scope != self.environment_scope || Some(thread) != self.selected_thread.as_deref() {
            return Err("Select this message's environment and thread first".into());
        }
        if !self.shell.synchronized || !self.connection_phase.ready() || self.commands.is_none() {
            return Err("Connect and synchronize before running a shell command".into());
        }
        self.terminal_active()
            .filter(|tab| {
                tab.exists && tab.stream_ready && !tab.busy && tab.attached == Some(self.generation)
            })
            .map(|tab| tab.id.clone())
            .ok_or_else(|| "Open an attached terminal in this thread first".into())
    }
    /// Explicit click-to-run only: no opening, restarting, command retention or replay.
    /// Ok means queued; a failed server receipt remains a scoped terminal error.
    pub(crate) fn terminal_run_scoped(
        &mut self,
        scope: &str,
        thread: &str,
        command: &str,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        let command = command.trim_end_matches(['\r', '\n']);
        if command.is_empty() || command.contains('\0') {
            return Err("Empty or invalid shell command".into());
        }
        let id = self.terminal_run_readiness(scope, thread)?;
        let data = format!(
            "{}\r",
            self.terminal_active().unwrap().screen.paste(command)
        );
        if self.terminal_enqueue(&id, "terminal.write", json!({"data":data}), cx) {
            Ok(())
        } else {
            Err("Terminal command could not be queued; nothing will be replayed".into())
        }
    }
    pub(crate) fn terminal_capture(&self) -> Option<TerminalCapture> {
        let tab = self.terminal_active()?;
        let (line_start, line_end, text) = tab.screen.selection_context()?;
        Some(TerminalCapture {
            scope: tab.scope.clone(),
            thread: tab.thread.clone(),
            terminal_id: tab.id.clone(),
            terminal_label: tab.title.clone(),
            line_start,
            line_end,
            text,
        })
    }
    /// Commit the menu-captured immutable selection to C-owned durable draft state.
    pub(super) fn terminal_intake_capture(
        &mut self,
        capture: &TerminalCapture,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let result = t3code_desktop_gpui::draft_context::DraftContextRecord::terminal(
            &format!("Terminal: {}", capture.terminal_label),
            &capture.terminal_id,
            &capture.terminal_label,
            capture.line_start as u64,
            capture.line_end as u64,
            &capture.text,
        )
        .and_then(|record| {
            self.intake_draft_context(&capture.scope, &capture.thread, record, window, cx)
        });
        match result {
            Ok(_) => self
                .composer
                .update(cx, |input, cx| input.focus(window, cx)),
            Err(error) => self.report_error(error, cx),
        }
    }
    pub(super) fn terminal_find(&mut self, reverse: bool, cx: &mut Context<Self>) {
        let query = self.terminal_drawer.search.read(cx).value().to_string();
        let Some(id) = self.terminal_active().map(|tab| tab.id.clone()) else {
            return;
        };
        let after = self
            .terminal_drawer
            .search_result
            .as_ref()
            .filter(|(previous_id, previous_query, _, _)| {
                previous_id == &id && previous_query == &query
            })
            .map(|(_, _, row, col)| (*row, *col));
        let tab = self
            .terminal_drawer
            .tabs
            .iter_mut()
            .find(|tab| {
                tab.id == id
                    && tab.scope == self.environment_scope
                    && Some(&tab.thread) == self.selected_thread.as_ref()
            })
            .unwrap();
        let result = tab.screen.search(&query, after, reverse);
        self.terminal_drawer.search_missing = !query.is_empty() && result.is_none();
        self.terminal_drawer.search_result = result.map(|(row, col)| (id, query, row, col));
        cx.notify();
    }
    pub(super) fn terminal_paste(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(item) = cx.read_from_clipboard() {
            <Self as EntityInputHandler>::paste(self, item, window, cx);
        }
    }
    pub(super) fn terminal_rename(&mut self, id: &str, title: String, cx: &mut Context<Self>) {
        let Some(tab) = self.terminal_drawer.tabs.iter_mut().find(|tab| {
            tab.id == id
                && tab.scope == self.environment_scope
                && Some(&tab.thread) == self.selected_thread.as_ref()
        }) else {
            return;
        };
        if let Err(error) = self
            .terminal_drawer
            .names
            .set(&tab.scope, &tab.thread, id, &title)
        {
            self.terminal_drawer.local_error =
                Some(format!("Terminal name was not saved: {error}"));
        } else {
            tab.title = title;
            tab.custom_title = true;
            self.terminal_drawer.local_error = None;
        }
        cx.notify();
    }
    pub(super) fn terminal_restart(&mut self, id: &str, confirmed: bool, cx: &mut Context<Self>) {
        let Some(tab) = self.terminal_active().filter(|tab| tab.id == id) else {
            return;
        };
        if tab.launch.is_null() {
            self.terminal_drawer.local_error = Some("Restart unavailable: original launch environment/provider configuration is unknown for this restored session.".into());
            cx.notify();
            return;
        }
        let owner = (
            tab.scope.clone(),
            tab.thread.clone(),
            tab.id.clone(),
            self.generation,
            tab.epoch,
        );
        if confirmed && self.terminal_drawer.restart_confirm.as_ref() != Some(&owner) {
            self.terminal_drawer.restart_confirm = None;
            cx.notify();
            return;
        }
        if !confirmed && tab.running != Some(false) {
            self.terminal_drawer.restart_confirm = Some(owner);
            cx.notify();
            return;
        }
        let mut payload = tab.launch.clone();
        payload["cols"] = tab.screen.size.cols.into();
        payload["rows"] = tab.screen.size.rows.into();
        self.terminal_drawer.restart_confirm = None;
        self.terminal_rpc(id, "terminal.restart", payload, cx);
    }
    /// Explicit discovery never creates or restarts a server PTY. Its global metadata
    /// stream is filtered before materializing tabs, then terminal_synchronize attaches.
    pub(super) fn terminal_discover(&mut self, cx: &mut Context<Self>) {
        self.terminal_retire(cx);
        if !self.terminal_drawer.retired.is_empty() {
            self.terminal_drawer.local_error = Some(
                "Waiting to unsubscribe retired terminal streams; retry session discovery.".into(),
            );
            cx.notify();
            return;
        }
        if !self.shell.synchronized || !self.connection_phase.ready() {
            self.terminal_drawer.pending_open = None;
            return;
        }
        let Some(sender) = &self.commands else {
            self.terminal_drawer.pending_open = None;
            return;
        };
        // Transport replacement with the same key interrupts its previous stream.
        // A full channel leaves the existing owner intact, so retry cannot orphan it.
        let key = self
            .terminal_drawer
            .metadata
            .as_ref()
            .map(|(key, _, _)| key.clone())
            .unwrap_or_else(|| format!("drawer-metadata-{}", uuid::Uuid::new_v4()));
        let command = connection::Command::Subscribe {
            key: key.clone(),
            tag: "subscribeTerminalMetadata".into(),
            payload: json!({}),
        };
        match sender.try_send(command) {
            Ok(()) => {
                self.terminal_drawer.metadata =
                    Some((key, self.environment_scope.clone(), self.generation));
                self.terminal_drawer.metadata_sender = Some(sender.clone());
                self.terminal_drawer.local_error = None;
            }
            Err(error) => {
                self.terminal_drawer.local_error =
                    Some(format!("Session discovery failed: {error}"))
            }
        }
        cx.notify();
    }
    pub(super) fn terminal_metadata_update(
        &mut self,
        key: &str,
        item: &Result<Value, String>,
        cx: &mut Context<Self>,
    ) -> bool {
        if !key.starts_with("drawer-metadata-") {
            return false;
        }
        let Some((expected, scope, generation)) = &self.terminal_drawer.metadata else {
            return true;
        };
        if key != expected || scope != &self.environment_scope || generation != &self.generation {
            return true;
        }
        match item {
            Err(error) => {
                self.terminal_drawer.local_error =
                    Some(format!("Session discovery failed: {error}"))
            }
            Ok(value) => match value["type"].as_str() {
                Some("snapshot") => {
                    if let Some(terminals) = value["terminals"].as_array() {
                        for tab in &mut self.terminal_drawer.tabs {
                            if tab.scope == self.environment_scope
                                && !tab.busy
                                && tab.exists
                                && !terminals.iter().any(|summary| {
                                    summary["threadId"] == tab.thread
                                        && summary["terminalId"] == tab.id
                                })
                            {
                                tab.exists = false;
                                tab.stream_ready = false;
                                tab.running = Some(false);
                                tab.status = "closed".into();
                            }
                        }
                        self.terminal_retire(cx);
                        self.terminal_drawer.local_error = None;
                        for summary in terminals {
                            self.terminal_discover_summary(summary);
                        }
                    } else {
                        self.terminal_drawer.local_error =
                            Some("Invalid terminal metadata snapshot".into());
                    }
                }
                Some("upsert") => self.terminal_discover_summary(&value["terminal"]),
                Some("remove") => {
                    let id = value["terminalId"].as_str();
                    let thread = value["threadId"].as_str();
                    for tab in &mut self.terminal_drawer.tabs {
                        if Some(tab.id.as_str()) == id
                            && Some(tab.thread.as_str()) == thread
                            && tab.scope == self.environment_scope
                        {
                            tab.exists = false;
                            tab.stream_ready = false;
                            tab.status = "closed".into();
                        }
                    }
                }
                _ => {
                    self.terminal_drawer.local_error =
                        Some("Invalid terminal metadata event".into())
                }
            },
        }
        if let Ok(value) = item
            && value["type"] == "snapshot"
            && value["terminals"].is_array()
            && let Some((scope, thread, generation)) = self.terminal_drawer.pending_open.take()
            && scope == self.environment_scope
            && Some(&thread) == self.selected_thread.as_ref()
            && generation == self.generation
            && self.terminal_drawer.open
        {
            let recovered = self
                .terminal_drawer
                .tabs
                .iter()
                .find(|tab| tab.scope == scope && tab.thread == thread && tab.exists)
                .map(|tab| tab.id.clone());
            if let Some(id) = recovered {
                self.terminal_drawer.active = Some(id);
            } else {
                self.new_terminal(cx);
            }
        } else if item.is_err() {
            self.terminal_drawer.pending_open = None;
        }
        self.terminal_retire(cx);
        cx.notify();
        true
    }
    fn terminal_discover_summary(&mut self, summary: &Value) {
        let (Some(thread), Some(id), Some(cwd), Some(status)) = (
            summary["threadId"].as_str(),
            summary["terminalId"].as_str(),
            summary["cwd"].as_str(),
            summary["status"].as_str(),
        ) else {
            return;
        };
        // Never import another thread's shells into the active drawer.
        if Some(thread) != self.selected_thread.as_deref() || id.is_empty() || cwd.is_empty() {
            return;
        }
        let title = self
            .terminal_drawer
            .names
            .get(&self.environment_scope, thread, id)
            .map(str::to_owned)
            .or_else(|| {
                summary["label"]
                    .as_str()
                    .filter(|s| !s.is_empty() && *s != id)
                    .map(str::to_owned)
            })
            .unwrap_or_else(|| self.terminal_discovered_title(thread, id));
        if let Some(tab) =
            self.terminal_drawer.tabs.iter_mut().find(|tab| {
                tab.scope == self.environment_scope && tab.thread == thread && tab.id == id
            })
        {
            if tab.custom_title
                || summary["label"]
                    .as_str()
                    .is_some_and(|s| !s.is_empty() && s != id)
            {
                tab.title = title;
            }
            tab.running = summary["hasRunningSubprocess"].as_bool();
            tab.status = status.into();
            tab.cwd = cwd.into();
            // Keep an existing authoritative launch record unchanged.
            if !tab.exists {
                tab.exists = true;
                tab.attached = None;
                tab.stream_ready = false;
                tab.sequence = None;
            }
            return;
        }
        // Total screen retention is bounded, including discovery (not just New).
        if !self.terminal_make_room() {
            self.terminal_drawer.local_error = Some("Showing at most 16 terminal sessions. Close one, then restore sessions to see more.".into());
            return;
        }
        if !self.terminal_drawer.retired.is_empty() {
            return;
        }
        self.terminal_drawer.tabs.push(Tab {
            id: id.into(),
            thread: thread.into(),
            scope: self.environment_scope.clone(),
            stream: format!("drawer-restored-{}", uuid::Uuid::new_v4()),
            cwd: cwd.into(),
            title,
            custom_title: self
                .terminal_drawer
                .names
                .get(&self.environment_scope, thread, id)
                .is_some(),
            launch: Value::Null,
            screen: Screen::new(Size::new(80, 12)),
            bounds: None,
            exists: true,
            busy: false,
            running: summary["hasRunningSubprocess"].as_bool(),
            status: status.into(),
            error: None,
            generation: self.generation,
            attached: None,
            subscription_sender: self.commands.clone(),
            stream_ready: false,
            sequence: None,
            epoch: 0,
        });
    }
}
