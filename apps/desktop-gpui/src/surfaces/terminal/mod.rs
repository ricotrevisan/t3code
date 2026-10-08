//! Interactive terminals are owned by environment + thread, never by mutable checkout forms.
use crate::NativeApp;
use gpui_kit::component::input::InputState;
use gpui_kit::{AppContext as _, Context, Entity, FocusHandle, Window};
use serde_json::{Value, json};
use t3code_desktop_gpui::{
    connection,
    terminal::{Screen, Size},
};

mod input;
pub(crate) mod keys;
mod names;
mod operations;
mod view;

pub(crate) struct Tab {
    pub id: String,
    pub stream: String,
    pub scope: String,
    pub thread: String,
    pub cwd: String,
    pub title: String,
    pub custom_title: bool,
    pub launch: Value,
    pub screen: Screen,
    pub exists: bool,
    pub busy: bool,
    pub running: Option<bool>,
    pub status: String,
    pub error: Option<String>,
    pub generation: u64,
    pub attached: Option<u64>,
    pub stream_ready: bool,
    pub sequence: Option<u64>,
    pub epoch: u64,
}
impl Tab {
    pub fn session(&self) -> Value {
        json!({"threadId":self.thread,"terminalId":self.id})
    }
    fn event(&mut self, value: Value) {
        let snapshot = matches!(
            value["type"].as_str(),
            Some("snapshot" | "restarted" | "started")
        );
        let body = if snapshot { &value["snapshot"] } else { &value };
        if snapshot
            && (body["threadId"].as_str().is_none()
                || body["terminalId"].as_str().is_none()
                || body["history"].as_str().is_none()
                || body["status"].as_str().is_none())
        {
            self.error = Some("Terminal attachment returned no valid snapshot; input is disabled. This server may not support terminal attachment.".into());
            self.stream_ready = false;
            return;
        }
        if body["threadId"] != self.thread || body["terminalId"] != self.id {
            return;
        }
        if let Some(sequence) = body["sequence"].as_u64() {
            if self
                .sequence
                .is_some_and(|old| sequence < old || (!snapshot && old == sequence))
            {
                return;
            }
            self.sequence = Some(sequence);
        }
        if snapshot {
            self.screen
                .reset(body["history"].as_str().unwrap_or_default());
            if let Some(cwd) = body["cwd"].as_str() {
                self.cwd = cwd.into();
                self.launch["cwd"] = cwd.into();
            }
            if body
                .as_object()
                .is_some_and(|body| body.contains_key("worktreePath"))
            {
                self.launch["worktreePath"] = body["worktreePath"].clone();
            }
            if !self.custom_title
                && let Some(label) = body["label"].as_str().filter(|label| !label.is_empty())
            {
                self.title = label.chars().take(128).collect();
            }
            self.status = body["status"].as_str().unwrap_or("unknown").into();
            self.exists = true;
            self.running = (self.status == "exited").then_some(false);
            self.error = None;
        } else {
            match value["type"].as_str() {
                Some("output") => self.screen.feed(value["data"].as_str().unwrap_or_default()),
                Some("activity") => {
                    self.running = value["hasRunningSubprocess"].as_bool();
                }
                Some("exited") => {
                    self.status = "exited".into();
                    self.running = Some(false);
                }
                Some("closed") => {
                    self.status = "closed".into();
                    self.exists = false;
                    self.running = Some(false);
                }
                Some("cleared") => self.screen.reset(""),
                Some("error") => self.error = value["message"].as_str().map(str::to_owned),
                _ => {}
            }
        }
    }
}
pub(crate) struct Drawer {
    pub open: bool,
    pub tabs: Vec<Tab>,
    pub active: Option<String>,
    pub focus: FocusHandle,
    pub rename: Entity<InputState>,
    pub renaming: Option<String>,
    pub confirm: Option<String>,
    pub restart_confirm: Option<String>,
    pub search: Entity<InputState>,
    pub searching: bool,
    pub search_result: Option<(String, String, i32, usize)>,
    pub search_missing: bool,
    names: names::Names,
    pub local_error: Option<String>,
    metadata: Option<(String, String, u64)>,
    pending_open: Option<(String, String, u64)>,
    pub height: f32,
    composition: Option<input::Composition>,
    pub drag: Option<(f32, f32)>,
    pub bounds: Option<gpui_kit::Bounds<gpui_kit::Pixels>>,
    pub cell_width: f32,
    pub line_height: f32,
}
impl Drawer {
    pub fn new(window: &mut Window, cx: &mut Context<NativeApp>) -> Self {
        let (names, local_error) = if cfg!(test) {
            (names::Names::default(), None)
        } else {
            match names::Names::load(
                t3code_desktop_gpui::drafts::default_path().with_file_name("terminal-names.json"),
            ) {
                Ok(names) => (names, None),
                Err(error) => (
                    names::Names::unavailable(),
                    Some(format!("Terminal names cannot be read: {error}")),
                ),
            }
        };
        Self {
            open: false,
            tabs: Vec::new(),
            active: None,
            focus: cx.focus_handle(),
            rename: cx.new(|cx| crate::single_line_input(window, cx)),
            renaming: None,
            confirm: None,
            restart_confirm: None,
            search: cx.new(|cx| crate::single_line_input(window, cx)),
            searching: false,
            search_result: None,
            search_missing: false,
            names,
            local_error,
            metadata: None,
            pending_open: None,
            height: crate::window_state::TERMINAL_HEIGHT.1,
            composition: None,
            drag: None,
            bounds: None,
            cell_width: 8.4,
            line_height: 20.,
        }
    }
}
impl NativeApp {
    pub(crate) fn toggle_terminal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.terminal_drawer.open = !self.terminal_drawer.open;
        if self.terminal_drawer.open {
            let id = self
                .terminal_drawer
                .tabs
                .iter()
                .find(|tab| {
                    tab.scope == self.environment_scope
                        && Some(&tab.thread) == self.selected_thread.as_ref()
                })
                .map(|tab| tab.id.clone());
            if let Some(id) = id {
                self.terminal_drawer.active = Some(id);
            } else if let Some(thread) = self.selected_thread.clone() {
                self.terminal_drawer.pending_open =
                    Some((self.environment_scope.clone(), thread, self.generation));
                self.terminal_discover(cx);
            }
            self.terminal_drawer.focus.focus(window, cx);
        } else {
            self.composer
                .update(cx, |input, cx| input.focus(window, cx));
        }
        cx.notify();
    }
    pub(crate) fn new_terminal(&mut self, cx: &mut Context<Self>) {
        self.terminal_drawer.pending_open = None;
        if !self.shell.synchronized || !self.connection_phase.ready() || self.commands.is_none() {
            self.set_status(
                "Terminal unavailable while disconnected or synchronizing",
                cx,
            );
            return;
        }
        let Some(thread) = self
            .shell
            .threads
            .iter()
            .find(|thread| Some(&thread.id) == self.selected_thread.as_ref())
        else {
            return;
        };
        let Some(project) = self
            .shell
            .projects
            .iter()
            .find(|project| project.id == thread.project_id)
        else {
            return;
        };
        let Some(cwd) = self.selected_workspace() else {
            return;
        };
        if self
            .terminal_drawer
            .tabs
            .iter()
            .filter(|tab| tab.scope == self.environment_scope && tab.thread == thread.id)
            .count()
            >= 16
        {
            self.set_status("Close a terminal before opening more (16 per thread)", cx);
            return;
        }
        let id = format!("drawer-{}", uuid::Uuid::new_v4());
        let stream = format!("{id}:stream:{}", uuid::Uuid::new_v4());
        let path = (cwd != project.workspace_root).then_some(cwd.clone());
        let mut env = json!({"T3CODE_PROJECT_ROOT":project.workspace_root});
        if let Some(path) = &path {
            env["T3CODE_WORKTREE_PATH"] = path.clone().into();
        }
        let payload = json!({"threadId":thread.id,"terminalId":id,"cwd":cwd,"worktreePath":path,"cols":80,"rows":12,"env":env});
        let tab = Tab {
            id: id.clone(),
            stream,
            scope: self.environment_scope.clone(),
            thread: thread.id.clone(),
            cwd,
            title: "Terminal".into(),
            custom_title: false,
            launch: payload.clone(),
            screen: Screen::new(Size::new(80, 12)),
            exists: false,
            busy: true,
            running: None,
            status: "starting".into(),
            error: None,
            generation: self.generation,
            attached: None,
            stream_ready: false,
            sequence: None,
            epoch: 0,
        };
        self.terminal_drawer.active = Some(id.clone());
        self.terminal_drawer.tabs.push(tab);
        let sender = self.commands.clone().unwrap();
        let generation = self.generation;
        let scope = self.environment_scope.clone();
        cx.spawn(async move |entity, cx| {
            let result = connection::call(&sender, "terminal.open", payload).await;
            let command = entity.update(cx, |app, cx| {
                let current = app.generation == generation && app.environment_scope == scope && app.connection_phase.ready() && app.shell.synchronized;
                let tab = app.terminal_drawer.tabs.iter_mut().find(|tab| tab.id == id)?;
                tab.busy = false;
                // Opening a gate is not proof of cancellation. Retain identity on a late receipt.
                tab.exists = result.is_ok() || !current;
                if !current { tab.error = Some("Connection changed during open; terminal state is unknown. Reconnect to its environment.".into()); cx.notify(); return None; }
                match result {
                    Ok(snapshot) => {
                        tab.event(json!({"type":"snapshot","snapshot":snapshot}));
                        tab.busy = true;
                        let command = connection::Command::Subscribe { key: tab.stream.clone(), tag: "terminal.attach".into(), payload: json!({"threadId":tab.thread,"terminalId":tab.id,"restartIfNotRunning":false}) };
                        cx.notify(); Some(command)
                    },
                    Err(error) => { tab.status = "error".into(); tab.error = Some(error); cx.notify(); None }
                }
            }).ok().flatten();
            if let Some(command) = command {
                let result = sender.send(command).await;
                let _ = entity.update(cx, |app, cx| {
                    if app.generation != generation || app.environment_scope != scope { return; }
                    if let Some(tab) = app.terminal_drawer.tabs.iter_mut().find(|tab| tab.id == id) {
                        tab.busy = false;
                        if result.is_ok() { tab.attached = Some(generation); }
                        else { tab.error = Some("Unable to attach terminal output".into()); }
                    }
                    cx.notify();
                });
            }
        }).detach();
        cx.notify();
    }
    /// Small hook for the shell subscription dispatcher. Replayed snapshots replace the grid;
    /// sequence and stream ownership prevent duplicates and late cross-environment chunks.
    #[allow(dead_code)] // connection dispatcher glue is outside this lane's ownership
    pub(crate) fn terminal_update(
        &mut self,
        stream: &str,
        item: Result<Value, String>,
        cx: &mut Context<Self>,
    ) {
        if self.terminal_metadata_update(stream, &item, cx) {
            return;
        }
        let Some(tab) = self.terminal_drawer.tabs.iter_mut().find(|tab| {
            tab.stream == stream
                && tab.scope == self.environment_scope
                && tab.attached == Some(self.generation)
        }) else {
            return;
        };
        match item {
            Ok(value) => {
                if value["type"] == "snapshot"
                    && value["snapshot"]["threadId"] == tab.thread
                    && value["snapshot"]["terminalId"] == tab.id
                {
                    tab.stream_ready = true;
                }
                tab.event(value);
            }
            Err(error) => {
                tab.error = Some(error);
                tab.stream_ready = false;
                tab.running = None;
            }
        }
        let id = tab.id.clone();
        let current_thread = Some(&tab.thread) == self.selected_thread.as_ref();
        let replies = tab.screen.take_replies();
        for reply in replies.into_iter().filter(|_| current_thread) {
            self.terminal_rpc(&id, "terminal.write", json!({"data":reply}), cx);
        }
        cx.notify();
    }
    /// Called from rendering, which is already invalidated by connection/shell updates.
    pub(crate) fn terminal_synchronize(&mut self, cx: &mut Context<Self>) {
        let ready =
            self.shell.synchronized && self.connection_phase.ready() && self.commands.is_some();
        for tab in &mut self.terminal_drawer.tabs {
            if tab.scope != self.environment_scope {
                continue;
            }
            if tab.generation != self.generation {
                tab.exists |= tab.busy;
                tab.busy = false;
                tab.epoch += 1;
                tab.generation = self.generation;
                tab.error =
                    Some("Connection changed; waiting for authoritative attach snapshot".into());
            }
            if !ready {
                tab.running = None;
                continue;
            }
            if !tab.exists || tab.busy || tab.attached == Some(self.generation) {
                continue;
            }
            tab.busy = true;
            tab.stream_ready = false;
            tab.generation = self.generation;
            tab.epoch += 1;
            tab.sequence = None;
            tab.stream = format!("{}:stream:{}", tab.id, uuid::Uuid::new_v4());
            let sender = self.commands.clone().unwrap();
            let command = connection::Command::Subscribe {
                key: tab.stream.clone(),
                tag: "terminal.attach".into(),
                payload: json!({"threadId":tab.thread,"terminalId":tab.id,"restartIfNotRunning":false}),
            };
            let stream = tab.stream.clone();
            let generation = self.generation;
            let scope = self.environment_scope.clone();
            cx.spawn(async move |entity, cx| {
                let result = sender.send(command).await;
                let _ = entity.update(cx, |app, cx| {
                    if app.generation != generation || app.environment_scope != scope {
                        return;
                    }
                    if let Some(tab) = app
                        .terminal_drawer
                        .tabs
                        .iter_mut()
                        .find(|tab| tab.stream == stream)
                    {
                        tab.busy = false;
                        if result.is_ok() {
                            tab.attached = Some(generation);
                        } else {
                            tab.error = Some("Unable to reattach terminal".into());
                        }
                    }
                    cx.notify();
                });
            })
            .detach();
        }
    }
    pub(crate) fn terminal_rpc(
        &mut self,
        id: &str,
        tag: &'static str,
        extra: Value,
        cx: &mut Context<Self>,
    ) {
        self.terminal_enqueue(id, tag, extra, cx);
    }
    pub(super) fn terminal_enqueue(
        &mut self,
        id: &str,
        tag: &'static str,
        extra: Value,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(sender) = &self.commands else {
            return false;
        };
        if !self.shell.synchronized || !self.connection_phase.ready() {
            return false;
        }
        let Some(tab) = self.terminal_drawer.tabs.iter_mut().find(|tab| {
            tab.id == id
                && tab.scope == self.environment_scope
                && Some(&tab.thread) == self.selected_thread.as_ref()
                && !tab.busy
                && tab.exists
                && tab.attached == Some(self.generation)
        }) else {
            return false;
        };
        if tag == "terminal.write" && !tab.stream_ready {
            return false;
        }
        let mut payload = tab.session();
        for (key, value) in extra.as_object().unwrap() {
            payload[key] = value.clone();
        }
        let (reply, receipt) = async_channel::bounded(1);
        // Queue immediately, not in an independently scheduled task: keystroke order is wire order.
        if let Err(error) = sender.try_send(connection::Command::Call {
            tag: tag.into(),
            payload,
            reply,
        }) {
            tab.error = Some(format!("Terminal input not sent: {error}"));
            cx.notify();
            return false;
        }
        if matches!(
            tag,
            "terminal.close" | "terminal.restart" | "terminal.clear"
        ) {
            tab.busy = true;
            tab.epoch += 1;
        }
        let id = id.to_owned();
        let thread = tab.thread.clone();
        let generation = self.generation;
        let scope = self.environment_scope.clone();
        let epoch = tab.epoch;
        if matches!(tag, "terminal.close" | "terminal.restart") {
            self.terminal_drawer.composition = None;
        }
        let sender = sender.clone();
        cx.spawn(async move |entity, cx| {
            let result = receipt
                .recv()
                .await
                .unwrap_or_else(|_| Err("Terminal receipt lost; state unknown".into()));
            let unsubscribe = entity
                .update(cx, |app, cx| {
                    if app.generation != generation || app.environment_scope != scope {
                        return None;
                    }
                    let tab = app.terminal_drawer.tabs.iter_mut().find(|tab| {
                        tab.id == id
                            && tab.scope == scope
                            && tab.thread == thread
                            && tab.epoch == epoch
                    })?;
                    tab.busy = false;
                    match result {
                        Ok(_) if tag == "terminal.close" => {
                            let stream = tab.stream.clone();
                            app.terminal_drawer.tabs.retain(|tab| {
                                !(tab.id == id && tab.scope == scope && tab.thread == thread)
                            });
                            cx.notify();
                            return Some(stream);
                        }
                        Ok(snapshot) if tag == "terminal.restart" => {
                            tab.event(json!({"type":"restarted","snapshot":snapshot}));
                        }
                        Ok(_) => {}
                        Err(error) => tab.error = Some(error),
                    }
                    cx.notify();
                    None
                })
                .ok()
                .flatten();
            if let Some(stream) = unsubscribe {
                let _ = sender.send(connection::Command::Unsubscribe(stream)).await;
            }
        })
        .detach();
        true
    }
    pub(crate) fn terminal_close(&mut self, id: &str, confirmed: bool, cx: &mut Context<Self>) {
        let Some(tab) = self.terminal_drawer.tabs.iter().find(|tab| {
            tab.id == id
                && tab.scope == self.environment_scope
                && Some(&tab.thread) == self.selected_thread.as_ref()
        }) else {
            return;
        };
        if !tab.exists && !tab.busy {
            if let Some(sender) = &self.commands {
                let _ = sender.try_send(connection::Command::Unsubscribe(tab.stream.clone()));
            }
            self.terminal_drawer.tabs.retain(|tab| {
                !(tab.id == id
                    && tab.scope == self.environment_scope
                    && Some(&tab.thread) == self.selected_thread.as_ref())
            });
            cx.notify();
            return;
        }
        if !confirmed && tab.running != Some(false) {
            self.terminal_drawer.confirm = Some(id.into());
            cx.notify();
            return;
        }
        self.terminal_drawer.confirm = None;
        self.terminal_rpc(id, "terminal.close", json!({}), cx);
    }
    pub(crate) fn terminal_resize(&mut self, id: &str, size: Size, cx: &mut Context<Self>) {
        if !self.shell.synchronized || !self.connection_phase.ready() {
            return;
        }
        let Some(tab) = self.terminal_drawer.tabs.iter_mut().find(|tab| {
            tab.id == id
                && tab.scope == self.environment_scope
                && Some(&tab.thread) == self.selected_thread.as_ref()
        }) else {
            return;
        };
        if tab.screen.size == size || tab.busy || tab.attached != Some(self.generation) {
            return;
        }
        tab.screen.resize(size);
        self.terminal_rpc(
            id,
            "terminal.resize",
            json!({"cols":size.cols,"rows":size.rows}),
            cx,
        );
        cx.notify();
    }
}
