use gpui_kit::base::Selectable;
use gpui_kit::component::{
    ActiveTheme as _, Disableable, Sizable,
    button::{Button, ButtonVariants as _},
    input::{Input, InputEvent, InputState, Position, Textarea, TextareaState},
    text::TextView,
    TitleBar,
};
use gpui_kit::{
    App, Bounds, Context, Entity, FocusHandle, FontWeight, KeyBinding, Subscription, Task, Window,
    WindowBounds, WindowOptions, div, prelude::*, px, size,
};
use serde_json::{Value, json};
use t3code_desktop_gpui::{
    auth, commands, connection, conversation, credentials, drafts, files, palette, project_actions,
    providers, shell, sidebar, thread, workspaces,
};

gpui_kit::actions!(
    native,
    [
        ContentSubmit,
        ContentNext,
        ContentPrevious,
        OpenFiles,
        OpenContentSearch,
        TogglePalette,
        ClosePalette,
        PaletteNext,
        PalettePrevious,
        NewThread,
        StashPrompt,
        ToggleSettle,
        TogglePin,
        QuitNative
    ]
);

#[cfg(test)]
mod ui_tests;
mod theme;

fn single_line_input(window: &mut Window, cx: &mut Context<InputState>) -> InputState {
    // GPUI's single-line shaper panics if native text insertion includes a newline.
    InputState::new(window, cx).validate(|text, _| !text.contains(['\n', '\r']))
}

