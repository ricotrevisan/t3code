//! Saved scripts run in server-owned PTYs. Rows outlive checkout selection;
//! detached launch/control tasks are never mistaken for subprocess cancellation.
use crate::NativeApp;
use gpui_kit::Context;
use serde_json::{Value, json};
use t3code_desktop_gpui::{connection, project_actions};

#[derive(Clone)]
pub(crate) struct ActionRun {
    pub key: String,
    pub scope: String,
    pub generation: u64,
    pub project: String,
    pub thread: String,
    pub cwd: String,
    pub root: String,
    pub script: Value,
    pub status: String,
    pub output: String,
    pub error: Option<String>,
    pub busy: bool,
    pub terminal_exists: bool,
    // Startup generation is immutable; stream ownership can move to a fresh connection.
    subscription_generation: Option<u64>,
    subscription_key: String,
    sequence: Option<u64>,
    control_epoch: u64,
    escape: u8,
}

impl ActionRun {
    fn session(&self) -> Value {
        json!({"threadId":self.thread,"terminalId":self.key})
    }
    fn open(&self) -> Value {
        let path = (self.cwd != self.root).then_some(&self.cwd);
        let mut env = json!({"T3CODE_PROJECT_ROOT":self.root});
        if let Some(path) = path {
            env["T3CODE_WORKTREE_PATH"] = path.clone().into();
        }
        json!({"threadId":self.thread,"terminalId":self.key,"cwd":self.cwd,
            "worktreePath":path,"cols":100,"rows":24,"env":env})
    }
    fn append(&mut self, text: &str) {
        // A restrained output log, not a terminal emulator. Strip ANSI CSI/OSC
        // across chunks so shell prompts don't display control sequences.
        for ch in text.chars() {
            match self.escape {
                1 => {
                    self.escape = match ch {
                        '[' => 2,
                        ']' => 3,
                        _ => 0,
                    }
                }
                2 => {
                    if ('@'..='~').contains(&ch) {
                        self.escape = 0;
                    }
                }
                3 => {
                    if ch == '\u{7}' {
                        self.escape = 0;
                    } else if ch == '\u{1b}' {
                        self.escape = 4;
                    }
                }
                4 => self.escape = if ch == '\\' { 0 } else { 3 },
                _ if ch == '\u{1b}' => self.escape = 1,
                _ if !ch.is_control() || matches!(ch, '\n' | '\t') => self.output.push(ch),
                _ => {}
            }
        }
        // Bound retained output, including when the inspector isn't visible.
        if self.output.len() > 64 * 1024 {
            let mut start = self.output.len() - 64 * 1024;
            while !self.output.is_char_boundary(start) {
                start += 1;
            }
            self.output.drain(..start);
        }
    }
    fn event(&mut self, value: Value) {
        let snapshot = matches!(value["type"].as_str(), Some("snapshot" | "restarted"));
        let body = if snapshot { &value["snapshot"] } else { &value };
        if snapshot
            && (body["threadId"].as_str().is_none() || body["terminalId"].as_str().is_none())
        {
            self.error = Some("Attach returned no terminal snapshot; terminal state is unknown. Close remains available.".into());
            return;
        }
        if body["threadId"] != self.thread || body["terminalId"] != self.key {
            return;
        }
        if let Some(sequence) = body["sequence"].as_u64() {
            if self
                .sequence
                .is_some_and(|old| sequence < old || (!snapshot && sequence == old))
            {
                return;
            }
            self.sequence = Some(sequence);
        }
        if snapshot {
            self.output.clear();
            self.escape = 0;
            self.append(body["history"].as_str().unwrap_or_default());
            self.status = body["status"].as_str().unwrap_or("running").into();
            self.terminal_exists = true;
            self.error = None;
            return;
        }
        match value["type"].as_str() {
            Some("output") => self.append(value["data"].as_str().unwrap_or_default()),
            Some("activity") => {
                self.status = if value["hasRunningSubprocess"] == true {
                    "Running action"
                } else {
                    "Shell ready"
                }
                .into()
            }
            Some("exited") => self.status = format!("Exited · {}", value["exitCode"]),
            Some("closed") => {
                self.status = "Closed".into();
                self.terminal_exists = false;
            }
            Some("error") => self.error = value["message"].as_str().map(str::to_owned),
            Some("cleared") => self.output.clear(),
            _ => {}
        }
    }
}

