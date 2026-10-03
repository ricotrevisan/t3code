use gpui::{
    App, Application, Bounds, Context, Entity, Subscription, Task, Window, WindowBounds,
    WindowOptions, div, prelude::*, px, rgb, size,
};
use gpui_component::{
    Root,
    button::Button,
    input::{Input, InputEvent, InputState},
    text::TextView,
};
use serde_json::{Value, json};
use t3code_native_macos::{commands, connection, drafts, shell, sidebar, thread};

#[cfg(test)]
mod ui_tests;

struct NativeApp {
    shell: shell::Shell,
    archived_shell: shell::Shell,
    detail: thread::ThreadDetail,
    status: String,
    storage_error: Option<String>,
    selected_project: Option<String>,
    selected_thread: Option<String>,
    sidebar_view: sidebar::View,
    snooze_wake: Option<Task<()>>,
    custom_snooze: Entity<InputState>,
    confirm_delete: Option<String>,
    commands: Option<async_channel::Sender<connection::Command>>,
    connection_task: Option<Task<()>>,
    generation: u64,
    pending_actions: usize,
    composer: Entity<InputState>,
    title: Entity<InputState>,
    workspace: Entity<InputState>,
    provider: Entity<InputState>,
    model: Entity<InputState>,
    search: Entity<InputState>,
    search_results: Vec<Value>,
    drafts: std::collections::HashMap<String, String>,
    draft_store: Option<drafts::Drafts>,
    draft_save: Option<Task<()>>,
    environment_scope: String,
    stash_memory: Vec<String>,
    _subscriptions: Vec<Subscription>,
}