struct NativeApp {
    focus: FocusHandle,
    palette_open: bool,
    palette_query: Entity<InputState>,
    palette_index: usize,
    previous_focus: Option<FocusHandle>,
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
    confirm_project_removal: Option<String>,
    commands: Option<async_channel::Sender<connection::Command>>,
    connection_task: Option<Task<()>>,
    generation: u64,
    server: Entity<InputState>,
    pairing: Entity<InputState>,
    access: Option<auth::Access>,
    pairing_task: Option<Task<()>>,
    use_dev_credential: bool,
    vault: credentials::Vault,
    credential_task: Option<Task<()>>,
    pending_actions: usize,
    composer: Entity<TextareaState>,
    title: Entity<InputState>,
    workspace: Entity<InputState>,
    provider: Entity<InputState>,
    model: Entity<InputState>,
    model_override: Option<Value>,
    provider_catalog: Vec<providers::Provider>,
    catalog_open: bool,
    project_actions_open: bool,
    project_settings: Option<Value>,
    action_name: Entity<InputState>,
    action_command: Entity<InputState>,
    action_edit_id: Option<String>,
    action_setup: bool,
    action_wait: bool,
    action_busy: bool,
    action_task: Option<Task<()>>,
    confirm_action_delete: Option<String>,
    search: Entity<InputState>,
    search_results: Vec<Value>,
    pending_threads: Vec<drafts::PendingThread>,
    prepare_new_worktree: bool,
    start_from_origin: bool,
    run_setup_script: bool,
    setup_progress: Option<workspaces::SetupProgress>,
    setup_expanded: bool,
    pending_history_attached: bool,
    new_thread_checkout: Option<(String, String, Option<String>)>,
    worktree_outcome: Option<Value>,
    worktrees_open: bool,
    worktree_ref: Entity<InputState>,
    worktree_branch: Entity<InputState>,
    worktree_path: Entity<InputState>,
    refs: Vec<workspaces::Ref>,
    refs_cursor: Option<u64>,
    worktree_task: Option<Task<()>>,
    worktree_epoch: u64,
    worktree_busy: bool,
    confirm_worktree_removal: Option<String>,
    clone_open: bool,
    clone_remote: Entity<InputState>,
    clone_progress: Vec<workspaces::CloneProgress>,
    files_open: bool,
    file_path: Entity<InputState>,
    file_contents: Entity<TextareaState>,
    file_editor: files::Editor,
    file_epoch: u64,
    file_busy: bool,
    file_environment: String,
    file_entries: Vec<files::Entry>,
    content_query: Entity<InputState>,
    content_options: files::SearchOptions,
    content_results: Option<files::ContentResults>,
    content_request_query: String,
    content_index: usize,
    content_scroll: gpui_kit::ScrollHandle,
    file_listing_truncated: bool,
    file_listing_task: Option<Task<()>>,
    file_listing_epoch: u64,
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
            TextareaState::new(window, cx)
                .auto_grow(3, 10)
                .placeholder("What should we work on?")
        });
        let server = cx.new(|cx| {
            single_line_input(window, cx)
                .placeholder("Server origin (https://…)")
                .default_value(std::env::var("T3_NATIVE_SERVER").unwrap_or_default())
        });
        let pairing = cx.new(|cx| {
            single_line_input(window, cx)
                .masked(true)
                .placeholder("Pairing URL or one-time token")
        });
        let file_path =
            cx.new(|cx| single_line_input(window, cx).placeholder("Workspace-relative file path"));
        let clone_remote =
            cx.new(|cx| single_line_input(window, cx).placeholder("Repository clone URL"));
        let content_query =
            cx.new(|cx| single_line_input(window, cx).placeholder("Search workspace contents"));
        let file_contents = cx.new(|cx| TextareaState::new(window, cx).auto_grow(8, 20));
        let palette_query =
            cx.new(|cx| single_line_input(window, cx).placeholder("Search commands…"));
        let title =
            cx.new(|cx| single_line_input(window, cx).placeholder("Project / thread title"));
        let workspace =
            cx.new(|cx| single_line_input(window, cx).placeholder("Absolute project directory"));
        let provider =
            cx.new(|cx| single_line_input(window, cx).placeholder("Provider instance ID"));
        let model = cx.new(|cx| single_line_input(window, cx).placeholder("Model ID"));
        let search =
            cx.new(|cx| single_line_input(window, cx).placeholder("Search thread history"));
        let custom_snooze = cx.new(|cx| {
            single_line_input(window, cx).placeholder("Snooze until: ISO date/time with timezone")
        });
        let mut subscriptions = vec![cx.subscribe_in(&search, window, |app, _, event, _, cx| {
            if matches!(event, InputEvent::PressEnter { .. }) {
                app.search_threads(cx);
            }
        })];
        subscriptions.push(
            cx.subscribe_in(
                &content_query,
                window,
                |app, _, event, window, cx| match event {
                    InputEvent::Change => {
                        if app.content_query.read(cx).value().as_str() == app.content_request_query
                        {
                            return;
                        }
                        app.content_results = None;
                        app.content_index = 0;
                        app.file_listing_epoch += 1;
                        app.file_listing_task = None;
                        cx.notify();
                    }
                    InputEvent::PressEnter { .. } => {
                        if app
                            .content_results
                            .as_ref()
                            .is_some_and(|results| !results.matches.is_empty())
                        {
                            app.open_content_hit(app.content_index, window, cx);
                        } else {
                            app.search_contents(cx);
                        }
                    }
                    _ => {}
                },
            ),
        );
        subscriptions.push(
            cx.subscribe_in(&pairing, window, |app, _, event, window, cx| {
                if matches!(event, InputEvent::PressEnter { .. }) {
                    app.pair(window, cx);
                }
            }),
        );
        subscriptions.push(
            cx.subscribe_in(&custom_snooze, window, |app, _, event, _, cx| {
                if matches!(event, InputEvent::PressEnter { .. }) {
                    app.snooze_custom(cx);
                }
            }),
        );
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
                if matches!(
                    event,
                    InputEvent::PressEnter {
                        secondary: true,
                        ..
                    }
                ) {
                    app.send_prompt(window, cx);
                }
            }),
        );
        subscriptions.push(
            cx.subscribe_in(
                &palette_query,
                window,
                |app, _, event, window, cx| match event {
                    InputEvent::Change => {
                        app.palette_index = 0;
                        cx.notify();
                    }
                    InputEvent::PressEnter { .. } => {
                        if let Some(entry) = palette::matching(
                            &app.palette_query.read(cx).value(),
                            app.selected_thread.is_some(),
                        )
                        .get(app.palette_index)
                        {
                            let command = entry.command;
                            app.close_palette(window, cx);
                            app.execute_command(command, window, cx);
                        }
                    }
                    _ => {}
                },
            ),
        );
        let mut app = Self {
            focus: cx.focus_handle(),
            palette_open: false,
            palette_query,
            palette_index: 0,
            previous_focus: None,
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
            confirm_project_removal: None,
            commands: None,
            connection_task: None,
            generation: 0,
            server,
            pairing,
            access: None,
            pairing_task: None,
            use_dev_credential: true,
            vault: credentials::Vault::default(),
            credential_task: None,
            pending_actions: 0,
            composer,
            title,
            workspace,
            provider,
            model,
            model_override: None,
            provider_catalog: vec![],
            catalog_open: false,
            project_actions_open: false,
            project_settings: None,
            action_name: cx.new(|cx| single_line_input(window, cx).placeholder("Action name")),
            action_command: cx.new(|cx| {
                single_line_input(window, cx).placeholder("Shell command in the project checkout")
            }),
            action_edit_id: None,
            action_setup: false,
            action_wait: false,
            action_busy: false,
            action_task: None,
            confirm_action_delete: None,
            search,
            search_results: vec![],
            pending_threads: vec![],
            prepare_new_worktree: false,
            start_from_origin: true,
            run_setup_script: true,
            setup_progress: None,
            setup_expanded: false,
            pending_history_attached: false,
            new_thread_checkout: None,
            worktree_outcome: None,
            worktrees_open: false,
            worktree_ref: cx
                .new(|cx| single_line_input(window, cx).placeholder("Base or existing branch")),
            worktree_branch: cx
                .new(|cx| single_line_input(window, cx).placeholder("New branch (optional)")),
            worktree_path: cx.new(|cx| {
                single_line_input(window, cx).placeholder("Worktree path (optional for creation)")
            }),
            refs: vec![],
            refs_cursor: None,
            worktree_task: None,
            worktree_epoch: 0,
            worktree_busy: false,
            confirm_worktree_removal: None,
            clone_open: false,
            clone_remote,
            clone_progress: Vec::new(),
            files_open: false,
            file_path,
            file_contents,
            file_editor: files::Editor::default(),
            file_epoch: 0,
            file_busy: false,
            file_environment: String::new(),
            file_entries: vec![],
            content_query,
            content_options: files::SearchOptions::default(),
            content_results: None,
            content_request_query: String::new(),
            content_index: 0,
            content_scroll: gpui_kit::ScrollHandle::new(),
            file_listing_truncated: false,
            file_listing_task: None,
            file_listing_epoch: 0,
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
            match drafts::Drafts::open(drafts::default_path()) {
                Ok(store) => app.draft_store = Some(store),
                Err(_) => app.storage_error = Some(
                    "Draft storage cannot be read; existing file preserved, persistence disabled"
                        .into(),
                ),
            }
            app.connect(cx);
        }
        app
    }

    fn toggle_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.palette_open {
            self.close_palette(window, cx);
            return;
        }
        self.previous_focus = window.focused(cx);
        self.palette_open = true;
        self.palette_index = 0;
        self.palette_query.update(cx, |input, cx| {
            input.set_value("", window, cx);
            input.focus(window, cx);
        });
        cx.notify();
    }

    fn close_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.palette_open = false;
        if let Some(focus) = self.previous_focus.take() {
            window.focus(&focus, cx);
        }
        cx.notify();
    }

    fn move_palette(&mut self, forward: bool, cx: &mut Context<Self>) {
        let count = palette::matching(
            &self.palette_query.read(cx).value(),
            self.selected_thread.is_some(),
        )
        .len();
        if count > 0 {
            self.palette_index = if forward {
                (self.palette_index + 1) % count
            } else {
                (self.palette_index + count - 1) % count
            };
        }
        cx.notify();
    }

    fn execute_command(
        &mut self,
        command: palette::Command,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        use palette::Command;
        if command == Command::Delete
            && self
                .pending_threads
                .iter()
                .any(|draft| Some(&draft.thread.id) == self.selected_thread.as_ref())
        {
            self.discard_pending(window, cx);
            return;
        }
        match command {
            Command::SaveProjectDefault => self.update_project_default(false, cx),
            Command::ClearProjectDefault => self.update_project_default(true, cx),
            Command::InheritProjectDefault => {
                self.project_actions(Some(json!({"operation":"model-inherit"})), cx)
            }
            Command::ProjectActions => {
                self.project_actions_open = true;
                self.project_actions(None, cx);
            }
            Command::Worktrees => {
                self.worktrees_open = true;
                self.worktree_operation("vcs.listRefs", false, cx);
            }
            Command::CloneProject => {
                self.clone_open = true;
                if let Some(sender) = &self.commands {
                    let _ = sender.try_send(connection::Command::Subscribe {
                        key: "project-clones".into(),
                        tag: "subscribeProjectClones".into(),
                        payload: json!({}),
                    });
                }
                self.clone_remote
                    .update(cx, |input, cx| input.focus(window, cx));
                cx.notify();
            }
            Command::Files | Command::ContentSearch => {
                self.files_open = true;
                if command == Command::ContentSearch {
                    self.content_query
                        .update(cx, |input, cx| input.focus(window, cx));
                } else {
                    self.file_path
                        .update(cx, |input, cx| input.focus(window, cx));
                }
                cx.notify();
            }
            Command::AddProject => match commands::create_project(
                &self.title.read(cx).value(),
                &self.workspace.read(cx).value(),
            ) {
                Ok(command) => self.dispatch(command, cx),
                Err(error) => {
                    self.status = error.into();
                    cx.notify();
                }
            },
            Command::RenameProject => self.update_project(false, cx),
            Command::RemoveProject => self.update_project(true, cx),
            Command::NewThread => self.new_thread(window, cx),
            Command::Search => self.search.update(cx, |input, cx| input.focus(window, cx)),
            Command::CustomSnooze => self
                .custom_snooze
                .update(cx, |input, cx| input.focus(window, cx)),
            Command::Stash => self.stash_prompt(false, window, cx),
            Command::RestoreStash => self.stash_prompt(true, window, cx),
            Command::Connect => self.connect(cx),
            Command::Disconnect => self.disconnect(cx),
            _ => self.lifecycle(
                match command {
                    Command::Rename => "thread.meta.update",
                    Command::Archive => "thread.archive",
                    Command::Unarchive => "thread.unarchive",
                    Command::Settle => "thread.settle",
                    Command::Unsettle => "thread.unsettle",
                    Command::Snooze => "thread.snooze",
                    Command::Unsnooze => "thread.unsnooze",
                    Command::Pin => "thread.pin",
                    Command::Unpin => "thread.unpin",
                    Command::Delete => "thread.delete",
                    Command::Interrupt => "thread.turn.interrupt",
                    Command::Stop => "thread.session.stop",
                    _ => unreachable!(),
                },
                cx,
            ),
        }
    }

    fn list_files(&mut self, search: bool, cx: &mut Context<Self>) {
        let Some(sender) = self.commands.clone() else {
            self.status = "Not connected".into();
            cx.notify();
            return;
        };
        let Some(project) = self
            .selected_project
            .as_ref()
            .and_then(|id| self.shell.projects.iter().find(|project| &project.id == id))
        else {
            self.status = "Select a project first".into();
            cx.notify();
            return;
        };
        let cwd = self
            .selected_workspace()
            .unwrap_or_else(|| project.workspace_root.clone());
        let query = self.file_path.read(cx).value().to_string();
        let payload = if search {
            json!({"cwd":cwd,"query":query,"limit":100})
        } else {
            json!({"cwd":cwd,"directoryPath":query.trim()})
        };
        self.file_listing_epoch += 1;
        let epoch = self.file_listing_epoch;
        let generation = self.generation;
        let project_id = self.selected_project.clone();
        self.file_listing_task = Some(cx.spawn(async move |entity, cx| {
            let result = connection::call(
                &sender,
                if search {
                    "projects.searchEntries"
                } else {
                    "projects.listEntries"
                },
                payload,
            )
            .await;
            let _ = entity.update(cx, |app, cx| {
                if app.generation != generation
                    || app.file_listing_epoch != epoch
                    || app.selected_project != project_id
                {
                    return;
                }
                match result {
                    Ok(value) => match serde_json::from_value::<files::Listing>(value) {
                        Ok(list) => {
                            app.file_entries = list.entries;
                            app.file_listing_truncated = list.truncated;
                            app.status = "Workspace entries loaded".into();
                        }
                        Err(_) => app.status = "Invalid workspace listing".into(),
                    },
                    Err(error) => app.status = error,
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn selected_setup(&self) -> Option<workspaces::SetupProgress> {
        self.setup_progress.clone().or_else(|| {
            self.detail
                .thread
                .as_ref()
                .and_then(|thread| thread["activities"].as_array())
                .and_then(|activities| {
                    activities
                        .iter()
                        .rev()
                        .find(|activity| activity["kind"] == "worktree-setup")
                })
                .and_then(|activity| serde_json::from_value(activity["payload"].clone()).ok())
        })
    }

    fn new_thread(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(project) = self
            .shell
            .projects
            .iter()
            .find(|project| Some(&project.id) == self.selected_project.as_ref())
        else {
            self.status = "Select a project first".into();
            cx.notify();
            return;
        };
        let project_id = project.id.clone();
        let cwd = project.workspace_root.clone();
        let effective = project_actions::model_default(
            self.project_settings.as_ref(),
            &project.id,
            project.default_model_selection.as_ref(),
        );
        let previous = &effective;
        let instance = self.provider.read(cx).value();
        let model = self.model.read(cx).value();
        let selection = if instance.trim().is_empty() && model.trim().is_empty() {
            previous.clone()
        } else {
            match self.model_selection(previous, cx) {
                Ok(selection) => selection,
                Err(error) => {
                    self.status = error.into();
                    cx.notify();
                    return;
                }
            }
        };
        let title = self.title.read(cx).value().to_string();
        let mut command = match commands::create_thread(&project_id, &title, selection) {
            Ok(value) => value,
            Err(error) => {
                self.status = error.into();
                cx.notify();
                return;
            }
        };
        if let Some((id, branch, path)) = &self.new_thread_checkout
            && id == &project_id
        {
            command["branch"] = json!(branch);
            command["worktreePath"] = json!(path);
        }
        let prepare = if self.prepare_new_worktree {
            let base = self.worktree_ref.read(cx).value().trim().to_owned();
            if base.is_empty() {
                self.status = "Select a base branch for the new worktree".into();
                cx.notify();
                return;
            }
            let mut value = json!({"projectCwd":cwd,"baseBranch":base,"startFromOrigin":self.start_from_origin});
            let branch = self.worktree_branch.read(cx).value().trim().to_owned();
            if !branch.is_empty() {
                value["branch"] = json!(branch);
            }
            Some(value)
        } else {
            None
        };
        command["id"] = command["threadId"].clone();
        let draft = drafts::PendingThread {
            thread: serde_json::from_value(command.clone())
                .expect("native create command has shell fields"),
            created_at: command["createdAt"].as_str().unwrap().into(),
            prepare_worktree: prepare,
            run_setup_script: self.run_setup_script,
        };
        let id = draft.thread.id.clone();
        if let Some(store) = &mut self.draft_store {
            store.put_pending(&self.environment_scope, draft.clone());
        }
        self.pending_threads.push(draft);
        self.schedule_draft_save(cx);
        self.select_thread(&id, window, cx);
        self.status = "New thread draft · sent to the server with the first message".into();
        cx.notify();
    }

    fn choose_checkout(
        &mut self,
        branch: String,
        path: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(project) = self.selected_project.clone() else {
            return;
        };
        if let Some(previous) = &self.selected_thread {
            self.drafts
                .insert(previous.clone(), self.composer.read(cx).value().to_string());
        }
        self.selected_thread = None;
        self.detail = thread::ThreadDetail::default();
        self.worktree_path.update(cx, |input, cx| {
            input.set_value(path.clone().unwrap_or_default(), window, cx)
        });
        self.new_thread_checkout = Some((project, branch, path));
        self.composer
            .update(cx, |input, cx| input.set_value("", window, cx));
        if let Some(sender) = &self.commands {
            let _ = sender.try_send(connection::Command::SelectThread(None));
        }
        self.status = "Checkout selected for the next new thread".into();
        cx.notify();
    }

    fn update_draft_checkout(&mut self, cx: &mut Context<Self>) {
        if self.pending_actions > 0 {
            self.status = "Wait for the pending server receipt".into();
            cx.notify();
            return;
        }
        let Some(draft) = self.pending_threads.iter_mut().find(|draft| {
            Some(&draft.thread.id) == self.selected_thread.as_ref()
                && !self
                    .shell
                    .threads
                    .iter()
                    .any(|thread| thread.id == draft.thread.id)
        }) else {
            self.status = "Select an unsent draft to change its checkout".into();
            cx.notify();
            return;
        };
        let Some(project) = self
            .shell
            .projects
            .iter()
            .find(|project| project.id == draft.thread.project_id)
        else {
            return;
        };
        let base = self.worktree_ref.read(cx).value().trim().to_owned();
        if self.prepare_new_worktree && base.is_empty() {
            self.status = "Select a base branch for the new worktree".into();
            cx.notify();
            return;
        }
        draft.prepare_worktree = self.prepare_new_worktree.then(|| {
            let mut recipe = json!({"projectCwd":project.workspace_root,"baseBranch":base,"startFromOrigin":self.start_from_origin});
            let branch = self.worktree_branch.read(cx).value().trim().to_owned();
            if !branch.is_empty() { recipe["branch"] = json!(branch); }
            recipe
        });
        draft.run_setup_script = self.run_setup_script;
        draft.thread.branch = None;
        draft.thread.worktree_path = None;
        if !self.prepare_new_worktree
            && let Some((project_id, branch, path)) = &self.new_thread_checkout
            && project_id == &project.id
        {
            draft.thread.branch = Some(branch.clone());
            draft.thread.worktree_path = path.clone();
        }
        if let Some(store) = &mut self.draft_store {
            store.put_pending(&self.environment_scope, draft.clone());
        }
        self.setup_progress = None;
        self.schedule_draft_save(cx);
        self.status = "Draft checkout updated; message retained".into();
        cx.notify();
    }

    fn worktree_operation(&mut self, tag: &str, more: bool, cx: &mut Context<Self>) {
        if self.worktree_busy {
            self.status = "Wait for the worktree operation to finish".into();
            cx.notify();
            return;
        }
        let Some(project) = self
            .shell
            .projects
            .iter()
            .find(|p| Some(&p.id) == self.selected_project.as_ref())
        else {
            self.status = "Select a project first".into();
            cx.notify();
            return;
        };
        let Some(sender) = self.commands.clone() else {
            self.status = "Not connected".into();
            cx.notify();
            return;
        };
        let cwd = project.workspace_root.clone();
        let project_id = project.id.clone();
        let base = self.worktree_ref.read(cx).value().trim().to_owned();
        let path = self.worktree_path.read(cx).value().trim().to_owned();
        let payload = match tag {
            "vcs.listRefs" => {
                let mut payload = json!({"cwd":cwd,"refresh":!more,"limit":100});
                if more {
                    if let Some(cursor) = self.refs_cursor {
                        payload["cursor"] = json!(cursor);
                    } else {
                        return;
                    }
                }
                payload
            }
            "vcs.createWorktree" => match workspaces::create_worktree(
                &cwd,
                &base,
                self.worktree_branch.read(cx).value().as_str(),
                &path,
            ) {
                Ok(value) => value,
                Err(error) => {
                    self.status = error.into();
                    cx.notify();
                    return;
                }
            },
            "vcs.switchRef" if !base.is_empty() => {
                json!({"cwd":self.selected_workspace().unwrap_or(cwd),"refName":base})
            }
            "vcs.removeWorktree" if !path.is_empty() => {
                if self.confirm_worktree_removal.as_ref() != Some(&path) {
                    self.confirm_worktree_removal = Some(path.clone());
                    self.status =
                        "Click Remove again to confirm worktree removal; dirty files are preserved"
                            .into();
                    cx.notify();
                    return;
                }
                json!({"cwd":cwd,"path":path,"force":false})
            }
            _ => {
                self.status = "Enter a branch or worktree path".into();
                cx.notify();
                return;
            }
        };
        self.worktree_busy = true;
        self.worktree_epoch += 1;
        let epoch = self.worktree_epoch;
        let generation = self.generation;
        let tag = tag.to_owned();
        self.worktree_task = Some(cx.spawn(async move |entity, cx| {
            let result = connection::call(&sender, &tag, payload).await;
            let _ = entity.update(cx, |app, cx| {
                if app.generation != generation
                    || app.worktree_epoch != epoch
                    || app.selected_project.as_deref() != Some(&project_id)
                {
                    return;
                }
                app.worktree_busy = false;
                match result {
                    Ok(value) => {
                        if tag == "vcs.listRefs" {
                            match serde_json::from_value::<workspaces::Refs>(value) {
                                Ok(rows) => {
                                    if !more {
                                        app.refs.clear();
                                    }
                                    app.refs.extend(rows.refs);
                                    app.refs_cursor = rows.next_cursor;
                                    app.status = if rows.is_repo {
                                        "Branches loaded"
                                    } else {
                                        "This directory is not a repository"
                                    }
                                    .into();
                                }
                                Err(_) => app.status = "Invalid branch listing".into(),
                            }
                        } else {
                            app.confirm_worktree_removal = None;
                            app.status = "Worktree operation completed".into();
                            app.worktree_outcome = Some(value);
                            app.refs.clear();
                            app.refs_cursor = None;
                        }
                    }
                    Err(error) => app.status = error,
                };
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn selected_workspace(&self) -> Option<String> {
        let project = self
            .shell
            .projects
            .iter()
            .find(|p| Some(&p.id) == self.selected_project.as_ref())?;
        let selected = self
            .shell
            .threads
            .iter()
            .chain(self.archived_shell.threads.iter())
            .chain(self.pending_threads.iter().map(|draft| &draft.thread))
            .find(|t| Some(&t.id) == self.selected_thread.as_ref() && t.project_id == project.id);
        if selected.is_none()
            && let Some((id, _, Some(path))) = &self.new_thread_checkout
            && id == &project.id
        {
            return Some(path.clone());
        }
        Some(
            selected
                .and_then(|t| {
                    if let Some(detail) = self.detail.thread.as_ref().filter(|detail| {
                        detail["id"] == t.id && detail.get("worktreePath").is_some()
                    }) {
                        detail["worktreePath"].as_str().map(str::to_owned)
                    } else {
                        t.worktree_path.clone()
                    }
                })
                .unwrap_or_else(|| project.workspace_root.clone()),
        )
    }

    fn clone_project(&mut self, cx: &mut Context<Self>) {
        if self.pending_actions > 0 {
            self.status = "Wait for the pending server receipt".into();
            cx.notify();
            return;
        }
        match workspaces::clone_request(
            self.title.read(cx).value().as_str(),
            self.clone_remote.read(cx).value().as_str(),
            self.workspace.read(cx).value().as_str(),
        ) {
            Ok(payload) => self.perform("projectClone.start", payload, None, cx),
            Err(error) => {
                self.status = error.into();
                cx.notify();
            }
        }
    }

    fn search_contents(&mut self, cx: &mut Context<Self>) {
        let Some(sender) = self.commands.clone() else {
            self.status = "Not connected".into();
            cx.notify();
            return;
        };
        let Some(project) = self
            .selected_project
            .as_ref()
            .and_then(|id| self.shell.projects.iter().find(|project| &project.id == id))
        else {
            self.status = "Select a project first".into();
            cx.notify();
            return;
        };
        let payload = match self.content_options.request(
            &self
                .selected_workspace()
                .unwrap_or_else(|| project.workspace_root.clone()),
            self.content_query.read(cx).value().as_str(),
        ) {
            Ok(payload) => payload,
            Err(error) => {
                self.status = error.into();
                cx.notify();
                return;
            }
        };
        self.file_listing_epoch += 1;
        let epoch = self.file_listing_epoch;
        let generation = self.generation;
        let project_id = self.selected_project.clone();
        self.content_results = None;
        self.content_request_query = self.content_query.read(cx).value().to_string();
        self.content_index = 0;
        self.file_listing_task = Some(cx.spawn(async move |entity, cx| {
            let result = connection::call(&sender, "projects.searchContents", payload).await;
            let _ = entity.update(cx, |app, cx| {
                if app.generation != generation
                    || app.file_listing_epoch != epoch
                    || app.selected_project != project_id
                {
                    return;
                }
                match result {
                    Ok(value) => match serde_json::from_value::<files::ContentResults>(value) {
                        Ok(results) => {
                            app.status = results
                                .regex_fallback_error
                                .clone()
                                .map(|error| {
                                    format!("Regex failed; showing literal matches: {error}")
                                })
                                .unwrap_or_else(|| "Workspace content results loaded".into());
                            app.content_results = Some(results);
                        }
                        Err(_) => app.status = "Invalid workspace content results".into(),
                    },
                    Err(error) => app.status = error,
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn move_content_hit(&mut self, forward: bool, cx: &mut Context<Self>) {
        let count = self
            .content_results
            .as_ref()
            .map_or(0, |results| results.matches.len());
        if count > 0 {
            self.content_index = if forward {
                (self.content_index + 1) % count
            } else {
                (self.content_index + count - 1) % count
            };
            self.content_scroll.scroll_to_item(self.content_index);
            cx.notify();
        }
    }

    fn open_content_hit(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(hit) = self
            .content_results
            .as_ref()
            .and_then(|results| results.matches.get(index))
            .cloned()
        else {
            return;
        };
        self.content_index = index;
        self.file_path.update(cx, |input, cx| {
            input.set_value(hit.path.clone(), window, cx)
        });
        let column = hit.character_column();
        self.file_operation_at(
            false,
            Some(Position::new(
                hit.line_number.saturating_sub(1) as u32,
                column as u32,
            )),
            window,
            cx,
        );
    }

    fn file_operation(&mut self, write: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.file_operation_at(write, None, window, cx);
    }

    fn file_operation_at(
        &mut self,
        write: bool,
        position: Option<Position>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.file_busy {
            self.status = "A file operation is already pending".into();
            cx.notify();
            return;
        }
        let Some(sender) = self.commands.clone() else {
            self.status = "Not connected".into();
            cx.notify();
            return;
        };
        let text = self.file_contents.read(cx).value().to_string();
        let target = if write {
            if !self.file_editor.writable() {
                self.status = "Read a complete workspace-relative text file before saving".into();
                cx.notify();
                return;
            }
            if self.file_environment != self.environment_scope
                || self.file_editor.target.as_ref().is_none_or(|target| {
                    !self
                        .shell
                        .projects
                        .iter()
                        .any(|project| project.workspace_root == target.cwd)
                        && !self
                            .shell
                            .threads
                            .iter()
                            .chain(self.archived_shell.threads.iter())
                            .any(|thread| {
                                thread.worktree_path.as_deref() == Some(target.cwd.as_str())
                            })
                })
            {
                self.status = "The file belongs to a disconnected workspace".into();
                cx.notify();
                return;
            }
            self.file_editor.target.clone().unwrap()
        } else {
            if self.file_editor.dirty(&text) {
                self.status = "Unsaved file edits: save before reading another file".into();
                cx.notify();
                return;
            }
            let Some(project) = self
                .selected_project
                .as_ref()
                .and_then(|id| self.shell.projects.iter().find(|project| &project.id == id))
            else {
                self.status = "Select a project first".into();
                cx.notify();
                return;
            };
            let path = self.file_path.read(cx).value().trim().to_owned();
            if path.is_empty() {
                self.status = "Enter a file path".into();
                cx.notify();
                return;
            }
            files::Target {
                cwd: self
                    .selected_workspace()
                    .unwrap_or_else(|| project.workspace_root.clone()),
                path,
            }
        };
        self.file_busy = true;
        self.file_epoch += 1;
        let epoch = self.file_epoch;
        let generation = self.generation;
        let handle = window.window_handle();
        let payload = if write {
            json!({"cwd":target.cwd,"relativePath":target.path,"contents":text})
        } else {
            json!({"cwd":target.cwd,"relativePath":target.path})
        };
        cx.spawn(async move |entity,cx| {
            let result=connection::call(&sender,if write { "projects.writeFile" } else { "projects.readFile" },payload).await;
            let _=handle.update(cx,|_,window,cx|entity.update(cx,|app,cx| {
                if app.generation!=generation || app.file_epoch!=epoch { return; }
                app.file_busy=false;
                match result {
                    Ok(_) if write=>{ app.file_editor.saved=text;app.status="File saved".into(); }
                    Ok(value)=>match serde_json::from_value::<files::Read>(value) {
                        Ok(read)=>{
                            if app.file_contents.read(cx).value().as_str()!=text {
                                app.status="File response arrived after an edit; edits preserved. Save or discard before retrying.".into();cx.notify();return;
                            }
                            app.file_contents.update(cx,|input,cx| {
                                input.set_value(read.contents.clone(),window,cx);
                                if let Some(mut position) = position {
                                    let lines = read.contents.split('\n').collect::<Vec<_>>();
                                    position.line = position.line.min(lines.len().saturating_sub(1) as u32);
                                    position.character = position.character.min(lines[position.line as usize].chars().count() as u32);
                                    input.set_cursor_position(position,window,cx);
                                }
                            });
                            app.status=if read.truncated { "File truncated; saving disabled" } else { "File loaded" }.into();
                            app.file_environment=app.environment_scope.clone();
                            app.file_editor.loaded(target.cwd,read);
                        }
                        Err(_)=>app.status="Invalid file response; existing edits preserved".into(),
                    },
                    Err(error)=>app.status=error,
                }
                cx.notify();
            }));
        }).detach();
        cx.notify();
    }

    fn update_project_default(&mut self, clear: bool, cx: &mut Context<Self>) {
        let selection = if clear {
            None
        } else {
            let previous = self
                .selected_thread
                .as_ref()
                .and_then(|id| self.shell.threads.iter().find(|thread| &thread.id == id))
                .map(|thread| &thread.model_selection)
                .unwrap_or(&Value::Null);
            match self.model_selection(previous, cx) {
                Ok(selection) => Some(selection),
                Err(error) => {
                    self.status = error.into();
                    cx.notify();
                    return;
                }
            }
        };
        self.project_actions(
            Some(json!({"operation":"model-default","selection":selection})),
            cx,
        );
    }

    fn update_project(&mut self, remove: bool, cx: &mut Context<Self>) {
        let Some(id) = self.selected_project.clone() else {
            self.status = "Select a project first".into();
            cx.notify();
            return;
        };
        if remove && self.confirm_project_removal.as_ref() != Some(&id) {
            self.confirm_project_removal = Some(id);
            self.status="Remove project again to confirm. Project threads are removed; workspace files remain untouched.".into();
            cx.notify();
            return;
        }
        let result = if remove {
            commands::remove_project(&id)
        } else {
            commands::rename_project(&id, &self.title.read(cx).value())
        };
        match result {
            Ok(command) => {
                self.confirm_project_removal = None;
                self.dispatch(command, cx);
            }
            Err(error) => {
                self.status = error.into();
                cx.notify();
            }
        }
    }

    fn toggle_thread_property(&mut self, pin: bool, cx: &mut Context<Self>) {
        let selected = self.selected_thread.as_ref().and_then(|id| {
            self.shell
                .threads
                .iter()
                .chain(&self.archived_shell.threads)
                .find(|thread| &thread.id == id)
        });
        let kind = if pin {
            if selected.is_some_and(|thread| thread.pinned_at.is_some()) {
                "thread.unpin"
            } else {
                "thread.pin"
            }
        } else if selected
            .is_some_and(|thread| thread.settled_override.as_deref() == Some("settled"))
        {
            "thread.unsettle"
        } else {
            "thread.settle"
        };
        self.lifecycle(kind, cx);
    }

    fn disconnect(&mut self, cx: &mut Context<Self>) {
        self.connection_task = None;
        self.pairing_task = None;
        self.credential_task = None;
        self.commands = None;
        self.action_task = None;
        self.action_busy = false;
        self.project_settings = None;
        self.action_edit_id = None;
        self.confirm_action_delete = None;
        self.shell = shell::Shell::default();
        self.archived_shell = shell::Shell::default();
        self.detail = thread::ThreadDetail::default();
        self.selected_thread = None;
        self.pending_history_attached = false;
        self.selected_project = None;
        self.confirm_project_removal = None;
        self.pending_actions = 0;
        self.provider_catalog.clear();
        self.clone_progress.clear();
        self.file_epoch += 1;
        self.file_busy = false;
        self.file_listing_task = None;
        self.file_entries.clear();
        self.content_results = None;
        self.content_index = 0;
        self.file_listing_epoch += 1;
        if !self.file_editor.dirty(&self.file_contents.read(cx).value()) {
            self.file_editor = files::Editor::default();
        }
        self.snooze_wake = None;
        self.worktree_task = None;
        self.worktree_busy = false;
        self.worktree_epoch += 1;
        self.refs.clear();
        self.refs_cursor = None;
        self.worktree_outcome = None;
        self.new_thread_checkout = None;
        self.generation += 1;
        self.model_override = None;
        self.status = "Disconnected".into();
        cx.notify();
    }

    fn pair(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.disconnect(cx);
        let server = self.server.read(cx).value().to_string();
        let credential = self.pairing.read(cx).value().to_string();
        self.pairing
            .update(cx, |input, cx| input.set_value("", window, cx));
        let receiver = auth::pair(server, credential);
        let generation = self.generation;
        self.status = "Pairing…".into();
        self.pairing_task = Some(cx.spawn(async move |entity, cx| {
            if let Ok(result) = receiver.recv().await {
                let _ = entity.update(cx, |app, cx| {
                    if app.generation != generation {
                        return;
                    }
                    match result {
                        Ok(access)
                            if auth::origin(&app.server.read(cx).value()).ok().as_ref()
                                == Some(&access.origin) =>
                        {
                            app.access = Some(access);
                            app.connect(cx);
                        }
                        Ok(_) => app.status = "Server changed while pairing; pair again".into(),
                        Err(message) => app.status = message.into(),
                    }
                    cx.notify();
                });
            }
        }));
        cx.notify();
    }

    fn credential_operation(&mut self, operation: credentials::Operation, cx: &mut Context<Self>) {
        let loading = matches!(operation, credentials::Operation::Load);
        if loading {
            self.disconnect(cx);
        }
        let server = self.server.read(cx).value().to_string();
        let origin = match auth::origin(&server) {
            Ok(origin) => origin,
            Err(error) => {
                self.status = error.into();
                cx.notify();
                return;
            }
        };
        let receiver = match self.vault.request(&server, operation) {
            Ok(receiver) => receiver,
            Err(error) => {
                self.status = error.into();
                cx.notify();
                return;
            }
        };
        let generation = self.generation;
        self.status = "Updating Keychain…".into();
        self.credential_task = Some(cx.spawn(async move |entity, cx| {
            if let Ok(result) = receiver.recv().await {
                let _ = entity.update(cx, |app, cx| {
                    if generation != app.generation
                        || auth::origin(&app.server.read(cx).value()).ok().as_ref() != Some(&origin)
                    {
                        return;
                    }
                    match result {
                        Ok(Some(token)) if loading => {
                            match auth::Access::from_keychain(origin, token) {
                                Ok(access) => {
                                    app.access = Some(access);
                                    app.connect(cx);
                                }
                                Err(error) => app.status = error.into(),
                            }
                        }
                        Ok(None) if loading => {
                            app.status =
                                "No remembered credential for this server. Pair to connect.".into()
                        }
                        Ok(_) => app.status = "Keychain updated".into(),
                        Err(error) => app.status = error.into(),
                    }
                    cx.notify();
                });
            }
        }));
        cx.notify();
    }

    fn remember_credential(&mut self, cx: &mut Context<Self>) {
        let origin = auth::origin(&self.server.read(cx).value()).ok();
        if let Some(access) = self
            .access
            .as_ref()
            .filter(|access| Some(&access.origin) == origin.as_ref())
        {
            self.credential_operation(
                credentials::Operation::Remember(access.keychain_value()),
                cx,
            );
        } else {
            self.status = "Pair with this server before remembering its credential".into();
            cx.notify();
        }
    }

    fn forget_credential(&mut self, cx: &mut Context<Self>) {
        self.disconnect(cx);
        self.access = None;
        self.use_dev_credential = false;
        self.credential_operation(credentials::Operation::Forget, cx);
    }

    fn connect(&mut self, cx: &mut Context<Self>) {
        self.disconnect(cx);
        self.status = "Enter the server origin and pair to connect".into();
        let base = self.server.read(cx).value().to_string();
        let origin = auth::origin(&base).ok();
        let token = self
            .access
            .as_ref()
            .filter(|access| Some(&access.origin) == origin.as_ref())
            .map(|access| access.token.clone())
            .or_else(|| {
                if self.use_dev_credential
                    && std::env::var("T3_NATIVE_SERVER")
                        .ok()
                        .and_then(|base| auth::origin(&base).ok())
                        == origin
                {
                    std::env::var("T3_NATIVE_ACCESS_TOKEN").ok()
                } else {
                    None
                }
            });
        if let Some(token) = token {
            match connection::ConnectionOptions::new(&base, token) {
                Ok(options) => {
                    let options = options.with_proof_key(
                        self.access
                            .as_ref()
                            .filter(|access| Some(&access.origin) == origin.as_ref())
                            .and_then(|access| access.proof_key.clone()),
                    );
                    let scope = options.origin();
                    if scope != self.environment_scope {
                        self.drafts.clear();
                        self.stash_memory.clear();
                        self.search_results.clear();
                    }
                    self.pending_threads = self
                        .draft_store
                        .as_ref()
                        .map(|store| store.pending_threads(&scope))
                        .unwrap_or_default();
                    self.environment_scope = scope;
                    let session = connection::start_session(options);
                    self.commands = Some(session.commands);
                    self.connection_task = Some(cx.spawn(async move |entity, cx| {
                        while let Ok(update) = session.updates.recv().await {
                            let keep_running = entity.update(cx, |app, cx| {
                                let result = match update {
                                    connection::Update::Resynchronizing => {
                                        app.shell = shell::Shell::default();
                                        app.archived_shell = shell::Shell::default();
                                        app.action_task = None;
                                        app.action_busy = false;
                                        app.project_settings = None;
                                        app.action_edit_id = None;
                                        app.confirm_action_delete = None;
                                        app.detail = thread::ThreadDetail::default();
                                        app.snooze_wake = None;
                                        app.clone_progress.clear();
                                        app.setup_progress = None;
                                        app.pending_history_attached = false;
                                        app.status =
                                            "Reconnecting — history is being synchronized".into();
                                        Ok(())
                                    }
                                    connection::Update::Status(status) => {
                                        app.status = status;
                                        Ok(())
                                    }
                                    connection::Update::Shell(item) => {
                                        let was_synchronized = app.shell.synchronized;
                                        let result = app.shell.apply(*item).map_err(str::to_owned);
                                        app.attach_created_pending(cx);
                                        app.schedule_snooze_wake(cx);
                                        if app.shell.synchronized && !was_synchronized {
                                            app.perform("server.getConfig", json!({}), None, cx);
                                            if let Some(sender) = &app.commands {
                                                let _ = sender.try_send(
                                                    connection::Command::Subscribe {
                                                        key: "server-config".into(),
                                                        tag: "subscribeServerConfig".into(),
                                                        payload: json!({}),
                                                    },
                                                );
                                            }
                                            if app.clone_open
                                                && let Some(sender) = &app.commands
                                            {
                                                let _ = sender.try_send(
                                                    connection::Command::Subscribe {
                                                        key: "project-clones".into(),
                                                        tag: "subscribeProjectClones".into(),
                                                        payload: json!({}),
                                                    },
                                                );
                                            }
                                        }
                                        if app.shell.synchronized {
                                            app.status = "Connected · synchronized".into();
                                        }
                                        result
                                    }
                                    connection::Update::Subscription { key, item } => {
                                        if key == "server-config" {
                                            match item {
                                                Ok(value) => {
                                                    if value["type"] == "snapshot" {
                                                        app.project_settings = Some(
                                                            value["config"]["settings"].clone(),
                                                        );
                                                    } else if value["type"] == "settingsUpdated" {
                                                        app.project_settings = Some(
                                                            value["payload"]["settings"].clone(),
                                                        );
                                                    }
                                                    Ok(())
                                                }
                                                Err(error) => {
                                                    app.project_settings = None;
                                                    app.status = error;
                                                    Ok(())
                                                }
                                            }
                                        } else if key == "project-clones" && app.clone_open {
                                            match item {
                                                Ok(value) => workspaces::apply_clones(
                                                    &mut app.clone_progress,
                                                    value,
                                                ),
                                                Err(error) => {
                                                    app.status = error;
                                                    app.clone_progress.clear();
                                                    Ok(())
                                                }
                                            }
                                        } else if key == "worktree-setup" {
                                            match item {
                                                Ok(value) => {
                                                    if let Some(id) = &app.selected_thread {
                                                        workspaces::apply_setup(
                                                            &mut app.setup_progress,
                                                            id,
                                                            value,
                                                        )
                                                    } else {
                                                        Ok(())
                                                    }
                                                }
                                                Err(error) => {
                                                    app.status = error;
                                                    Ok(())
                                                }
                                            }
                                        } else {
                                            Ok(())
                                        }
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
        self.model_override = None;
        self.selected_thread = Some(id.into());
        self.pending_history_attached = false;
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
            .chain(self.pending_threads.iter().map(|draft| &draft.thread))
            .find(|t| t.id == id)
        {
            if self.selected_project.as_ref() != Some(&thread.project_id) {
                self.action_task = None;
                self.action_busy = false;
                self.action_edit_id = None;
                self.confirm_action_delete = None;
            }
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
        let pending = self
            .pending_threads
            .iter()
            .any(|draft| draft.thread.id == id)
            && !self.shell.threads.iter().any(|thread| thread.id == id);
        self.pending_history_attached = !pending;
        if pending
            && let Some(draft) = self
                .pending_threads
                .iter()
                .find(|draft| draft.thread.id == id)
        {
            self.prepare_new_worktree = draft.prepare_worktree.is_some();
            self.run_setup_script = draft.run_setup_script;
            if let Some(recipe) = &draft.prepare_worktree {
                self.start_from_origin = recipe["startFromOrigin"] == true;
                self.worktree_ref.update(cx, |input, cx| {
                    input.set_value(
                        recipe["baseBranch"].as_str().unwrap_or_default().to_owned(),
                        window,
                        cx,
                    )
                });
                self.worktree_branch.update(cx, |input, cx| {
                    input.set_value(
                        recipe["branch"].as_str().unwrap_or_default().to_owned(),
                        window,
                        cx,
                    )
                });
            }
        }
        self.setup_progress = None;
        if let Some(sender) = &self.commands {
            let _ = sender.try_send(connection::Command::Subscribe {
                key: "worktree-setup".into(),
                tag: "subscribeWorktreeSetup".into(),
                payload: json!({"threadId":id}),
            });
        }
        if let Some(sender) = &self.commands
            && sender
                .try_send(connection::Command::SelectThread(if pending {
                    None
                } else {
                    Some(id.into())
                }))
                .is_err()
        {
            self.status = "Connection busy or closed; retry selection".into();
        }
        cx.notify();
    }

    fn attach_created_pending(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.selected_thread.as_ref() else {
            return;
        };
        if !self
            .pending_threads
            .iter()
            .any(|draft| &draft.thread.id == id)
        {
            return;
        }
        let materialized = self.shell.threads.iter().any(|thread| &thread.id == id);
        if self.pending_history_attached && !materialized {
            // Bootstrap cancellation deletes the server thread but retains the local draft.
            // Its old starting session must not prevent sending that draft again.
            self.detail = thread::ThreadDetail::default();
            self.pending_history_attached = false;
            if let Some(sender) = &self.commands {
                let _ = sender.try_send(connection::Command::SelectThread(None));
            }
            cx.notify();
            return;
        }
        if self.pending_history_attached || !materialized {
            return;
        }
        if let Some(sender) = &self.commands
            && sender
                .try_send(connection::Command::SelectThread(Some(id.clone())))
                .is_ok()
        {
            self.pending_history_attached = true;
            cx.notify();
        }
    }

    fn project_actions(&mut self, mutation: Option<Value>, cx: &mut Context<Self>) {
        if self.action_busy {
            self.status = "Another action change is pending".into();
            cx.notify();
            return;
        }
        let Some(project) = self.selected_project.clone() else {
            self.status = "Select a project first".into();
            cx.notify();
            return;
        };
        let Some(sender) = self.commands.clone() else {
            self.status = "Not connected".into();
            cx.notify();
            return;
        };
        let legacy = self
            .shell
            .projects
            .iter()
            .find(|p| p.id == project)
            .map(|p| p.scripts.clone())
            .unwrap_or_default();
        let generation = self.generation;
        self.action_busy = true;
        self.status = "Reading project actions…".into();
        self.action_task = Some(cx.spawn(async move |entity, cx| {
            let saved_change = mutation.clone();
            let result = async {
                let settings = connection::call(&sender, "server.getSettings", json!({})).await?;
                let Some(change) = mutation else {
                    return Ok(settings);
                };
                if change["operation"] == "model-default" || change["operation"] == "model-inherit"
                {
                    return connection::call(
                        &sender,
                        "server.updateSettings",
                        project_actions::model_patch(
                            &settings,
                            &project,
                            if change["operation"] == "model-inherit" {
                                None
                            } else {
                                Some(change["selection"].clone())
                            },
                        ),
                    )
                    .await;
                }
                let current = project_actions::resolve(&settings, &project, &legacy);
                let scripts = match change["operation"].as_str() {
                    Some("inherit") => None,
                    Some("delete") => Some(
                        current
                            .into_iter()
                            .filter(|s| s["id"] != change["id"])
                            .collect(),
                    ),
                    Some("save") => Some(
                        project_actions::save(
                            &current,
                            change["previousId"].as_str(),
                            change["id"].as_str().unwrap_or_default(),
                            change["name"].as_str().unwrap_or_default(),
                            change["command"].as_str().unwrap_or_default(),
                            change["setup"] == true,
                            change["wait"] == true,
                        )
                        .map_err(str::to_owned)?,
                    ),
                    _ => return Err("Unknown project action operation".into()),
                };
                connection::call(
                    &sender,
                    "server.updateSettings",
                    project_actions::patch(&settings, &project, scripts),
                )
                .await
            }
            .await;
            let _ = entity.update(cx, |app, cx| {
                if app.generation != generation || app.selected_project.as_ref() != Some(&project) {
                    return;
                }
                app.action_busy = false;
                match result {
                    Ok(settings) => {
                        if let Some(change) = &saved_change {
                            if change["operation"] == "save"
                                && app.action_name.read(cx).value().as_str()
                                    == change["name"].as_str().unwrap_or_default()
                                && app.action_command.read(cx).value().as_str()
                                    == change["command"].as_str().unwrap_or_default()
                            {
                                app.action_edit_id = change["id"].as_str().map(str::to_owned);
                            } else if change["operation"] == "delete"
                                && app.action_edit_id.as_deref() == change["id"].as_str()
                            {
                                app.action_edit_id = None;
                            }
                        }
                        app.project_settings = Some(settings);
                        app.confirm_action_delete = None;
                        app.status = "Project actions loaded from this environment".into();
                    }
                    Err(error) => app.status = error,
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn save_project_action(&mut self, cx: &mut Context<Self>) {
        let name = self.action_name.read(cx).value().to_string();
        let command = self.action_command.read(cx).value().to_string();
        if name.trim().is_empty() || command.trim().is_empty() {
            self.status = "Action name and command are required".into();
            cx.notify();
            return;
        }
        self.project_actions(Some(json!({"operation":"save","previousId":self.action_edit_id,"id":self.action_edit_id.clone().unwrap_or_else(||uuid::Uuid::new_v4().to_string()),"name":name,"command":command,"setup":self.action_setup,"wait":self.action_wait})),cx);
    }

    fn perform(
        &mut self,
        tag: &str,
        payload: Value,
        sent_prompt: Option<(String, gpui_kit::AnyWindowHandle)>,
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
                        if sent_prompt.is_some()
                            && let Some(id) = &thread_id
                            && app
                                .pending_threads
                                .iter()
                                .any(|draft| &draft.thread.id == id)
                        {
                            app.pending_threads.retain(|draft| &draft.thread.id != id);
                            if let Some(store) = &mut app.draft_store {
                                store.remove_pending(&app.environment_scope, id);
                            }
                            app.schedule_draft_save(cx);
                            if app.selected_thread.as_ref() == Some(id)
                                && let Some(sender) = &app.commands
                            {
                                let _ = sender
                                    .try_send(connection::Command::SelectThread(Some(id.clone())));
                            }
                        }
                        if tag == "projectClone.start" {
                            app.selected_project = value["projectId"].as_str().map(str::to_owned);
                        }
                        if tag == "server.getSettings" {
                            app.project_settings = Some(value.clone());
                        }
                        if tag == "server.getConfig" {
                            match serde_json::from_value::<providers::Config>(value.clone()) {
                                Ok(config) => app.provider_catalog = config.providers,
                                Err(_) => {
                                    app.status = "Invalid provider catalog response".into();
                                    cx.notify();
                                    return;
                                }
                            }
                        }
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
                        app.status = if tag == "server.getConfig" {
                            format!(
                                "Connected · provider catalog loaded ({} instances)",
                                app.provider_catalog.len()
                            )
                        } else {
                            format!(
                                "Server accepted · sequence {}",
                                value["sequence"]
                                    .as_u64()
                                    .map(|s| s.to_string())
                                    .unwrap_or_else(|| "—".into())
                            )
                        };
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

    fn discard_pending(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = self.selected_thread.clone() else {
            return;
        };
        if self.confirm_delete.as_ref() != Some(&id) {
            self.confirm_delete = Some(id);
            self.status = "Click Delete again to discard this unsent thread".into();
            cx.notify();
            return;
        }
        self.pending_threads.retain(|draft| draft.thread.id != id);
        self.drafts.remove(&id);
        if let Some(store) = &mut self.draft_store {
            store.remove_pending(&self.environment_scope, &id);
            store.set(&self.environment_scope, &id, String::new());
        }
        self.selected_thread = None;
        self.confirm_delete = None;
        self.composer
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.setup_progress = None;
        if let Some(sender) = &self.commands {
            let _ = sender.try_send(connection::Command::Unsubscribe("worktree-setup".into()));
        }
        self.schedule_draft_save(cx);
        self.status = "Unsent thread discarded".into();
        cx.notify();
    }

    fn lifecycle(&mut self, kind: &str, cx: &mut Context<Self>) {
        let Some(id) = &self.selected_thread else {
            return;
        };
        if let Some(draft) = self
            .pending_threads
            .iter_mut()
            .find(|draft| &draft.thread.id == id)
        {
            if kind == "thread.meta.update" {
                let title = self.title.read(cx).value().trim().to_owned();
                if title.is_empty() {
                    self.status = "A title is required".into();
                    cx.notify();
                    return;
                }
                draft.thread.title = title;
                if let Some(store) = &mut self.draft_store {
                    store.put_pending(&self.environment_scope, draft.clone());
                }
                self.schedule_draft_save(cx);
                self.status = "Draft renamed".into();
            } else {
                self.status = "Send the first message before changing thread status".into();
            }
            cx.notify();
            return;
        }
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
                cx.background_executor().timer(delay).await;
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
            cx.background_executor()
                .timer(std::time::Duration::from_millis(200))
                .await;
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

    fn model_selection(&self, previous: &Value, cx: &App) -> Result<Value, &'static str> {
        let selected = commands::select_model(
            previous,
            &self.provider.read(cx).value(),
            &self.model.read(cx).value(),
        )?;
        Ok(self
            .model_override
            .as_ref()
            .filter(|value| {
                value["instanceId"] == selected["instanceId"] && value["model"] == selected["model"]
            })
            .cloned()
            .unwrap_or(selected))
    }

    fn choose_model_option(&mut self, id: &str, value: Value, cx: &mut Context<Self>) {
        let previous = self
            .selected_thread
            .as_ref()
            .and_then(|id| {
                self.shell
                    .threads
                    .iter()
                    .chain(self.pending_threads.iter().map(|draft| &draft.thread))
                    .find(|thread| &thread.id == id)
            })
            .map(|thread| &thread.model_selection)
            .unwrap_or(&Value::Null);
        let mut selection = match self.model_selection(previous, cx) {
            Ok(selection) => selection,
            Err(error) => {
                self.status = error.into();
                cx.notify();
                return;
            }
        };
        let descriptor = self
            .provider_catalog
            .iter()
            .find(|provider| provider.instance_id == selection["instanceId"])
            .and_then(|provider| {
                provider
                    .models
                    .iter()
                    .find(|model| model.slug == selection["model"])
            })
            .and_then(|model| model.capabilities["optionDescriptors"].as_array())
            .and_then(|options| options.iter().find(|option| option["id"] == id));
        let valid = descriptor.is_some_and(|descriptor| {
            (descriptor["type"] == "boolean" && value.is_boolean())
                || (descriptor["type"] == "select"
                    && descriptor["options"]
                        .as_array()
                        .is_some_and(|choices| choices.iter().any(|choice| choice["id"] == value)))
        });
        if !valid {
            self.status = "Model option is unavailable; refresh the catalog".into();
            cx.notify();
            return;
        }
        let mut options = selection["options"].as_array().cloned().unwrap_or_default();
        options.retain(|option| option["id"] != id);
        options.push(json!({"id":id,"value":value}));
        selection["options"] = json!(options);
        self.model_override = Some(selection.clone());
        if let Some(draft) = self
            .pending_threads
            .iter_mut()
            .find(|draft| Some(&draft.thread.id) == self.selected_thread.as_ref())
        {
            draft.thread.model_selection = selection;
            if let Some(store) = &mut self.draft_store {
                store.put_pending(&self.environment_scope, draft.clone());
            }
            self.schedule_draft_save(cx);
        }
        cx.notify();
    }

    fn pick_model(
        &mut self,
        instance: &str,
        model: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(provider) = self
            .provider_catalog
            .iter()
            .find(|provider| provider.instance_id == instance)
        else {
            self.status = "Provider instance no longer available".into();
            cx.notify();
            return;
        };
        if !provider.selectable() || !provider.models.iter().any(|entry| entry.slug == model) {
            self.status = "Provider or model is unavailable; refresh its catalog".into();
            cx.notify();
            return;
        }
        if provider.requires_new_thread_for_model_change
            && self.detail.thread.as_ref().is_some_and(|thread| {
                thread["messages"]
                    .as_array()
                    .is_some_and(|messages| !messages.is_empty())
                    && (thread["modelSelection"]["instanceId"] != instance
                        || thread["modelSelection"]["model"] != model)
            })
        {
            self.status = "This provider requires a new thread for a model change".into();
            cx.notify();
            return;
        }
        self.provider.update(cx, |input, cx| {
            input.set_value(instance.to_owned(), window, cx)
        });
        self.model.update(cx, |input, cx| {
            input.set_value(model.to_owned(), window, cx)
        });
        self.catalog_open = false;
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
        let Some(thread) = self
            .shell
            .threads
            .iter()
            .chain(self.pending_threads.iter().map(|draft| &draft.thread))
            .find(|t| t.id == id)
        else {
            return;
        };
        let text = self.composer.read(cx).value().to_string();
        let selection = match self.model_selection(&thread.model_selection, cx) {
            Ok(selection) => selection,
            Err(error) => {
                self.status = error.into();
                cx.notify();
                return;
            }
        };
        if self.provider_catalog.iter().any(|provider| {
            provider.instance_id == selection["instanceId"].as_str().unwrap_or("")
                && provider.requires_new_thread_for_model_change
        }) && selection != thread.model_selection
            && self.detail.thread.as_ref().is_some_and(|detail| {
                detail["messages"]
                    .as_array()
                    .is_some_and(|messages| !messages.is_empty())
            })
        {
            self.status = "This provider requires a new thread for a model change".into();
            cx.notify();
            return;
        }
        match commands::start_turn(
            &id,
            &text,
            selection,
            thread.runtime_mode.as_deref().unwrap_or("full-access"),
            thread.interaction_mode.as_deref().unwrap_or("default"),
        ) {
            Ok(mut payload) => {
                if let Some(draft) = self
                    .pending_threads
                    .iter()
                    .find(|draft| draft.thread.id == id)
                {
                    payload["bootstrap"] = json!({"createThread":{"projectId":draft.thread.project_id,"title":draft.thread.title,"modelSelection":payload["modelSelection"],"runtimeMode":payload["runtimeMode"],"interactionMode":payload["interactionMode"],"branch":draft.thread.branch,"worktreePath":draft.thread.worktree_path,"createdAt":draft.created_at},"runSetupScript":draft.run_setup_script});
                    if let Some(prepare) = &draft.prepare_worktree {
                        payload["bootstrap"]["prepareWorktree"] = prepare.clone();
                    }
                }
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
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Zed's One Dark surfaces, resolved once so the element tree reads as
        // one palette instead of a field of literals.
        let background = cx.theme().background;
        let foreground = cx.theme().foreground;
        let muted_foreground = cx.theme().muted_foreground;
        let border = cx.theme().border;
        let group_box = cx.theme().group_box;
        let muted = cx.theme().muted;
        let secondary = cx.theme().secondary;
        let warning = cx.theme().warning;
        let blue = cx.theme().blue;
        let green = cx.theme().green;
        let sidebar = cx.theme().sidebar;
        let sidebar_border = cx.theme().sidebar_border;
        let sidebar_foreground = cx.theme().sidebar_foreground;
        let danger = cx.theme().danger;
        let selection = cx.theme().selection;
        let popover = cx.theme().popover;

        let active_thread_title = self
            .detail
            .thread
            .as_ref()
            .and_then(|detail| detail["title"].as_str())
            .map(str::to_owned)
            .or_else(|| {
                self.selected_thread.as_ref().and_then(|id| {
                    self.shell
                        .threads
                        .iter()
                        .chain(self.pending_threads.iter().map(|draft| &draft.thread))
                        .find(|thread| &thread.id == id)
                        .map(|thread| thread.title.clone())
                })
            })
            .unwrap_or_else(|| "No thread selected".to_owned());

        let mut projects = Vec::new();
        for project in &self.shell.projects {
            let project_id = project.id.clone();
            let title = project.title.clone();
            let workspace = project.workspace_root.clone();
            let default_selection = project_actions::model_default(
                self.project_settings.as_ref(),
                &project.id,
                project.default_model_selection.as_ref(),
            );
            let project_selected = self.selected_project.as_ref() == Some(&project_id);
            let mut row = div()
                .flex()
                .flex_col()
                .gap_1()
                .child(
                    Button::new(gpui_kit::SharedString::from(format!("project-{project_id}")))
                        .label(title.clone())
                        .ghost()
                        .selected(project_selected)
                        .debug_selector({
                            let id = project_id.clone();
                            move || format!("project-{id}")
                        })
                        .on_click(cx.listener(move |app, _, window, cx| {
                            if app.selected_project.as_ref() != Some(&project_id) {
                                app.model_override = None;
                                app.provider.update(cx, |input, cx| {
                                    input.set_value(
                                        default_selection["instanceId"]
                                            .as_str()
                                            .unwrap_or_default()
                                            .to_owned(),
                                        window,
                                        cx,
                                    )
                                });
                                app.model.update(cx, |input, cx| {
                                    input.set_value(
                                        default_selection["model"]
                                            .as_str()
                                            .unwrap_or_default()
                                            .to_owned(),
                                        window,
                                        cx,
                                    )
                                });
                                app.action_task = None;
                                app.action_busy = false;
                                app.action_edit_id = None;
                                app.confirm_action_delete = None;
                                if let Some(previous) = app.selected_thread.take() {
                                    app.drafts.insert(
                                        previous.clone(),
                                        app.composer.read(cx).value().to_string(),
                                    );
                                    if let Some(store) = &mut app.draft_store {
                                        store.set(
                                            &app.environment_scope,
                                            &previous,
                                            app.composer.read(cx).value().to_string(),
                                        );
                                    }
                                    app.schedule_draft_save(cx);
                                }
                                app.detail = thread::ThreadDetail::default();
                                app.setup_progress = None;
                                app.confirm_delete = None;
                                app.composer
                                    .update(cx, |input, cx| input.set_value("", window, cx));
                                if let Some(sender) = &app.commands {
                                    let _ = sender.try_send(connection::Command::Unsubscribe(
                                        "worktree-setup".into(),
                                    ));
                                    let _ = sender.try_send(connection::Command::SelectThread(None));
                                }
                            }
                            app.selected_project = Some(project_id.clone());
                            app.worktree_task = None;
                            app.worktree_busy = false;
                            app.worktree_epoch += 1;
                            app.refs.clear();
                            app.worktree_outcome = None;
                            app.new_thread_checkout = None;
                            app.confirm_worktree_removal = None;
                            app.confirm_project_removal = None;
                            app.file_entries.clear();
                            app.file_listing_task = None;
                            app.file_listing_epoch += 1;
                            app.content_results = None;
                            app.title
                                .update(cx, |input, cx| input.set_value(title.clone(), window, cx));
                            app.workspace.update(cx, |input, cx| {
                                input.set_value(workspace.clone(), window, cx)
                            });
                            cx.notify();
                        })),
                );
            for draft in self.pending_threads.iter().filter(|draft| {
                draft.thread.project_id == project.id
                    && !self
                        .shell
                        .threads
                        .iter()
                        .any(|thread| thread.id == draft.thread.id)
            }) {
                let id = draft.thread.id.clone();
                row = row.child(div().pl_4().child(
                    Button::new(gpui_kit::SharedString::from(format!("draft-{id}")))
                        .label(format!("{} · unsent", draft.thread.title))
                        .ghost()
                        .selected(self.selected_thread.as_ref() == Some(&id))
                        .on_click(cx.listener(move |app, _, window, cx| {
                            app.select_thread(&id, window, cx)
                        })),
                ));
            }
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
                row = row.child(div().pl_4().child(
                    Button::new(gpui_kit::SharedString::from(format!("thread-{id}")))
                        .label(label)
                        .ghost()
                        .selected(self.selected_thread.as_ref() == Some(&id))
                        .on_click(cx.listener(move |app, _, window, cx| {
                            app.select_thread(&id, window, cx)
                        })),
                ));
            }
            projects.push(row);
        }

        let mut conversation = div()
            .id("conversation")
            .min_h(px(140.))
            .flex_shrink_0()
            .flex()
            .flex_col()
            .flex_1()
            .overflow_y_scroll()
            .gap_2()
            .p_3();
        if let Some(detail) = &self.detail.thread {
            let title = detail["title"].as_str().unwrap_or("Thread").to_owned();
            let branch = detail["branch"].as_str().unwrap_or("").to_owned();
            conversation = conversation.child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .pb_1()
                    .child(div().text_lg().font_weight(FontWeight::SEMIBOLD).child(title))
                    .when(!branch.is_empty(), |view| {
                        view.child(
                            div()
                                .px_2()
                                .rounded_sm()
                                .bg(muted)
                                .text_xs()
                                .text_color(muted_foreground)
                                .child(branch),
                        )
                    }),
            );

            if let Some((label, state)) = conversation::turn_summary(&detail["latestTurn"]) {
                let color = if state.is_danger() {
                    danger
                } else {
                    muted_foreground
                };
                conversation = conversation.child(div().text_xs().text_color(color).child(label));
            }

            if let Some(messages) = detail["messages"].as_array() {
                for message in messages {
                    let id = gpui_kit::SharedString::from(format!(
                        "message-{}",
                        message["id"].as_str().unwrap_or("unknown")
                    ));
                    let text = message["text"].as_str().unwrap_or("").to_owned();
                    let role = message["role"].as_str().unwrap_or("message");
                    let label = conversation::role_label(role);
                    let created = message["createdAt"].as_str().and_then(conversation::clock_time);
                    let meta = match (created, message["streaming"] == true) {
                        (Some(time), true) => format!("{label} · {time} · streaming"),
                        (Some(time), false) => format!("{label} · {time}"),
                        (None, true) => format!("{label} · streaming"),
                        (None, false) => label.to_owned(),
                    };
                    let body = div()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .p_2()
                        .rounded_md()
                        .child(div().text_xs().text_color(muted_foreground).child(meta))
                        .child(TextView::markdown(id, text));
                    let row = match role {
                        "user" => div().flex().justify_end().child(body.max_w(px(720.)).bg(secondary)),
                        "reasoning" => {
                            div().flex().justify_center().child(body.max_w(px(720.)).bg(muted))
                        }
                        "system" => div()
                            .flex()
                            .justify_center()
                            .child(body.text_color(muted_foreground)),
                        _ => body.bg(group_box),
                    };
                    conversation = conversation.child(row);
                }
            }

            if let Some(activities) = detail["activities"].as_array() {
                let visible: Vec<&Value> = activities
                    .iter()
                    .filter(|activity| activity["kind"] != "worktree-setup")
                    .collect();
                if !visible.is_empty() {
                    let mut log = div().flex().flex_col().gap_1().pt_1();
                    for activity in visible {
                        let color = match activity["tone"].as_str().unwrap_or("info") {
                            "error" => danger,
                            "approval" => warning,
                            "tool" => muted_foreground,
                            _ => blue,
                        };
                        log = log.child(
                            div()
                                .flex()
                                .items_center()
                                .gap_2()
                                .text_xs()
                                .text_color(muted_foreground)
                                .child(div().w(px(6.)).h(px(6.)).rounded_full().bg(color))
                                .child(activity["summary"].as_str().unwrap_or("").to_owned()),
                        );
                    }
                    conversation = conversation.child(log);
                }
            }

            if let Some(plans) = detail["proposedPlans"].as_array() {
                for plan in plans {
                    let markdown = plan["planMarkdown"].as_str().unwrap_or("").to_owned();
                    if markdown.is_empty() {
                        continue;
                    }
                    let id = gpui_kit::SharedString::from(format!(
                        "plan-{}",
                        plan["id"].as_str().unwrap_or("unknown")
                    ));
                    let implemented = !plan["implementedAt"].is_null();
                    conversation = conversation.child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .p_3()
                            .rounded_md()
                            .bg(group_box)
                            .border_1()
                            .border_color(border)
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_2()
                                    .text_xs()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(muted_foreground)
                                    .child("PROPOSED PLAN")
                                    .when(implemented, |view| {
                                        view.child(
                                            div().px_2().rounded_sm().bg(muted).child("implemented"),
                                        )
                                    }),
                            )
                            .child(TextView::markdown(id, markdown)),
                    );
                }
            }

            if let Some(checkpoint) = detail["checkpoints"].as_array().and_then(|list| list.last()) {
                let files = checkpoint["files"].as_array().cloned().unwrap_or_default();
                if !files.is_empty() {
                    let (additions, deletions) = conversation::diff_totals(&files);
                    let mut card = div()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .p_3()
                        .rounded_md()
                        .bg(group_box)
                        .border_1()
                        .border_color(border)
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap_2()
                                .text_xs()
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(muted_foreground)
                                .child("CHANGES")
                                .child(format!("{} files", files.len()))
                                .child(div().text_color(green).child(format!("+{additions}")))
                                .child(div().text_color(danger).child(format!("-{deletions}"))),
                        );
                    for file in files.iter().take(8) {
                        card = card.child(
                            div()
                                .flex()
                                .justify_between()
                                .gap_2()
                                .text_xs()
                                .text_color(muted_foreground)
                                .child(
                                    div()
                                        .min_w_0()
                                        .truncate()
                                        .child(file["path"].as_str().unwrap_or("").to_owned()),
                                )
                                .child(format!(
                                    "+{} -{}",
                                    file["additions"].as_u64().unwrap_or(0),
                                    file["deletions"].as_u64().unwrap_or(0)
                                )),
                        );
                    }
                    conversation = conversation.child(card);
                }
            }

            if let Some(error) = detail["session"]["lastError"].as_str() {
                conversation = conversation.child(div().text_xs().text_color(danger).child(error.to_owned()));
            }
        } else if self
            .pending_threads
            .iter()
            .any(|draft| Some(&draft.thread.id) == self.selected_thread.as_ref())
        {
            let phase = self.selected_setup().map(|setup| setup.phase.clone());
            conversation = conversation.child(match phase.as_deref() {
                Some("running") => "Preparing worktree · your first message is waiting for setup",
                Some("cancelled") => {
                    "Setup cancelled · your draft and message are retained for retry"
                }
                Some("failed") => "Setup failed · your draft and message are retained for retry",
                _ => "New thread draft · send a message to start the conversation",
            });
            if phase.is_some() {
                conversation = conversation.child(self.composer.read(cx).value().to_string());
            }
        } else if self.selected_thread.is_some() {
            conversation = conversation.child(
                div().text_color(muted_foreground).child("Loading thread history…"),
            );
        } else {
            conversation = conversation.child(
                div()
                    .flex()
                    .flex_col()
                    .items_center()
                    .justify_center()
                    .gap_2()
                    .h(px(360.))
                    .text_color(muted_foreground)
                    .child(
                        div()
                            .text_lg()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(foreground)
                            .child("Select a thread"),
                    )
                    .child("Choose a thread in the sidebar, or start a new one with ⌘N"),
            );
        }

        let mut search_results = div().flex().flex_col().gap_1();
        for result in &self.search_results {
            if let Some(id) = result["threadId"].as_str() {
                let id = id.to_owned();
                search_results = search_results.child(
                    Button::new(gpui_kit::SharedString::from(format!("search-{id}")))
                        .label(result["snippet"].as_str().unwrap_or("Match").to_owned())
                        .ghost()
                        .on_click(cx.listener(move |app, _, window, cx| {
                            app.select_thread(&id, window, cx)
                        })),
                );
            }
        }

        let palette = self.palette_open.then(|| {
            let entries = palette::matching(
                &self.palette_query.read(cx).value(),
                self.selected_thread.is_some(),
            );
            div()
                .id("command-palette")
                .key_context("NativePalette")
                .absolute()
                .top(px(72.))
                .left(px(320.))
                .w(px(640.))
                .max_h(px(600.))
                .overflow_y_scroll()
                .flex()
                .flex_col()
                .gap_2()
                .p_3()
                .rounded_lg()
                .bg(popover)
                .border_1()
                .border_color(border)
                .shadow_lg()
                .child(
                    div()
                        .flex()
                        .justify_between()
                        .items_center()
                        .text_sm()
                        .text_color(muted_foreground)
                        .child("Commands")
                        .child(Button::new("close-palette").label("Close").ghost().small().on_click(
                            cx.listener(|app, _, window, cx| app.close_palette(window, cx)),
                        )),
                )
                .child(Input::new(&self.palette_query))
                .when(entries.is_empty(), |view| {
                    view.child(
                        div()
                            .text_color(muted_foreground)
                            .child("No matching commands"),
                    )
                })
                .children(entries.into_iter().enumerate().map(|(index, entry)| {
                    let command = entry.command;
                    Button::new(gpui_kit::SharedString::from(format!("palette-{index}")))
                        .debug_selector(move || format!("palette-{index}"))
                        .ghost()
                        .selected(index == self.palette_index)
                        .label(format!(
                            "{}{}",
                            if index == self.palette_index {
                                "› "
                            } else {
                                ""
                            },
                            entry.label
                        ))
                        .on_click(cx.listener(move |app, _, window, cx| {
                            app.close_palette(window, cx);
                            app.execute_command(command, window, cx);
                        }))
                }))
        });

        let title_bar_row = TitleBar::new()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .min_w_0()
                    .child(div().font_weight(FontWeight::SEMIBOLD).child("T3 Code"))
                    .child(
                        div()
                            .text_sm()
                            .text_color(muted_foreground)
                            .truncate()
                            .child(active_thread_title),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(
                        div()
                            .text_sm()
                            .text_color(muted_foreground)
                            .max_w(px(360.))
                            .truncate()
                            .child(self.status.clone()),
                    )
                    .child(
                        Button::new("commands")
                            .debug_selector(|| "commands".into())
                            .label("Commands ⌘K")
                            .ghost()
                            .small()
                            .on_click(
                                cx.listener(|app, _, window, cx| app.toggle_palette(window, cx)),
                            ),
                    ),
            );

        div()
            .relative()
            .key_context("NativeApp")
            .track_focus(&self.focus)
            .on_action(cx.listener(|app, _: &ContentSubmit, window, cx| {
                if app.content_results.as_ref().is_some_and(|results| !results.matches.is_empty()) { app.open_content_hit(app.content_index,window,cx); } else { app.search_contents(cx); }
            }))
            .on_action(cx.listener(|app, _: &ContentNext, _, cx| app.move_content_hit(true,cx)))
            .on_action(cx.listener(|app, _: &ContentPrevious, _, cx| app.move_content_hit(false,cx)))
            .on_action(cx.listener(|app, _: &OpenFiles, window, cx| app.execute_command(palette::Command::Files,window,cx)))
            .on_action(cx.listener(|app, _: &OpenContentSearch, window, cx| app.execute_command(palette::Command::ContentSearch,window,cx)))
            .on_action(
                cx.listener(|app, _: &TogglePalette, window, cx| app.toggle_palette(window, cx)),
            )
            .on_action(
                cx.listener(|app, _: &ClosePalette, window, cx| app.close_palette(window, cx)),
            )
            .on_action(cx.listener(|app, _: &PaletteNext, _, cx| app.move_palette(true, cx)))
            .on_action(cx.listener(|app, _: &PalettePrevious, _, cx| app.move_palette(false, cx)))
            .on_action(cx.listener(|app, _: &NewThread, window, cx| {
                app.execute_command(palette::Command::NewThread, window, cx)
            }))
            .on_action(cx.listener(|app, _: &StashPrompt, window, cx| {
                app.execute_command(palette::Command::Stash, window, cx)
            }))
            .on_action(
                cx.listener(|app, _: &ToggleSettle, _, cx| app.toggle_thread_property(false, cx)),
            )
            .on_action(
                cx.listener(|app, _: &TogglePin, _, cx| app.toggle_thread_property(true, cx)),
            )
            .flex()
            .flex_col()
            .size_full()
            .bg(background)
            .text_color(foreground)
            .child(title_bar_row)
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_h_0()
                    // Sidebar
                    .child(
                        div()
                            .id("sidebar")
                            .flex()
                            .flex_col()
                            .w(px(288.))
                            .flex_shrink_0()
                            .h_full()
                            .overflow_y_scroll()
                            .p_3()
                            .gap_2()
                            .bg(sidebar)
                            .text_color(sidebar_foreground)
                            .border_r_1()
                            .border_color(sidebar_border)
                            .child(
                                div()
                                    .px_1()
                                    .pb_1()
                                    .text_xs()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(muted_foreground)
                                    .child("CONNECTION"),
                            )
                            .child(Input::new(&self.server))
                            .child(Input::new(&self.pairing))
                            .child(
                                div().flex().flex_wrap().gap_2()
                                    .child(
                                        Button::new("pair")
                                            .debug_selector(|| "pair".into())
                                            .label("Pair and connect")
                                            .primary()
                                            .on_click(cx.listener(|app, _, window, cx| app.pair(window, cx))),
                                    )
                                    .child(
                                        Button::new("remember-credential")
                                            .debug_selector(|| "remember-credential".into())
                                            .label("Remember in Keychain")
                                            .ghost()
                                            .on_click(cx.listener(|app, _, _, cx| app.remember_credential(cx))),
                                    )
                                    .child(
                                        Button::new("load-credential")
                                            .debug_selector(|| "load-credential".into())
                                            .label("Connect remembered")
                                            .ghost()
                                            .on_click(cx.listener(|app, _, _, cx| {
                                                app.credential_operation(credentials::Operation::Load, cx)
                                            })),
                                    )
                                    .child(
                                        Button::new("forget-credential")
                                            .debug_selector(|| "forget-credential".into())
                                            .label("Forget credential")
                                            .ghost()
                                            .on_click(cx.listener(|app, _, _, cx| app.forget_credential(cx))),
                                    )
                                    .child(
                                        Button::new("connect")
                                            .label("Connect / retry")
                                            .secondary()
                                            .on_click(cx.listener(|app, _, _, cx| app.connect(cx))),
                                    )
                                    .child(
                                        Button::new("disconnect")
                                            .label("Disconnect")
                                            .ghost()
                                            .on_click(cx.listener(|app, _, _, cx| app.disconnect(cx))),
                                    ),
                            )
                            .child(
                                div()
                                    .px_1()
                                    .pt_2()
                                    .pb_1()
                                    .text_xs()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(muted_foreground)
                                    .child("THREADS"),
                            )
                            .child(
                                div().flex().gap_2()
                                    .child(div().flex_1().min_w_0().child(Input::new(&self.search)))
                                    .child(
                                        Button::new("search")
                                            .label("Search")
                                            .secondary()
                                            .on_click(cx.listener(|app, _, _, cx| app.search_threads(cx))),
                                    ),
                            )
                            .child(search_results)
                            .child(
                                div().flex().flex_wrap().gap_1().children(
                                    [
                                        ("active-view", "Active", sidebar::View::Active),
                                        ("settled-view", "Settled", sidebar::View::Settled),
                                        ("snoozed-view", "Snoozed", sidebar::View::Snoozed),
                                        ("archived-view", "Archived", sidebar::View::Archived),
                                    ]
                                    .into_iter()
                                    .map(|(id, label, view)| {
                                        Button::new(id)
                                            .debug_selector(move || id.into())
                                            .label(label)
                                            .ghost()
                                            .small()
                                            .selected(self.sidebar_view == view)
                                            .on_click(cx.listener(move |app, _, _, cx| {
                                                app.sidebar_view = view;
                                                if view == sidebar::View::Archived {
                                                    app.perform(
                                                        "orchestration.getArchivedShellSnapshot",
                                                        json!({}),
                                                        None,
                                                        cx,
                                                    );
                                                }
                                                cx.notify();
                                            }))
                                    }),
                                ),
                            )
                            .child(div().flex().flex_col().gap_1().children(projects)),
                    )
                    // Workspace
                    .child(
                        div()
                            .id("native-workspace")
                            .min_w_0()
                            .overflow_y_scroll()
                            .flex()
                            .flex_col()
                            .flex_1()
                            .h_full()
                            .p_3()
                            .gap_2()
                            .children(
                                self.storage_error
                                    .clone()
                                    .map(|error| div().text_color(danger).child(error)),
                            )
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .gap_2()
                                    .p_2()
                                    .rounded_md()
                                    .bg(group_box)
                                    .border_1()
                                    .border_color(border)
                                    .child(
                                        div()
                                            .text_xs()
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .text_color(muted_foreground)
                                            .child("PROJECT"),
                                    )
                                    .child(
                                        div()
                                            .flex()
                                            .gap_2()
                                            .child(Input::new(&self.title))
                                            .child(Input::new(&self.workspace)),
                                    )
                                    .child(
                                        div()
                                            .flex()
                                            .flex_wrap()
                                            .gap_2()
                                            .child(Button::new("add-project").label("Add project").primary().on_click(
                                                cx.listener(|app, _, window, cx| {
                                                    app.execute_command(palette::Command::AddProject, window, cx)
                                                }),
                                            ))
                                            .child(
                                                Button::new("rename-project")
                                                    .debug_selector(|| "rename-project".into())
                                                    .label("Rename project")
                                                    .ghost()
                                                    .on_click(
                                                        cx.listener(|app, _, _, cx| app.update_project(false, cx)),
                                                    ),
                                            )
                                            .child(
                                                Button::new("remove-project")
                                                    .debug_selector(|| "remove-project".into())
                                                    .label("Remove project")
                                                    .danger()
                                                    .outline()
                                                    .on_click(
                                                        cx.listener(|app, _, _, cx| app.update_project(true, cx)),
                                                    ),
                                            )
                                            .child(Button::new("new-thread").label("New thread").secondary().on_click(
                                                cx.listener(|app, _, window, cx| {
                                                    app.execute_command(palette::Command::NewThread, window, cx)
                                                }),
                                            ))
                                            .child(
                                                Button::new("rename")
                                                    .debug_selector(|| "rename".into())
                                                    .label("Rename thread")
                                                    .ghost()
                                                    .on_click(cx.listener(|app, _, _, cx| {
                                                        app.lifecycle("thread.meta.update", cx)
                                                    })),
                                            ),
                                    ),
                            )
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .gap_2()
                                    .p_2()
                                    .rounded_md()
                                    .bg(group_box)
                                    .border_1()
                                    .border_color(border)
                                    .child(
                                        div()
                                            .text_xs()
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .text_color(muted_foreground)
                                            .child("THREAD"),
                                    )
                                    .child(
                                        div().flex().flex_wrap().gap_2().children(
                                            [
                                                ("archive", "Archive", "thread.archive"),
                                                ("unarchive", "Unarchive", "thread.unarchive"),
                                                ("settle", "Settle", "thread.settle"),
                                                ("unsettle", "Unsettle", "thread.unsettle"),
                                                ("snooze", "Snooze 1h", "thread.snooze"),
                                                ("unsnooze", "Unsnooze", "thread.unsnooze"),
                                                ("pin", "Pin", "thread.pin"),
                                                ("unpin", "Unpin", "thread.unpin"),
                                                ("delete", "Delete thread", "thread.delete"),
                                            ]
                                            .into_iter()
                                            .map(|(id, label, kind)| {
                                                Button::new(id)
                                                    .debug_selector(move || id.into())
                                                    .label(label)
                                                    .ghost()
                                                    .on_click(
                                                        cx.listener(move |app, _, window, cx| {if kind=="thread.delete" && app.pending_threads.iter().any(|draft|Some(&draft.thread.id)==app.selected_thread.as_ref()){app.discard_pending(window,cx);}else{app.lifecycle(kind,cx);}}),
                                                    )
                                            }),
                                        ),
                                    )
                                    .child(
                                        div()
                                            .flex()
                                            .gap_2()
                                            .child(Input::new(&self.custom_snooze))
                                            .child(
                                                Button::new("custom-snooze")
                                                    .label("Custom snooze")
                                                    .secondary()
                                                    .on_click(cx.listener(|app, _, _, cx| app.snooze_custom(cx))),
                                            ),
                                    ),
                            )
                            .when_some(self.selected_setup(),|view,setup| {
                                let id=setup.thread_id.clone();let show=self.setup_expanded;
                                view.child(div().w_full().min_w_0().whitespace_normal().flex().flex_col().gap_1().p_2().rounded_md().bg(group_box).border_1().border_color(border)
                                    .child(format!("Worktree setup · {}{}",setup.phase,setup.branch.as_ref().map_or(String::new(),|branch|format!(" · {branch}"))))
                                    .child(Button::new("setup-details").ghost().label(if show{"Hide setup details"}else{"Show setup details"}).on_click(cx.listener(|app,_,_,cx|{app.setup_expanded= !app.setup_expanded;cx.notify();})))
                                    .when_some(setup.worktree_path.clone(),|view,path|view.child(Button::new("copy-worktree-path").ghost().label("Copy worktree path").tooltip(path.clone()).on_click(cx.listener(move|_,_,_,cx|cx.write_to_clipboard(gpui_kit::ClipboardItem::new_string(path.clone()))))))
                                    .children(setup.error.clone())
                                    .when(show,|view|view.children(setup.stages.iter().map(|stage|div().w_full().min_w_0().whitespace_normal().flex().flex_col().text_color(muted_foreground).child(format!("{} · {}{}",stage.id,stage.status,stage.percent.map_or(String::new(),|percent|format!(" {percent}%")))).children(stage.detail.clone()).children(stage.tail.clone()))))
                                    .when(setup.phase=="running",|view|view.child(Button::new("setup-cancel").danger().outline().label("Cancel worktree setup").on_click(cx.listener(move|app,_,_,cx|app.perform("worktreeSetup.cancel",json!({"threadId":id}),None,cx)))))
                                )
                            })
                            .child(Button::new("project-actions").label("Project actions").secondary().on_click(cx.listener(|app,_,_,cx|{app.project_actions_open=true;app.project_actions(None,cx);})))
                            .when(self.project_actions_open,|view| {
                                let scripts=self.selected_project.as_ref().and_then(|id|self.project_settings.as_ref().map(|settings|project_actions::resolve(settings,id,self.shell.projects.iter().find(|p|&p.id==id).map(|p|p.scripts.as_slice()).unwrap_or_default()))).unwrap_or_default();
                                view.child(div().flex().flex_col().gap_2().min_w_0().p_2().rounded_md().bg(group_box).border_1().border_color(border)
                                    .child(div().text_xs().font_weight(FontWeight::SEMIBOLD).text_color(muted_foreground).child("PROJECT ACTIONS"))
                                    .child("Actions for the selected project on this environment")
                                    .child(Input::new(&self.action_name)).child(Input::new(&self.action_command))
                                    .child(Button::new("action-setup").ghost().selected(self.action_setup).label(if self.action_setup{"Runs on worktree creation"}else{"Run manually"}).on_click(cx.listener(|app,_,_,cx|{app.action_setup= !app.action_setup;cx.notify();})))
                                    .child(Button::new("action-wait").ghost().selected(self.action_wait).label(if self.action_wait{"Wait before starting agent"}else{"Start agent while setup runs"}).on_click(cx.listener(|app,_,_,cx|{app.action_wait= !app.action_wait;cx.notify();})))
                                    .child(Button::new("action-save").debug_selector(||"action-save".into()).label("Save action").primary().disabled(self.action_busy).on_click(cx.listener(|app,_,_,cx|app.save_project_action(cx))))
                                    .child(Button::new("action-new").label("New action").secondary().on_click(cx.listener(|app,_,window,cx|{app.action_edit_id=None;app.confirm_action_delete=None;app.action_setup=false;app.action_wait=false;app.action_name.update(cx,|input,cx|input.set_value("",window,cx));app.action_command.update(cx,|input,cx|input.set_value("",window,cx));cx.notify();})))
                                    .children(scripts.into_iter().enumerate().map(|(index,script)| {
                                        let edit=script.clone();let id=script["id"].as_str().unwrap_or_default().to_owned();let confirm=self.confirm_action_delete.as_ref()==Some(&id);
                                        div().flex().flex_col().gap_1().min_w_0().whitespace_normal().child(format!("{} · {}{}",script["name"].as_str().unwrap_or_default(),script["command"].as_str().unwrap_or_default(),if script["runOnWorktreeCreate"]==true{" · worktree setup"}else{""}))
                                            .child(Button::new(gpui_kit::SharedString::from(format!("action-edit-{index}"))).label("Edit action").ghost().on_click(cx.listener(move|app,_,window,cx|{app.action_edit_id=edit["id"].as_str().map(str::to_owned);app.action_setup=edit["runOnWorktreeCreate"]==true;app.action_wait=edit["async"]==false;app.action_name.update(cx,|input,cx|input.set_value(edit["name"].as_str().unwrap_or_default().to_owned(),window,cx));app.action_command.update(cx,|input,cx|input.set_value(edit["command"].as_str().unwrap_or_default().to_owned(),window,cx));cx.notify();})))
                                            .child(Button::new(gpui_kit::SharedString::from(format!("action-delete-{index}"))).label(if confirm{"Confirm delete action"}else{"Delete action"}).danger().outline().disabled(self.action_busy).on_click(cx.listener(move|app,_,_,cx|{if app.confirm_action_delete.as_ref()==Some(&id){app.project_actions(Some(json!({"operation":"delete","id":id})),cx);}else{app.confirm_action_delete=Some(id.clone());cx.notify();}})))
                                    }))
                                    .child(Button::new("project-default-save").label("Use selected provider and model for new threads").secondary().on_click(cx.listener(|app,_,_,cx|app.update_project_default(false,cx))))
                                    .child(Button::new("project-default-inherit").label("Use environment model default").ghost().on_click(cx.listener(|app,_,_,cx|app.project_actions(Some(json!({"operation":"model-inherit"})),cx))))
                                    .child(Button::new("project-default-clear").label("Clear project model default").ghost().on_click(cx.listener(|app,_,_,cx|app.update_project_default(true,cx))))
                                    .child(Button::new("actions-refresh").label("Refresh actions").secondary().disabled(self.action_busy).on_click(cx.listener(|app,_,_,cx|app.project_actions(None,cx))))
                                    .child(Button::new("actions-inherit").label("Use environment action defaults").secondary().disabled(self.action_busy).on_click(cx.listener(|app,_,_,cx|app.project_actions(Some(json!({"operation":"inherit"})),cx))))
                                    .child(Button::new("actions-close").label("Close actions").ghost().on_click(cx.listener(|app,_,_,cx|{app.project_actions_open=false;cx.notify();}))))
                            })
                            .child(Button::new("worktrees").label("Branches and worktrees").secondary().on_click(cx.listener(|app,_,window,cx|app.execute_command(palette::Command::Worktrees,window,cx))))
                            .when(self.worktrees_open,|view|view.child(div().flex().flex_col().gap_2().p_2().rounded_md().bg(group_box).border_1().border_color(border)
                                .child(div().text_xs().font_weight(FontWeight::SEMIBOLD).text_color(muted_foreground).child("BRANCHES"))
                                .child(Button::new("prepare-worktree").ghost().selected(self.prepare_new_worktree).label(if self.prepare_new_worktree{"Next new thread: new worktree"}else{"Next new thread: selected checkout"}).on_click(cx.listener(|app,_,_,cx|{app.prepare_new_worktree= !app.prepare_new_worktree;cx.notify();})))
                                .child(Button::new("worktree-origin").ghost().selected(self.start_from_origin).label(if self.start_from_origin{"Base: fetch from origin"}else{"Base: local branch"}).on_click(cx.listener(|app,_,_,cx|{app.start_from_origin= !app.start_from_origin;cx.notify();})))
                                .child(Button::new("worktree-setup-script").ghost().selected(self.run_setup_script).label(if self.run_setup_script{"Run setup script"}else{"Skip setup script"}).on_click(cx.listener(|app,_,_,cx|{app.run_setup_script= !app.run_setup_script;cx.notify();})))
                                .child(Input::new(&self.worktree_ref)).child(Input::new(&self.worktree_branch)).child(Input::new(&self.worktree_path))
                                .child(Button::new("draft-checkout-update").debug_selector(||"draft-checkout-update".into()).label("Update selected draft checkout").secondary().on_click(cx.listener(|app,_,_,cx|app.update_draft_checkout(cx))))
                                .child(div().flex().flex_wrap().gap_2()
                                    .child(Button::new("refs-refresh").label("Refresh branches").secondary().on_click(cx.listener(|app,_,_,cx|app.worktree_operation("vcs.listRefs",false,cx))))
                                    .child(Button::new("worktree-create").label("Create worktree").primary().on_click(cx.listener(|app,_,_,cx|app.worktree_operation("vcs.createWorktree",false,cx))))
                                    .child(Button::new("ref-switch").label("Switch branch").secondary().on_click(cx.listener(|app,_,_,cx|app.worktree_operation("vcs.switchRef",false,cx))))
                                    .child(Button::new("worktree-remove").label("Remove worktree").danger().outline().on_click(cx.listener(|app,_,_,cx|app.worktree_operation("vcs.removeWorktree",false,cx))))
                                    .child(Button::new("worktrees-close").label("Close").ghost().on_click(cx.listener(|app,_,_,cx|{app.worktrees_open=false;cx.notify();}))))
                                .children(self.refs.iter().map(|row| {let branch=row.name.clone();let path=row.worktree_path.clone();Button::new(gpui_kit::SharedString::from(format!("choose-ref-{}",row.name))).ghost().selected(row.current).label(format!("{}{}",if row.current{"● "}else{""},row.name)).tooltip(row.worktree_path.clone().unwrap_or_else(||"No worktree checkout".into())).on_click(cx.listener(move|app,_,window,cx|app.choose_checkout(branch.clone(),path.clone(),window,cx))) }))
                                .when_some(self.worktree_outcome.as_ref(),|view,result|view.child(div().w_full().min_w_0().whitespace_normal().child(format!("Checkout updated: {}",result["worktree"]["refName"].as_str().or_else(||result["refName"].as_str()).unwrap_or("removed"))).when_some(result["worktree"]["path"].as_str(),|view,path|{let path=path.to_owned();view.child(Button::new("copy-created-worktree").ghost().label("Copy worktree path").tooltip(path.clone()).on_click(cx.listener(move|_,_,_,cx|cx.write_to_clipboard(gpui_kit::ClipboardItem::new_string(path.clone())))))})))
                                .when(self.refs_cursor.is_some(),|view|view.child(Button::new("refs-more").ghost().label("More branches").on_click(cx.listener(|app,_,_,cx|app.worktree_operation("vcs.listRefs",true,cx)))))
                            ))
                            .child(Button::new("clone-repository").label("Clone repository").secondary().on_click(cx.listener(|app,_,window,cx| app.execute_command(palette::Command::CloneProject,window,cx))))
                            .when(self.clone_open,|view| view.child(div().flex().flex_col().gap_2().p_2().rounded_md().bg(group_box).border_1().border_color(border)
                                .child(div().text_xs().font_weight(FontWeight::SEMIBOLD).text_color(muted_foreground).child("CLONE"))
                                .child(Button::new("clone-close").label("Close clone panel").ghost().on_click(cx.listener(|app,_,_,cx| { app.clone_open = false; if let Some(sender) = &app.commands { let _ = sender.try_send(connection::Command::Unsubscribe("project-clones".into())); } cx.notify(); })))
                                .child(Input::new(&self.clone_remote))
                                .child(Button::new("clone-start").debug_selector(|| "clone-start".into()).label("Start clone to project directory").primary().on_click(cx.listener(|app,_,_,cx|app.clone_project(cx))))
                                .children(self.clone_progress.iter().map(|row| {
                                    let cancel_id = row.project_id.clone();let retry_id=cancel_id.clone();
                                    div().flex().flex_col().gap_1().child(format!("{} · {} · {}{}",row.destination_path,row.phase,row.stage,row.percent.map_or(String::new(),|percent| format!(" {percent}%"))))
                                        .children(row.detail.clone()).children(row.error.clone())
                                        .when(row.phase == "running",|view|view.child(Button::new(gpui_kit::SharedString::from(format!("clone-cancel-{cancel_id}"))).label("Cancel clone").danger().outline().on_click(cx.listener(move|app,_,_,cx| app.perform("projectClone.cancel",json!({"projectId":cancel_id}),None,cx)))))
                                        .when(row.phase == "failed" || row.phase == "cancelled",|view|view.child(Button::new(gpui_kit::SharedString::from(format!("clone-retry-{retry_id}"))).label("Retry clone").secondary().on_click(cx.listener(move|app,_,_,cx| app.perform("projectClone.retry",json!({"projectId":retry_id}),None,cx)))))
                                }))
                            ))
                            .child(
                                Button::new("files")
                                    .debug_selector(|| "files".into())
                                    .label("Workspace file editor")
                                    .secondary()
                                    .on_click(cx.listener(|app, _, _, cx| {
                                        app.files_open = !app.files_open;
                                        cx.notify();
                                    })),
                            )
                            .when(self.files_open, |view| {
                                view.child(
                                    div()
                                        .flex()
                                        .flex_col()
                                        .gap_2()
                                        .p_2()
                                        .rounded_md()
                                        .bg(group_box)
                                        .border_1()
                                        .border_color(border)
                                        .child(div().text_xs().font_weight(FontWeight::SEMIBOLD).text_color(muted_foreground).child("FILES"))
                                        .child(div().key_context("NativeContentSearch").child(Input::new(&self.content_query)))
                                        .child(div().flex().flex_wrap().gap_2()
                                            .child(Button::new("content-case").ghost().selected(self.content_options.case_sensitive).label(if self.content_options.case_sensitive { "Case sensitive ✓" } else { "Case sensitive" }).on_click(cx.listener(|app,_,_,cx| { app.content_options.case_sensitive = !app.content_options.case_sensitive; app.search_contents(cx); })))
                                            .child(Button::new("content-word").ghost().selected(self.content_options.whole_word).label(if self.content_options.whole_word { "Whole word ✓" } else { "Whole word" }).on_click(cx.listener(|app,_,_,cx| { app.content_options.whole_word = !app.content_options.whole_word; app.search_contents(cx); })))
                                            .child(Button::new("content-regex").ghost().selected(self.content_options.use_regex).label(if self.content_options.use_regex { "Regex ✓" } else { "Regex" }).on_click(cx.listener(|app,_,_,cx| { app.content_options.use_regex = !app.content_options.use_regex; app.search_contents(cx); })))
                                            .child(Button::new("search-contents").debug_selector(|| "search-contents".into()).label("Search contents").primary().on_click(cx.listener(|app,_,_,cx| app.search_contents(cx)))))
                                        .children(self.content_results.as_ref().map(|results| div().flex().flex_col().gap_1()
                                            .when(results.truncated, |view| view.child("Content results truncated; refine the search"))
                                            .when(results.matches.is_empty(), |view| view.child("No content matches"))
                                            .child(div().id("content-result-list").track_scroll(&self.content_scroll).max_h(px(160.)).overflow_y_scroll().flex().flex_col().children(results.matches.iter().enumerate().map(|(index,hit)| {
                                                Button::new(gpui_kit::SharedString::from(format!("content-hit-{index}"))).debug_selector(move||format!("content-hit-{index}")).ghost().selected(index == self.content_index).label(format!("{}{}:{}  {}",if index == self.content_index { "› " } else { "" },hit.path,hit.line_number,"")).child(gpui_kit::StyledText::new(hit.line_content.clone()).with_highlights(hit.byte_ranges().into_iter().map(|range| (range,gpui_kit::HighlightStyle { background_color:Some(selection),..Default::default() })))).on_click(cx.listener(move|app,_,window,cx| app.open_content_hit(index,window,cx)))
                                            })))
                                        ))
                                        .child(Input::new(&self.file_path))
                                        .child(div().flex().flex_wrap().gap_2()
                                            .child(Button::new("list-files").debug_selector(||"list-files".into()).label("Browse directory").secondary().on_click(cx.listener(|app,_,_,cx|app.list_files(false,cx))))
                                            .child(Button::new("search-files").debug_selector(||"search-files".into()).label("Search file paths").secondary().on_click(cx.listener(|app,_,_,cx|app.list_files(true,cx)))))
                                        .when(self.file_listing_truncated,|view|view.child("Results truncated; refine the path or search"))
                                        .child(div().id("file-entry-list").max_h(px(160.)).overflow_y_scroll().flex().flex_col().gap_1().children(self.file_entries.iter().enumerate().map(|(index,entry)| {
                                            let path=entry.path.clone();let directory=entry.kind=="directory";
                                            Button::new(gpui_kit::SharedString::from(format!("file-entry-{index}"))).debug_selector(move||format!("file-entry-{index}")).ghost().label(format!("{}{}",entry.path,if directory { "/" } else { "" }))
                                                .on_click(cx.listener(move|app,_,window,cx| {
                                                    app.file_path.update(cx,|input,cx|input.set_value(path.clone(),window,cx));
                                                    if directory { app.list_files(false,cx); } else { app.file_operation(false,window,cx); }
                                                }))
                                        })))
                                        .child(
                                            div()
                                                .flex()
                                                .gap_2()
                                                .child(
                                                    Button::new("read-file")
                                                        .debug_selector(|| "read-file".into())
                                                        .label("Read file")
                                                        .secondary()
                                                        .on_click(cx.listener(|app, _, window, cx| {
                                                            app.file_operation(false, window, cx)
                                                        })),
                                                )
                                                .child(
                                                    Button::new("save-file")
                                                        .debug_selector(|| "save-file".into())
                                                        .label("Save file")
                                                        .primary()
                                                        .on_click(cx.listener(|app, _, window, cx| {
                                                            app.file_operation(true, window, cx)
                                                        })),
                                                ),
                                        )
                                        .children(self.file_editor.target.as_ref().map(|target|format!("Loaded: {} / {}{}",target.cwd,target.path,if self.file_editor.writable() { "" } else { " (read-only)" })))
                                        .child(Button::new("discard-file-edits").debug_selector(||"discard-file-edits".into()).label("Discard unsaved file edits").ghost().on_click(cx.listener(|app,_,window,cx| {
                                            if app.file_busy { app.status="Wait for the pending file operation before discarding".into();cx.notify();return; }
                                            let text=app.file_editor.saved.clone();app.file_contents.update(cx,|input,cx|input.set_value(text,window,cx));cx.notify();
                                        })))
                                        .child(Textarea::new(&self.file_contents)),
                                )
                            })
                            .child(conversation)
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .gap_2()
                                    .p_2()
                                    .rounded_md()
                                    .bg(group_box)
                                    .border_1()
                                    .border_color(border)
                                    .child(div().text_xs().font_weight(FontWeight::SEMIBOLD).text_color(muted_foreground).child("MODEL"))
                                    .child(
                                        div()
                                            .flex()
                                            .gap_2()
                                            .child(Input::new(&self.provider))
                                            .child(Input::new(&self.model)),
                                    )
                                    .child(Button::new("model-catalog").debug_selector(||"model-catalog".into()).label("Choose provider and model").secondary().on_click(cx.listener(|app,_,_,cx| { app.catalog_open = !app.catalog_open;cx.notify(); })))
                                    .when(self.catalog_open,|view|view.child(div().id("provider-model-catalog").flex_shrink_0().max_h(px(200.)).overflow_y_scroll().flex().flex_col().gap_2()
                                        .child(Button::new("refresh-catalog").label("Reload server catalog").ghost().on_click(cx.listener(|app,_,_,cx|app.perform("server.getConfig",json!({}),None,cx))))
                                        .children(self.provider_catalog.iter().map(|provider|div().flex().flex_col().gap_1()
                                            .child(div().text_color(muted_foreground).child(format!("{} · {} · {}",provider.label(),provider.instance_id,provider.explanation())))
                                            .children(provider.models.iter().enumerate().map(|(index,model)| {
                                                let instance=provider.instance_id.clone();let slug=model.slug.clone();let enabled=provider.selectable();
                                                Button::new(gpui_kit::SharedString::from(format!("catalog-{}-{index}",provider.instance_id))).debug_selector({ let id=format!("catalog-{}-{index}",provider.instance_id);move||id.clone() }).ghost().selected(model.slug == self.model.read(cx).value()).disabled(!enabled).label(model.name.clone()).on_click(cx.listener(move|app,_,window,cx|app.pick_model(&instance,&slug,window,cx)))
                                            })))))
                                    )
                                    .children(self.provider_catalog.iter().find(|provider|provider.instance_id==self.provider.read(cx).value().as_str()).and_then(|provider|provider.models.iter().find(|model|model.slug==self.model.read(cx).value().as_str())).and_then(|model|model.capabilities["optionDescriptors"].as_array()).map(|options|div().flex().flex_col().gap_1().children(options.iter().enumerate().map(|(index,option)| {
                                        let id=option["id"].as_str().unwrap_or_default().to_owned();
                                        let previous=self.selected_thread.as_ref().and_then(|id|self.shell.threads.iter().chain(self.pending_threads.iter().map(|draft|&draft.thread)).find(|thread|&thread.id==id)).map(|thread|&thread.model_selection).unwrap_or(&Value::Null);
                                        let selected=self.model_selection(previous,cx).ok().and_then(|selection|selection["options"].as_array().and_then(|options|options.iter().find(|option|option["id"]==id)).map(|option|option["value"].clone())).or_else(||option.get("currentValue").cloned()).or_else(||option["options"].as_array().and_then(|choices|choices.iter().find(|choice|choice["isDefault"]==true)).map(|choice|choice["id"].clone()));
                                        let choices=if option["type"]=="boolean" {vec![json!({"id":false,"label":"Off"}),json!({"id":true,"label":"On"})]} else {option["options"].as_array().cloned().unwrap_or_default()};
                                        div().flex().flex_col().gap_1().child(div().text_color(muted_foreground).child(option["label"].as_str().unwrap_or_default().to_owned())).child(div().flex().flex_wrap().gap_1().children(choices.into_iter().enumerate().map(|(choice_index,choice)| {
                                            let id=id.clone();let value=choice["id"].clone();
                                            Button::new(gpui_kit::SharedString::from(format!("model-option-{index}-{choice_index}"))).debug_selector(move||format!("model-option-{index}-{choice_index}")).ghost().selected(selected.as_ref()==Some(&value)).label(format!("{}{}",choice["label"].as_str().unwrap_or_default(),if selected.as_ref()==Some(&value){" ✓"}else{""})).on_click(cx.listener(move|app,_,_,cx|app.choose_model_option(&id,value.clone(),cx)))
                                        })))
                                    }))))
                            )
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .gap_2()
                                    .p_2()
                                    .rounded_md()
                                    .bg(group_box)
                                    .border_1()
                                    .border_color(border)
                                    .child(div().text_xs().font_weight(FontWeight::SEMIBOLD).text_color(muted_foreground).child("PROMPT"))
                                    .child(Textarea::new(&self.composer))
                                    .child(
                                        div()
                                            .flex()
                                            .flex_wrap()
                                            .gap_2()
                                            .child(
                                                Button::new("send")
                                                    .debug_selector(|| "send".into())
                                                    .label("Send")
                                                    .primary()
                                                    .on_click(cx.listener(|app, _, window, cx| {
                                                        app.send_prompt(window, cx)
                                                    })),
                                            )
                                            .child(
                                                Button::new("stash")
                                                    .debug_selector(|| "stash".into())
                                                    .label("Stash prompt")
                                                    .ghost()
                                                    .on_click(cx.listener(|app, _, window, cx| {
                                                        app.stash_prompt(false, window, cx)
                                                    })),
                                            )
                                            .child(
                                                Button::new("restore-stash")
                                                    .debug_selector(|| "restore-stash".into())
                                                    .label("Restore stashed")
                                                    .ghost()
                                                    .on_click(cx.listener(|app, _, window, cx| {
                                                        app.stash_prompt(true, window, cx)
                                                    })),
                                            )
                                            .child(Button::new("interrupt").label("Interrupt").ghost().on_click(
                                                cx.listener(|app, _, _, cx| {
                                                    app.lifecycle("thread.turn.interrupt", cx)
                                                }),
                                            ))
                                            .child(Button::new("stop").label("Stop session").danger().outline().on_click(
                                                cx.listener(|app, _, _, cx| {
                                                    app.lifecycle("thread.session.stop", cx)
                                                }),
                                            )),
                                    ),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(muted_foreground)
                                    .child("Experimental native slice — feature parity is still being evaluated"),
                            ),
                    ),
            )
            .children(palette)
    }
}

fn bind_native_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("cmd-q", QuitNative, None),
        KeyBinding::new(
            "enter",
            ContentSubmit,
            Some("NativeApp > NativeContentSearch > Input"),
        ),
        KeyBinding::new(
            "down",
            ContentNext,
            Some("NativeApp > NativeContentSearch > Input"),
        ),
        KeyBinding::new(
            "up",
            ContentPrevious,
            Some("NativeApp > NativeContentSearch > Input"),
        ),
        KeyBinding::new("cmd-p", OpenFiles, Some("NativeApp")),
        KeyBinding::new("cmd-shift-f", OpenContentSearch, Some("NativeApp")),
        KeyBinding::new("cmd-k", TogglePalette, Some("NativeApp")),
        KeyBinding::new("cmd-s", StashPrompt, Some("NativeApp")),
        KeyBinding::new("cmd-n", NewThread, Some("NativeApp")),
        KeyBinding::new("cmd-shift-o", NewThread, Some("NativeApp")),
        KeyBinding::new("cmd-shift-s", ToggleSettle, Some("NativeApp")),
        KeyBinding::new("cmd-shift-p", TogglePin, Some("NativeApp")),
        KeyBinding::new(
            "escape",
            ClosePalette,
            Some("NativeApp > NativePalette > Input"),
        ),
        KeyBinding::new(
            "down",
            PaletteNext,
            Some("NativeApp > NativePalette > Input"),
        ),
        KeyBinding::new(
            "up",
            PalettePrevious,
            Some("NativeApp > NativePalette > Input"),
        ),
    ]);
}

fn main() {
    gpui_kit::application().run(move |cx: &mut App| {
        gpui_kit::init(cx);
        theme::init(cx);
        bind_native_keys(cx);
        cx.on_action(|_: &QuitNative, cx| cx.quit());
        cx.set_menus(vec![
            gpui_kit::Menu {
                name: "T3 Code Native".into(),
                disabled: false,
                items: vec![gpui_kit::MenuItem::action("Quit", QuitNative)],
            },
            gpui_kit::Menu {
                name: "Thread".into(),
                disabled: false,
                items: vec![
                    gpui_kit::MenuItem::action("New thread", NewThread),
                    gpui_kit::MenuItem::action("Toggle settlement", ToggleSettle),
                    gpui_kit::MenuItem::action("Toggle pin", TogglePin),
                    gpui_kit::MenuItem::action("Stash prompt", StashPrompt),
                ],
            },
            gpui_kit::Menu {
                name: "View".into(),
                disabled: false,
                items: vec![
                    gpui_kit::MenuItem::action("Commands", TogglePalette),
                    gpui_kit::MenuItem::action("Workspace files", OpenFiles),
                    gpui_kit::MenuItem::action("Search contents", OpenContentSearch),
                ],
            },
        ]);
        let bounds = Bounds::centered(None, size(px(1280.), px(900.)), cx);
        gpui_kit::open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..TitleBar::window_options()
            },
            cx,
            |window, cx| cx.new(|cx| NativeApp::new(window, cx, true)),
        )
        .expect("could not open native window");
        cx.on_window_closed(|cx, _| {
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();
        cx.activate(true);
    });
}