impl NativeApp {
    pub(crate) fn run_project_action(&mut self, id: &str, cx: &mut Context<Self>) {
        let Some(project) = self
            .shell
            .projects
            .iter()
            .find(|p| Some(&p.id) == self.selected_project.as_ref())
        else {
            return;
        };
        // Resolve at click time, not from the editable action/clone/add forms.
        let Some(settings) = &self.project_settings else {
            self.set_status("Refresh saved actions before running", cx);
            return;
        };
        let Some(script) = project_actions::resolve(settings, &project.id, &project.scripts)
            .into_iter()
            .find(|script| script["id"] == id)
        else {
            self.set_status("Action was removed; refresh actions", cx);
            return;
        };
        let Some(thread) =
            self.shell.threads.iter().find(|t| {
                Some(&t.id) == self.selected_thread.as_ref() && t.project_id == project.id
            })
        else {
            self.set_status("Select a saved thread to run an action", cx);
            return;
        };
        let Some(cwd) = self.selected_workspace() else {
            return;
        };
        if self.action_runs.iter().any(|row| {
            row.busy
                && row.generation == self.generation
                && row.scope == self.environment_scope
                && row.thread == thread.id
                && row.cwd == cwd
                && row.script["id"] == id
        }) {
            return;
        }
        let key = format!("action-{}", uuid::Uuid::new_v4());
        let run = ActionRun {
            subscription_key: key.clone(),
            subscription_generation: None,
            key,
            scope: self.environment_scope.clone(),
            generation: self.generation,
            project: project.id.clone(),
            thread: thread.id.clone(),
            cwd,
            root: project.workspace_root.clone(),
            script,
            status: "Starting".into(),
            output: String::new(),
            error: None,
            busy: true,
            terminal_exists: false,
            sequence: None,
            control_epoch: 0,
            escape: 0,
        };
        self.start_action_run(run, cx);
    }

    fn start_action_run(&mut self, run: ActionRun, cx: &mut Context<Self>) {
        let Some(sender) = self.commands.clone() else {
            self.set_status("Not connected", cx);
            return;
        };
        let key = run.key.clone();
        let generation = run.generation;
        let epoch = run.control_epoch;
        self.action_runs.push(run.clone());
        cx.spawn(async move |entity, cx| {
            let opened = connection::call(&sender, "terminal.open", run.open()).await;
            let ready = opened.is_ok();
            let active = entity.update(cx, |app, cx| {
                let active = app.generation == generation && app.environment_scope == run.scope
                    && app.action_runs.iter().any(|row| row.key == key && row.control_epoch == epoch);
                if active && let Some(row) = app.action_runs.iter_mut().find(|row| row.key == key) {
                    row.terminal_exists = ready || opened.as_ref().err().is_some_and(|error| error.contains("outcome is unknown"));
                    if let Ok(snapshot) = &opened {
                        row.event(json!({"type":"snapshot","snapshot":snapshot}));
                        row.subscription_generation = Some(generation);
                    }
                }
                // A late receipt must never overwrite renewed stream/control ownership.
                if !active && let Some(row) = app.action_runs.iter_mut().find(|row| row.key == key && row.control_epoch == epoch) {
                    row.terminal_exists = true;
                    row.busy = false;
                    row.error = Some("Connection changed during startup; terminal may still exist. Close it on its original environment.".into());
                }
                cx.notify(); active
            }).unwrap_or(false);
            if !active { return; }
            let result = async {
                opened?;
                // Attach before write. Its snapshot reconciles any output racing attachment.
                sender.send(connection::Command::Subscribe {
                    key: key.clone(), tag: "terminal.attach".into(),
                    payload: json!({"threadId":run.thread,"terminalId":key,"restartIfNotRunning":false}),
                }).await.map_err(|_| "Not connected".to_owned())?;
                let active = entity.update(cx, |app, _| {
                    app.generation == generation && app.environment_scope == run.scope
                        && app.action_runs.iter().any(|row| row.key == key && row.control_epoch == epoch)
                }).unwrap_or(false);
                if !active { return Err("Connection changed; command was not sent".into()); }
                let mut input = run.session();
                input["data"] = format!("{}\r", run.script["command"].as_str().unwrap_or_default()).into();
                connection::call(&sender, "terminal.write", input).await?;
                Ok::<_, String>(())
            }.await;
            let _ = entity.update(cx, |app, cx| {
                if app.generation != generation || app.environment_scope != run.scope { return; }
                if let Some(row) = app.action_runs.iter_mut().find(|row| row.key == key && row.control_epoch == epoch) {
                    row.busy = false;
                    match result {
                        Ok(()) => { if row.status == "Starting" || row.status == "running" { row.status = "Command sent".into(); } },
                        Err(error) => { row.status = "Startup failed".into(); row.error = Some(error); },
                    }
                }
                cx.notify();
            });
        }).detach();
        cx.notify();
    }

    pub(crate) fn action_runs_connection_lost(&mut self) {
        for row in &mut self.action_runs {
            if row.scope == self.environment_scope && row.status != "Closed" {
                // Disconnect is not proof the PTY stopped. Preserve its identity
                // and allow Close on reconnect to the same environment.
                row.terminal_exists |= row.busy;
                row.busy = false;
                row.control_epoch += 1;
                row.error = Some("Connection lost; terminal state is unknown. Reconnect to its environment to close it.".into());
            }
        }
    }