impl NativeApp {
    fn new(window: &mut Window, cx: &mut Context<Self>, auto_connect: bool) -> Self {
        let composer = cx.new(|cx| {
            InputState::new(window, cx)
                .multi_line(true)
                .auto_grow(3, 10)
                .placeholder("What should we work on?")
        });
        let title = cx.new(|cx| InputState::new(window, cx).placeholder("Project / thread title"));
        let workspace =
            cx.new(|cx| InputState::new(window, cx).placeholder("Absolute project directory"));
        let provider = cx.new(|cx| InputState::new(window, cx).placeholder("Provider instance ID"));
        let model = cx.new(|cx| InputState::new(window, cx).placeholder("Model ID"));
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("Search thread history"));
        let custom_snooze = cx.new(|cx| {
            InputState::new(window, cx).placeholder("Snooze until: ISO date/time with timezone")
        });
        let mut subscriptions = vec![cx.subscribe_in(&search, window, |app, _, event, _, cx| {
            if matches!(event, InputEvent::PressEnter { .. }) {
                app.search_threads(cx);
            }
        })];
        subscriptions.push(
            cx.subscribe_in(&composer, window, |app, _, event, window, cx| {
                if matches!(event, InputEvent::Change)
                    && let Some(id) = app.selected_thread.clone()
                {
                    let text = app.composer.read(cx).value().to_string();
                    app.drafts.insert(id.clone(), text.clone());
                    if let Some(store) = &mut app.draft_store {
                        store.set(&app.environment_scope, &id, text);
                        app.schedule_draft_save(cx);
                    }
                }
                if matches!(event, InputEvent::PressEnter { secondary: true }) {
                    app.send_prompt(window, cx);
                }
            }),
        );
        let mut app = Self {
            shell: shell::Shell::default(),
            archived_shell: shell::Shell::default(),
            detail: thread::ThreadDetail::default(),
            status: "Not connected".into(),
            storage_error: None,
            selected_project: None,
            selected_thread: None,
            sidebar_view: sidebar::View::Active,
            snooze_wake: None,
            custom_snooze,
            confirm_delete: None,
            commands: None,
            connection_task: None,
            generation: 0,
            pending_actions: 0,
            composer,
            title,
            workspace,
            provider,
            model,
            search,
            search_results: vec![],
            drafts: Default::default(),
            draft_store: None,
            draft_save: None,
            environment_scope: std::env::var("T3_NATIVE_SERVER")
                .ok()
                .and_then(|base| url::Url::parse(&base).ok())
                .map(|url| url.origin().ascii_serialization())
                .unwrap_or_else(|| "disconnected".into()),
            stash_memory: Vec::new(),
            _subscriptions: subscriptions,
        };
        if auto_connect {
            app.connect(cx);
            match drafts::Drafts::open(drafts::default_path()) {
                Ok(store) => app.draft_store = Some(store),
                Err(_) => app.storage_error = Some(
                    "Draft storage cannot be read; existing file preserved, persistence disabled"
                        .into(),
                ),
            }
        }
        app
    }

    fn disconnect(&mut self, cx: &mut Context<Self>) {
        self.connection_task = None;
        self.commands = None;
        self.shell = shell::Shell::default();
        self.archived_shell = shell::Shell::default();
        self.detail = thread::ThreadDetail::default();
        self.selected_thread = None;
        self.selected_project = None;
        self.pending_actions = 0;
        self.snooze_wake = None;
        self.generation += 1;
        self.status = "Disconnected".into();
        cx.notify();
    }

    fn connect(&mut self, cx: &mut Context<Self>) {
        self.disconnect(cx);
        self.status = "Configure T3_NATIVE_SERVER and T3_NATIVE_ACCESS_TOKEN to connect".into();
        if let (Ok(base), Ok(token)) = (
            std::env::var("T3_NATIVE_SERVER"),
            std::env::var("T3_NATIVE_ACCESS_TOKEN"),
        ) {
            match connection::ConnectionOptions::new(&base, token) {
                Ok(options) => {
                    let session = connection::start_session(options);
                    self.commands = Some(session.commands);
                    self.connection_task = Some(cx.spawn(async move |entity, cx| {
                        while let Ok(update) = session.updates.recv().await {
                            let keep_running = entity.update(cx, |app, cx| {
                                let result = match update {
                                    connection::Update::Status(status) => {
                                        app.status = status;
                                        Ok(())
                                    }
                                    connection::Update::Shell(item) => {
                                        let result = app.shell.apply(item).map_err(str::to_owned);
                                        app.schedule_snooze_wake(cx);
                                        if app.shell.synchronized {
                                            app.status = "Connected · synchronized".into();
                                        }
                                        result
                                    }
                                    connection::Update::Thread { thread_id, item } => {
                                        if app.selected_thread.as_ref() == Some(&thread_id) {
                                            app.detail.apply(&thread_id, item)
                                        } else {
                                            Ok(())
                                        }
                                    }
                                };
                                if let Err(error) = result {
                                    app.status =
                                        format!("Projection failed — reconnect required: {error}");
                                    app.commands = None;
                                    cx.notify();
                                    return false;
                                }
                                cx.notify();
                                true
                            });
                            if !matches!(keep_running, Ok(true)) {
                                break;
                            }
                        }
                    }));
                }
                Err(message) => self.status = message.into(),
            }
        }
        cx.notify();
    }

    fn select_thread(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(previous) = &self.selected_thread {
            self.drafts
                .insert(previous.clone(), self.composer.read(cx).value().to_string());
        }
        self.selected_thread = Some(id.into());
        self.confirm_delete = None;
        self.detail = thread::ThreadDetail::default();
        let text = self.drafts.get(id).cloned().unwrap_or_else(|| {
            self.draft_store
                .as_ref()
                .map(|store| store.get(&self.environment_scope, id).to_owned())
                .unwrap_or_default()
        });
        self.composer
            .update(cx, |input, cx| input.set_value(text, window, cx));
        if let Some(thread) = self
            .shell
            .threads
            .iter()
            .chain(&self.archived_shell.threads)
            .find(|t| t.id == id)
        {
            self.selected_project = Some(thread.project_id.clone());
            self.title.update(cx, |input, cx| {
                input.set_value(thread.title.clone(), window, cx)
            });
            self.provider.update(cx, |input, cx| {
                input.set_value(
                    thread.model_selection["instanceId"]
                        .as_str()
                        .unwrap_or("")
                        .to_owned(),
                    window,
                    cx,
                )
            });
            self.model.update(cx, |input, cx| {
                input.set_value(
                    thread.model_selection["model"]
                        .as_str()
                        .unwrap_or("")
                        .to_owned(),
                    window,
                    cx,
                )
            });
        }
        if let Some(sender) = &self.commands
            && sender
                .try_send(connection::Command::SelectThread(Some(id.into())))
                .is_err()
        {
            self.status = "Connection busy or closed; retry selection".into();
        }
        cx.notify();
    }

    fn perform(
        &mut self,
        tag: &str,
        payload: Value,
        sent_prompt: Option<(String, gpui::AnyWindowHandle)>,
        cx: &mut Context<Self>,
    ) {
        let Some(sender) = self.commands.clone() else {
            self.status = "Not connected".into();
            cx.notify();
            return;
        };
        let generation = self.generation;
        let thread_id = self.selected_thread.clone();
        let tag = tag.to_owned();
        self.pending_actions += 1;
        self.status = "Waiting for server receipt…".into();
        cx.spawn(async move |entity, cx| {
            let result = connection::call(&sender, &tag, payload).await;
            if result.is_ok()
                && let Some((text, handle)) = &sent_prompt
            {
                let _ = handle.update(cx, |_, window, cx| {
                    entity.update(cx, |app, cx| {
                        if app.generation == generation
                            && app.selected_thread == thread_id
                            && app.composer.read(cx).value().as_str() == text
                        {
                            app.composer
                                .update(cx, |input, cx| input.set_value("", window, cx));
                        }
                    })
                });
            }
            let _ = entity.update(cx, |app, cx| {
                if app.generation != generation {
                    return;
                }
                app.pending_actions = app.pending_actions.saturating_sub(1);
                match result {
                    Ok(value) => {
                        if tag == "orchestration.getArchivedShellSnapshot" {
                            match serde_json::from_value::<shell::Snapshot>(value.clone()) {
                                Ok(snapshot) => {
                                    let _ = app
                                        .archived_shell
                                        .apply(shell::ShellItem::Snapshot { snapshot });
                                }
                                Err(_) => {
                                    app.status = "Invalid archive snapshot".into();
                                    cx.notify();
                                    return;
                                }
                            }
                        }
                        if tag == "orchestration.dispatchCommand"
                            && app.sidebar_view == sidebar::View::Archived
                        {
                            app.perform(
                                "orchestration.getArchivedShellSnapshot",
                                json!({}),
                                None,
                                cx,
                            );
                        }
                        if tag == "orchestration.searchThreads" {
                            app.search_results =
                                value["matches"].as_array().cloned().unwrap_or_default();
                        }
                        if let Some((text, _)) = sent_prompt
                            && let Some(id) = thread_id
                            && app.drafts.get(&id).is_some_and(|draft| draft == &text)
                        {
                            app.drafts.remove(&id);
                            if let Some(store) = &mut app.draft_store {
                                store.set(&app.environment_scope, &id, String::new());
                                app.schedule_draft_save(cx);
                            }
                        }
                        app.status = format!(
                            "Server accepted · sequence {}",
                            value["sequence"]
                                .as_u64()
                                .map(|s| s.to_string())
                                .unwrap_or_else(|| "—".into())
                        );
                    }
                    Err(error) => app.status = error,
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn dispatch(&mut self, payload: Value, cx: &mut Context<Self>) {
        self.perform("orchestration.dispatchCommand", payload, None, cx);
    }

    fn lifecycle(&mut self, kind: &str, cx: &mut Context<Self>) {
        let Some(id) = &self.selected_thread else {
            return;
        };
        let mut payload = commands::command(kind, id);
        match kind {
            "thread.meta.update" => {
                let title = self.title.read(cx).value();
                if title.trim().is_empty() {
                    self.status = "A title is required".into();
                    cx.notify();
                    return;
                }
                payload["title"] = json!(title.trim());
            }
            "thread.unsnooze" | "thread.unsettle" => payload["reason"] = json!("user"),
            "thread.snooze" => {
                payload["snoozedUntil"] = json!(
                    (chrono::Utc::now() + chrono::Duration::hours(1))
                        .to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
                )
            }
            "thread.turn.interrupt" | "thread.session.stop" => {
                payload["createdAt"] = json!(commands::now())
            }
            "thread.delete" if self.confirm_delete.as_ref() != Some(id) => {
                self.confirm_delete = Some(id.clone());
                self.status = "Click Delete again to confirm permanent thread deletion".into();
                cx.notify();
                return;
            }
            _ => {}
        }
        self.dispatch(payload, cx);
    }

    fn schedule_snooze_wake(&mut self, cx: &mut Context<Self>) {
        self.snooze_wake = sidebar::expiry(&self.shell.threads, chrono::Utc::now()).map(|delay| {
            cx.spawn(async move |entity, cx| {
                gpui::Timer::after(delay).await;
                let _ = entity.update(cx, |app, cx| {
                    app.schedule_snooze_wake(cx);
                    cx.notify();
                });
            })
        });
    }

    fn snooze_custom(&mut self, cx: &mut Context<Self>) {
        let Some(id) = &self.selected_thread else {
            return;
        };
        let value = self.custom_snooze.read(cx).value();
        let until = match chrono::DateTime::parse_from_rfc3339(value.trim()) {
            Ok(until) if until > chrono::Utc::now() => until,
            _ => {
                self.status = "Choose a future ISO date/time with timezone".into();
                cx.notify();
                return;
            }
        };
        let mut command = commands::command("thread.snooze", id);
        command["snoozedUntil"] = json!(
            until
                .with_timezone(&chrono::Utc)
                .to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
        );
        self.dispatch(command, cx);
    }

    fn search_threads(&mut self, cx: &mut Context<Self>) {
        let query = self.search.read(cx).value();
        if !(2..=200).contains(&query.trim().chars().count()) {
            self.status = "Search needs 2–200 characters".into();
            cx.notify();
            return;
        }
        self.perform(
            "orchestration.searchThreads",
            json!({"query":query.trim(),"limit":50}),
            None,
            cx,
        );
    }

    fn schedule_draft_save(&mut self, cx: &mut Context<Self>) {
        self.draft_save = Some(cx.spawn(async move |entity, cx| {
            gpui::Timer::after(std::time::Duration::from_millis(200)).await;
            let write = entity.update(cx, |app, _| {
                app.draft_store.as_ref().map(|store| store.write_snapshot())
            });
            if let Ok(Some(Ok(write))) = write {
                let result = cx
                    .background_executor()
                    .spawn(async move { write.persist() })
                    .await;
                if result.is_err() {
                    let _ = entity.update(cx, |app, cx| {
                        app.storage_error = Some("Could not persist native drafts".into());
                        cx.notify();
                    });
                }
            }
        }));
    }

    fn stash_prompt(&mut self, restore: bool, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.composer.read(cx).value().to_string();
        if restore {
            if !text.is_empty() {
                self.status = "Stash or clear the current prompt before restoring".into();
                cx.notify();
                return;
            }
            let text = if let Some(store) = &mut self.draft_store {
                store.unstash(&self.environment_scope)
            } else {
                self.stash_memory.pop()
            };
            if let Some(text) = text {
                self.composer
                    .update(cx, |input, cx| input.set_value(text, window, cx));
            }
        } else {
            if text.trim().is_empty() {
                return;
            }
            if let Some(store) = &mut self.draft_store {
                store.stash(&self.environment_scope, text);
            } else {
                self.stash_memory.push(text);
            }
            self.composer
                .update(cx, |input, cx| input.set_value("", window, cx));
        }
        if self.draft_store.is_some() {
            self.schedule_draft_save(cx);
        }
        cx.notify();
    }

    fn send_prompt(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.pending_actions > 0 {
            self.status = "Wait for the pending server receipt".into();
            cx.notify();
            return;
        }
        if self.detail.thread.as_ref().is_some_and(|thread| {
            matches!(
                thread["session"]["status"].as_str(),
                Some("starting" | "running")
            )
        }) {
            self.status =
                "Turn running — interrupt first. Native queue support is not implemented yet."
                    .into();
            cx.notify();
            return;
        }
        let Some(id) = self.selected_thread.clone() else {
            return;
        };
        let Some(thread) = self.shell.threads.iter().find(|t| t.id == id) else {
            return;
        };
        let text = self.composer.read(cx).value().to_string();
        let selection = match commands::select_model(
            &thread.model_selection,
            self.provider.read(cx).value().as_str(),
            self.model.read(cx).value().as_str(),
        ) {
            Ok(selection) => selection,
            Err(error) => {
                self.status = error.into();
                cx.notify();
                return;
            }
        };
        match commands::start_turn(
            &id,
            &text,
            selection,
            thread.runtime_mode.as_deref().unwrap_or("full-access"),
            thread.interaction_mode.as_deref().unwrap_or("default"),
        ) {
            Ok(payload) => {
                self.drafts.insert(id, text.clone());
                self.perform(
                    "orchestration.dispatchCommand",
                    payload,
                    Some((text, window.window_handle())),
                    cx,
                );
            }
            Err(message) => {
                self.status = message.into();
                cx.notify();
            }
        }
    }
}

impl Drop for NativeApp {
    fn drop(&mut self) {
        // Flush the final edit on normal window closure; regular saves use the background executor.
        self.draft_save = None;
        if let Some(store) = &self.draft_store
            && let Ok(write) = store.write_snapshot()
            && write.persist().is_err()
        {
            eprintln!("Could not flush native drafts during shutdown");
        }
    }
}

impl Render for NativeApp {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mut projects = Vec::new();
        for project in &self.shell.projects {
            let project_id = project.id.clone();
            let title = project.title.clone();
            let workspace = project.workspace_root.clone();
            let mut row = div().flex().flex_col().gap_2().child(
                Button::new(gpui::SharedString::from(format!("project-{project_id}")))
                    .label(title.clone())
                    .on_click(cx.listener(move |app, _, window, cx| {
                        app.selected_project = Some(project_id.clone());
                        app.title
                            .update(cx, |input, cx| input.set_value(title.clone(), window, cx));
                        app.workspace.update(cx, |input, cx| {
                            input.set_value(workspace.clone(), window, cx)
                        });
                        cx.notify();
                    })),
            );
            let threads = if self.sidebar_view == sidebar::View::Archived {
                &self.archived_shell.threads
            } else {
                &self.shell.threads
            };
            for thread in threads.iter().filter(|t| {
                t.project_id == project.id
                    && sidebar::belongs(t, self.sidebar_view, chrono::Utc::now())
            }) {
                let id = thread.id.clone();
                let label = format!(
                    "{}{}{}",
                    if thread.pinned_at.is_some() {
                        "📌 "
                    } else {
                        ""
                    },
                    thread.title,
                    if sidebar::snoozed(thread, chrono::Utc::now()) {
                        " · snoozed"
                    } else {
                        ""
                    }
                );
                row = row.child(
                    Button::new(gpui::SharedString::from(format!("thread-{id}")))
                        .label(label)
                        .on_click(cx.listener(move |app, _, window, cx| {
                            app.select_thread(&id, window, cx)
                        })),
                );
            }
            projects.push(row);
        }
        let mut conversation = div()
            .id("conversation")
            .flex()
            .flex_col()
            .flex_1()
            .overflow_y_scroll()
            .gap_4()
            .p_4();
        if let Some(detail) = &self.detail.thread {
            conversation =
                conversation.child(detail["title"].as_str().unwrap_or("Thread").to_owned());
            if let Some(messages) = detail["messages"].as_array() {
                for message in messages {
                    let id = gpui::SharedString::from(format!(
                        "message-{}",
                        message["id"].as_str().unwrap_or("unknown")
                    ));
                    let text = message["text"].as_str().unwrap_or("").to_owned();
                    conversation = conversation.child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_2()
                            .child(format!(
                                "{}{}",
                                message["role"].as_str().unwrap_or("message"),
                                if message["streaming"] == true {
                                    " · streaming"
                                } else {
                                    ""
                                }
                            ))
                            .child(TextView::markdown(id, text, window, cx)),
                    );
                }
            }
            if let Some(activities) = detail["activities"].as_array() {
                for activity in activities {
                    conversation = conversation.child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(format!(
                                "{} · {}",
                                activity["tone"].as_str().unwrap_or("activity"),
                                activity["summary"].as_str().unwrap_or("")
                            ))
                            .child(activity["payload"].to_string()),
                    );
                }
            }
            if let Some(session) = detail.get("session").filter(|s| !s.is_null()) {
                conversation = conversation.child(format!(
                    "Session: {} · {}",
                    session["status"].as_str().unwrap_or("unknown"),
                    session["lastError"].as_str().unwrap_or("")
                ));
            }
        } else {
            conversation = conversation.child(
                "Select a thread. Loading history uses the server's per-thread subscription.",
            );
        }
        let mut search_results = div().flex().flex_col().gap_2();
        for result in &self.search_results {
            if let Some(id) = result["threadId"].as_str() {
                let id = id.to_owned();
                search_results = search_results.child(
                    Button::new(gpui::SharedString::from(format!("search-{id}")))
                        .label(result["snippet"].as_str().unwrap_or("Match").to_owned())
                        .on_click(cx.listener(move |app, _, window, cx| {
                            app.select_thread(&id, window, cx)
                        })),
                );
            }
        }
        div().flex().size_full().bg(rgb(0x16181d)).text_color(rgb(0xe5e7eb))
            .child(div().id("sidebar").flex().flex_col().w(px(280.)).h_full().overflow_y_scroll().p_3().gap_3()
                .child("T3 Code · Native")
                .child(Button::new("connect").label("Connect / retry").on_click(cx.listener(|app,_,_,cx|app.connect(cx))))
                .child(Button::new("disconnect").label("Disconnect").on_click(cx.listener(|app,_,_,cx|app.disconnect(cx))))
                .child(Input::new(&self.search)).child(Button::new("search").label("Search history").on_click(cx.listener(|app,_,_,cx|app.search_threads(cx))))
                .child(search_results)
                .child(div().flex().flex_wrap().gap_2().children([
                    ("active-view","Active",sidebar::View::Active),
                    ("settled-view","Settled",sidebar::View::Settled),
                    ("snoozed-view","Snoozed",sidebar::View::Snoozed),
                    ("archived-view","Archived",sidebar::View::Archived),
                ].into_iter().map(|(id,label,view)|Button::new(id).debug_selector(move||id.into()).label(label).on_click(cx.listener(move|app,_,_,cx| {
                    app.sidebar_view = view;
                    if view == sidebar::View::Archived { app.perform("orchestration.getArchivedShellSnapshot",json!({}),None,cx); }
                    cx.notify();
                })))))
                .children(projects))
            .child(div().flex().flex_col().flex_1().h_full().p_3().gap_3()
                .child(self.status.clone())
                .children(self.storage_error.clone().map(|error|div().text_color(rgb(0xfca5a5)).child(error)))
                .child(div().flex().gap_2().child(Input::new(&self.title)).child(Input::new(&self.workspace)))
                .child(div().flex().gap_2()
                    .child(Button::new("add-project").label("Add project").on_click(cx.listener(|app,_,_,cx| {
                        match commands::create_project(app.title.read(cx).value().as_str(), app.workspace.read(cx).value().as_str()) { Ok(command) => app.dispatch(command,cx), Err(error) => { app.status = error.into(); cx.notify(); } }
                    })))
                    .child(Button::new("new-thread").label("New thread").on_click(cx.listener(|app,_,_,cx| {
                        if let Some(project) = &app.selected_project {
                            let selection = json!({"instanceId":app.provider.read(cx).value().as_str(),"model":app.model.read(cx).value().as_str()});
                            match commands::create_thread(project,app.title.read(cx).value().as_str(),selection) { Ok(command) => app.dispatch(command,cx), Err(error) => { app.status = error.into(); cx.notify(); } }
                        } else { app.status = "Select a project first".into(); cx.notify(); }
                    })))
                    .child(Button::new("rename").debug_selector(|| "rename".into()).label("Rename thread").on_click(cx.listener(|app,_,_,cx|app.lifecycle("thread.meta.update",cx)))))
                .child(div().flex().flex_wrap().gap_2().children([
                    ("archive","Archive","thread.archive"),("unarchive","Unarchive","thread.unarchive"),
                    ("settle","Settle","thread.settle"),("unsettle","Unsettle","thread.unsettle"),
                    ("snooze","Snooze 1h","thread.snooze"),("unsnooze","Unsnooze","thread.unsnooze"),
                    ("pin","Pin","thread.pin"),("unpin","Unpin","thread.unpin"),
                    ("delete","Delete thread","thread.delete"),
                ].into_iter().map(|(id,label,kind)| Button::new(id).debug_selector(move || id.into()).label(label).on_click(cx.listener(move |app,_,_,cx|app.lifecycle(kind,cx))))))
                .child(div().flex().gap_2().child(Input::new(&self.custom_snooze)).child(Button::new("custom-snooze").label("Custom snooze").on_click(cx.listener(|app,_,_,cx|app.snooze_custom(cx)))))
                .child(conversation)
                .child(div().flex().gap_2().child(Input::new(&self.provider)).child(Input::new(&self.model)))
                .child(Input::new(&self.composer))
                .child(div().flex().gap_2()
                    .child(Button::new("send").debug_selector(|| "send".into()).label("Send").on_click(cx.listener(|app,_,window,cx|app.send_prompt(window,cx))))
                    .child(Button::new("stash").debug_selector(|| "stash".into()).label("Stash prompt").on_click(cx.listener(|app,_,window,cx|app.stash_prompt(false,window,cx))))
                    .child(Button::new("restore-stash").debug_selector(|| "restore-stash".into()).label("Restore stashed").on_click(cx.listener(|app,_,window,cx|app.stash_prompt(true,window,cx))))
                    .child(Button::new("interrupt").label("Interrupt").on_click(cx.listener(|app,_,_,cx|app.lifecycle("thread.turn.interrupt",cx))))
                    .child(Button::new("stop").label("Stop session").on_click(cx.listener(|app,_,_,cx|app.lifecycle("thread.session.stop",cx)))))
                .child("Experimental native slice — feature parity is still being evaluated"))
    }
}

fn main() {
    Application::new().run(move |cx: &mut App| {
        gpui_component::init(cx);
        gpui_component::Theme::change(gpui_component::ThemeMode::Dark, None, cx);
        let bounds = Bounds::centered(None, size(px(1280.), px(900.)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..Default::default()
            },
            |window, cx| {
                let view = cx.new(|cx| NativeApp::new(window, cx, true));
                cx.new(|cx| Root::new(view, window, cx))
            },
        )
        .expect("could not open native window");
        cx.on_window_closed(|cx| {
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();
        cx.activate(true);
    });
}