    /// Called after the current shell becomes synchronized. Transport resync in
    /// the same generation replays its subscriptions; only fresh connections
    /// need a new attach. Terminal identity is never the subscription identity.
    pub(crate) fn action_runs_synchronized(&mut self, cx: &mut Context<Self>) {
        if !self.shell.synchronized {
            return;
        }
        let Some(sender) = &self.commands else {
            return;
        };
        for row in &mut self.action_runs {
            if row.scope != self.environment_scope
                || !row.terminal_exists
                || (row.busy
                    && row.generation == self.generation
                    && row.subscription_generation.is_none())
                || row.subscription_generation == Some(self.generation)
            {
                continue;
            }
            let key = format!("{}:stream:{}", row.key, uuid::Uuid::new_v4());
            let command = connection::Command::Subscribe {
                key: key.clone(),
                tag: "terminal.attach".into(),
                payload: json!({"threadId":row.thread,"terminalId":row.key,"restartIfNotRunning":false}),
            };
            row.subscription_generation = Some(self.generation);
            row.subscription_key = key.clone();
            row.sequence = None;
            let sender = sender.clone();
            // Reserve ownership before yielding, but don't drop attach when the
            // bounded command channel is busy synchronizing other app features.
            cx.spawn(async move |entity, cx| {
                if sender.send(command).await.is_err() {
                    let _ = entity.update(cx, |app, cx| {
                        app.action_run_update(&key, Err("Unable to attach output; terminal state is unknown. Close remains available.".into()), cx);
                    });
                }
            }).detach();
        }
        cx.notify();
    }

    pub(crate) fn action_run_update(
        &mut self,
        key: &str,
        item: Result<Value, String>,
        cx: &mut Context<Self>,
    ) {
        if let Some(row) = self.action_runs.iter_mut().find(|row| {
            row.subscription_key == key
                && row.scope == self.environment_scope
                && row.subscription_generation == Some(self.generation)
        }) {
            match item {
                Ok(value) => row.event(value),
                Err(error) => row.error = Some(error),
            }
            cx.notify();
        }
    }

    pub(crate) fn control_action_run(&mut self, key: &str, close: bool, cx: &mut Context<Self>) {
        let Some(sender) = self.commands.clone() else {
            return;
        };
        let Some(row) = self
            .action_runs
            .iter_mut()
            .find(|row| row.key == key && row.scope == self.environment_scope && !row.busy)
        else {
            return;
        };
        row.busy = true;
        row.error = None;
        row.control_epoch += 1;
        let epoch = row.control_epoch;
        let run = row.clone();
        let generation = self.generation;
        cx.spawn(async move |entity, cx| {
            let mut payload = run.session();
            if !close {
                payload["data"] = "\u{3}".into();
            }
            let result = connection::call(
                &sender,
                if close {
                    "terminal.close"
                } else {
                    "terminal.write"
                },
                payload,
            )
            .await;
            let unsubscribe = entity
                .update(cx, |app, cx| {
                    if app.generation != generation || app.environment_scope != run.scope {
                        return None;
                    }
                    let mut unsubscribe = None;
                    if let Some(row) = app
                        .action_runs
                        .iter_mut()
                        .find(|row| row.key == run.key && row.control_epoch == epoch)
                    {
                        row.busy = false;
                        match result {
                            Ok(_) if close => {
                                row.terminal_exists = false;
                                row.status = "Closed".into();
                                unsubscribe = Some(row.subscription_key.clone());
                            }
                            Ok(_) => row.status = "Interrupt sent".into(),
                            Err(error) => row.error = Some(error),
                        }
                    }
                    cx.notify();
                    unsubscribe
                })
                .ok()
                .flatten();
            if let Some(key) = unsubscribe {
                let _ = sender.send(connection::Command::Unsubscribe(key)).await;
            }
        })
        .detach();
        cx.notify();
    }

    pub(crate) fn retry_action_run(&mut self, key: &str, cx: &mut Context<Self>) {
        let Some(mut run) = self
            .action_runs
            .iter()
            .find(|row| {
                row.key == key
                    && row.scope == self.environment_scope
                    && !row.busy
                    && !row.terminal_exists
            })
            .cloned()
        else {
            return;
        };
        run.key = format!("action-{}", uuid::Uuid::new_v4());
        run.generation = self.generation;
        run.subscription_generation = None;
        run.subscription_key = run.key.clone();
        run.control_epoch += 1;
        run.output.clear();
        run.error = None;
        run.sequence = None;
        run.escape = 0;
        run.status = "Starting".into();
        run.busy = true;
        self.start_action_run(run, cx);
    }
}
