use gpui_kit::component::{
    ActiveTheme as _,
    input::{InputEvent, InputState, Position, TextareaState},
    resizable::{h_resizable, resizable_panel},
    TitleBar,
};
use gpui_kit::{
    App, Bounds, Context, Entity, FocusHandle, KeyBinding, Pixels, Point, Subscription, Task,
    Window, WindowBounds, WindowOptions, div, point, prelude::*, px, size,
};
use serde_json::{Value, json};
use t3code_desktop_gpui::{
    auth, commands, connection, credentials, drafts, files, palette, project_actions,
    providers, shell, sidebar, thread, window_state, workspaces,
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
        SlashNext,
        SlashPrevious,
        SlashRun,
        SlashClear,
        NewThread,
        StashPrompt,
        ToggleSettle,
        TogglePin,
        DismissOverlay,
        PreviousThread,
        NextThread,
        OpenSettings,
        ToggleRightPanel,
        QuitNative
    ]
);

/// Content type inferred from a filename, defaulting to a generic binary.
fn attachment_mime(name: &str) -> &'static str {
    match std::path::Path::new(name)
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        Some("pdf") => "application/pdf",
        Some("json") => "application/json",
        Some("md") => "text/markdown",
        Some("txt") => "text/plain",
        Some("csv") => "text/csv",
        _ => "application/octet-stream",
    }
}

const THREAD_ACTIONS: &[(&str, &str, &str)] = &[
    ("rename", "Rename", "thread.meta.update"),
    ("archive", "Archive", "thread.archive"),
    ("unarchive", "Unarchive", "thread.unarchive"),
    ("settle", "Settle", "thread.settle"),
    ("unsettle", "Unsettle", "thread.unsettle"),
    ("snooze", "Snooze 1h", "thread.snooze"),
    ("unsnooze", "Unsnooze", "thread.unsnooze"),
    ("pin", "Pin", "thread.pin"),
    ("unpin", "Unpin", "thread.unpin"),
    ("delete", "Delete thread", "thread.delete"),
];

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum RightTab {
    Inspector,
    Changes,
    Agents,
}

/// The one open accordion section of the Inspector tab.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum InspectorSection {
    Project,
    Worktrees,
    ProjectActions,
    Clone,
    Files,
    Thread,
}

#[cfg(test)]
mod ui_tests;
mod theme;
mod views;

fn single_line_input(window: &mut Window, cx: &mut Context<InputState>) -> InputState {
    // GPUI's single-line shaper panics if native text insertion includes a newline.
    InputState::new(window, cx).validate(|text, _| !text.contains(['\n', '\r']))
}

struct NativeApp {
    focus: FocusHandle,
    palette_open: bool,
    palette_query: Entity<InputState>,
    palette_index: usize,
    /// Highlighted row of the composer's "/" menu.
    slash_index: usize,
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
    confirm_revert: Option<u64>,
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
    connection_expanded: bool,
    settings_open: bool,
    /// Activity disclosures the user toggled, keyed by turn.
    activity_expanded: std::collections::HashMap<String, bool>,
    inspector_open: bool,
    thread_menu_open: bool,
    /// The sidebar row whose secondary click opened the thread menu, and the
    /// pointer position it opens at. None while the header menu is open.
    thread_menu_row: Option<(String, Point<Pixels>)>,
    collapsed_projects: Vec<String>,
    collapsed_groups: Vec<String>,
    conversation_scroll: gpui_kit::ScrollHandle,
    attachments: Vec<Value>,
    auth_state: Option<Value>,
    auth_callback: Entity<InputState>,
    right_tab: RightTab,
    available_editors: Vec<String>,
    saved_window: Option<window_state::WindowSize>,
    inspector_section: Option<InspectorSection>,
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
    worktree_ref: Entity<InputState>,
    worktree_branch: Entity<InputState>,
    worktree_path: Entity<InputState>,
    refs: Vec<workspaces::Ref>,
    refs_cursor: Option<u64>,
    worktree_task: Option<Task<()>>,
    worktree_epoch: u64,
    worktree_busy: bool,
    confirm_worktree_removal: Option<String>,
    clone_remote: Entity<InputState>,
    clone_progress: Vec<workspaces::CloneProgress>,
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
                .submit_on_enter(true)
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
        let search = cx.new(|cx| {
            single_line_input(window, cx)
                .submit_on_enter(true)
                .placeholder("Search thread history")
        });
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
                if matches!(event, InputEvent::Change) {
                    app.slash_index = 0;
                    if let Some(id) = app.selected_thread.clone() {
                        let text = app.composer.read(cx).value().to_string();
                        app.drafts.insert(id.clone(), text.clone());
                        if let Some(store) = &mut app.draft_store {
                            store.set(&app.environment_scope, &id, text);
                            app.schedule_draft_save(cx);
                        }
                    }
                }
                // Enter sends and Shift+Enter inserts a newline, as in the reference composer.
                if matches!(event, InputEvent::PressEnter { shift: false, .. }) {
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
        subscriptions.push(cx.observe_window_bounds(window, |app, window, _cx| {
            if let gpui_kit::WindowBounds::Windowed(bounds) = window.window_bounds() {
                app.saved_window = Some(window_state::WindowSize {
                    width: bounds.size.width / px(1.),
                    height: bounds.size.height / px(1.),
                });
            }
        }));
        let mut app = Self {
            focus: cx.focus_handle(),
            palette_open: false,
            palette_query,
            palette_index: 0,
            slash_index: 0,
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
            confirm_revert: None,
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
            connection_expanded: false,
            settings_open: false,
            activity_expanded: Default::default(),
            inspector_open: true,
            thread_menu_open: false,
            thread_menu_row: None,
            collapsed_projects: Vec::new(),
            collapsed_groups: Vec::new(),
            conversation_scroll: gpui_kit::ScrollHandle::new(),
            attachments: Vec::new(),
            auth_state: None,
            auth_callback: cx.new(|cx| {
                single_line_input(window, cx).placeholder("Paste the authorization callback URL")
            }),
            right_tab: RightTab::Inspector,
            available_editors: Vec::new(),
            saved_window: None,
            inspector_section: Some(InspectorSection::Project),
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
            clone_remote,
            clone_progress: Vec::new(),
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

    /// Applies a thread subscription item, keeping the conversation on its newest
    /// content unless the user has scrolled up to read history.
    fn apply_thread_update(&mut self, thread_id: &str, item: Value) -> Result<(), String> {
        let scroll = &self.conversation_scroll;
        let at_tail = scroll.max_offset().y + scroll.offset().y <= px(48.);
        let result = self.detail.apply(thread_id, item);
        if at_tail {
            self.conversation_scroll.scroll_to_bottom();
        }
        result
    }

    fn turn_running(&self) -> bool {
        self.detail.thread.as_ref().is_some_and(|thread| {
            matches!(
                thread["session"]["status"].as_str(),
                Some("starting" | "running")
            )
        })
    }

    /// Closes the topmost open overlay; false when there was nothing to close.
    fn dismiss_overlay(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if self.palette_open {
            self.close_palette(window, cx);
            return true;
        }
        if self.thread_menu_open {
            self.close_thread_menu();
        } else if self.catalog_open {
            self.catalog_open = false;
        } else if self.settings_open {
            self.settings_open = false;
        } else {
            return false;
        }
        cx.notify();
        true
    }

    /// Selects the previous or next thread in the order the sidebar shows them.
    fn step_thread(&mut self, forward: bool, window: &mut Window, cx: &mut Context<Self>) {
        let now = chrono::Utc::now();
        let threads = if self.sidebar_view == sidebar::View::Archived {
            &self.archived_shell.threads
        } else {
            &self.shell.threads
        };
        let order: Vec<String> = self
            .shell
            .projects
            .iter()
            .filter(|project| !self.collapsed_projects.contains(&project.id))
            .flat_map(|project| {
                sidebar::grouped(threads, &project.id, self.sidebar_view, now)
                    .into_iter()
                    .filter(|(group, _, _)| {
                        !self
                            .collapsed_groups
                            .contains(&sidebar::group_key(&project.id, group))
                    })
                    .flat_map(|(_, _, rows)| rows.into_iter().map(|thread| thread.id.clone()))
                    .collect::<Vec<_>>()
            })
            .collect();
        if let Some(id) = sidebar::neighbor(&order, self.selected_thread.as_deref(), forward) {
            self.select_thread(&id, window, cx);
        }
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
        self.palette_index = palette::step(self.palette_index, count, forward);
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
                self.reveal_inspector(InspectorSection::ProjectActions, window, cx)
            }
            Command::Worktrees => self.reveal_inspector(InspectorSection::Worktrees, window, cx),
            Command::CloneProject => self.reveal_inspector(InspectorSection::Clone, window, cx),
            Command::Files => {
                self.reveal_inspector(InspectorSection::Files, window, cx);
                self.file_path
                    .update(cx, |input, cx| input.focus(window, cx));
            }
            Command::ContentSearch => self.reveal_inspector(InspectorSection::Files, window, cx),
            Command::AddProject => {
                self.reveal_inspector(InspectorSection::Project, window, cx);
                match commands::create_project(
                    &self.title.read(cx).value(),
                    &self.workspace.read(cx).value(),
                ) {
                    Ok(command) => self.dispatch(command, cx),
                    Err(error) => {
                        self.status = error.into();
                        cx.notify();
                    }
                }
            }
            Command::RenameProject => self.update_project(false, cx),
            Command::RemoveProject => self.update_project(true, cx),
            Command::NewThread => self.new_thread(window, cx),
            Command::Search => self.search.update(cx, |input, cx| input.focus(window, cx)),
            Command::CustomSnooze => self.reveal_inspector(InspectorSection::Thread, window, cx),
            Command::Stash => self.stash_prompt(false, window, cx),
            Command::RestoreStash => self.stash_prompt(true, window, cx),
            Command::Connect => self.connect(cx),
            Command::Disconnect => self.disconnect(cx),
            Command::Settings => {
                self.settings_open = true;
                cx.notify();
            }
            Command::Rename => {
                self.reveal_inspector(InspectorSection::Project, window, cx);
                self.lifecycle("thread.meta.update", cx);
            }
            _ => self.lifecycle(
                match command {
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
                                            if app.inspector_section
                                                == Some(InspectorSection::Clone)
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
                                        } else if key == "project-clones"
                                            && app.inspector_section
                                                == Some(InspectorSection::Clone)
                                        {
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
                                            app.apply_thread_update(&thread_id, item)
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
        self.close_thread_menu();
        self.detail = thread::ThreadDetail::default();
        self.conversation_scroll.set_offset(point(px(0.), px(0.)));
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
                            app.attachments.clear();
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
                            app.available_editors = value["availableEditors"]
                                .as_array()
                                .map(|editors| {
                                    editors
                                        .iter()
                                        .filter_map(|editor| editor.as_str().map(str::to_owned))
                                        .collect()
                                })
                                .unwrap_or_default();
                        }
                        if tag == "provider.auth.start" {
                            app.auth_state = Some(value.clone());
                            if let Some(url) = value["authorizationUrl"].as_str() {
                                cx.open_url(url);
                            }
                        }
                        if tag == "provider.auth.start" {
                            app.auth_state = Some(value.clone());
                            if let Some(url) = value["authorizationUrl"].as_str() {
                                cx.open_url(url);
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

    fn close_thread_menu(&mut self) {
        self.thread_menu_open = false;
        self.thread_menu_row = None;
        self.confirm_delete = None;
    }

    /// The thread the open thread menu acts on: its sidebar row, else the selection.
    fn thread_menu_target(&self) -> Option<String> {
        self.thread_menu_row
            .as_ref()
            .map(|(id, _)| id.clone())
            .or_else(|| self.selected_thread.clone())
    }

    /// Runs a thread menu entry on the menu's target thread. Rename from a row
    /// selects that thread and focuses the inspector title, since the title field
    /// carries the new name.
    fn run_thread_menu_action(&mut self, kind: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(target) = self.thread_menu_target() else {
            self.close_thread_menu();
            cx.notify();
            return;
        };
        let from_row = self.thread_menu_row.is_some();
        if from_row && kind == "thread.meta.update" {
            self.close_thread_menu();
            if self.selected_thread.as_ref() != Some(&target) {
                self.select_thread(&target, window, cx);
            }
            self.reveal_inspector(InspectorSection::Project, window, cx);
            self.status = "Edit the title, then choose Rename from More".into();
            cx.notify();
            return;
        }
        let confirming = self.confirm_delete.as_ref() == Some(&target);
        if !from_row
            && kind == "thread.delete"
            && self
                .pending_threads
                .iter()
                .any(|draft| draft.thread.id == target)
        {
            self.discard_pending(window, cx);
        } else {
            self.thread_lifecycle(&target, kind, cx);
        }
        // Delete keeps the menu open so the second click confirms.
        if kind != "thread.delete" || confirming {
            self.close_thread_menu();
        }
        cx.notify();
    }

    /// Runs a thread command on the selected thread.
    fn lifecycle(&mut self, kind: &str, cx: &mut Context<Self>) {
        if let Some(id) = self.selected_thread.clone() {
            self.thread_lifecycle(&id, kind, cx);
        }
    }

    fn thread_lifecycle(&mut self, id: &str, kind: &str, cx: &mut Context<Self>) {
        if let Some(draft) = self
            .pending_threads
            .iter_mut()
            .find(|draft| draft.thread.id == id)
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
            "thread.delete" if self.confirm_delete.as_deref() != Some(id) => {
                self.confirm_delete = Some(id.to_owned());
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
        if self.turn_running() {
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
            &self.attachments,
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

    /// The effective runtime and interaction modes for the selected thread.
    fn selected_modes(&self) -> (String, String) {
        if let Some(detail) = &self.detail.thread
            && let (Some(runtime), Some(interaction)) =
                (detail["runtimeMode"].as_str(), detail["interactionMode"].as_str())
        {
            return (runtime.to_owned(), interaction.to_owned());
        }
        if let Some(id) = &self.selected_thread {
            if let Some(draft) = self.pending_threads.iter().find(|draft| &draft.thread.id == id) {
                return (
                    draft
                        .thread
                        .runtime_mode
                        .clone()
                        .unwrap_or_else(|| "full-access".into()),
                    draft
                        .thread
                        .interaction_mode
                        .clone()
                        .unwrap_or_else(|| "default".into()),
                );
            }
            if let Some(thread) = self.shell.threads.iter().find(|thread| &thread.id == id) {
                return (
                    thread
                        .runtime_mode
                        .clone()
                        .unwrap_or_else(|| "full-access".into()),
                    thread
                        .interaction_mode
                        .clone()
                        .unwrap_or_else(|| "default".into()),
                );
            }
        }
        ("full-access".into(), "default".into())
    }

    /// Change a thread's runtime or interaction mode. Pending drafts update
    /// locally because the server has no thread to mutate yet.
    fn change_thread_mode(
        &mut self,
        kind: &str,
        field: &str,
        value: &str,
        cx: &mut Context<Self>,
    ) {
        let Some(id) = self.selected_thread.clone() else {
            return;
        };
        if let Some(draft) = self
            .pending_threads
            .iter_mut()
            .find(|draft| draft.thread.id == id)
        {
            match field {
                "runtimeMode" => draft.thread.runtime_mode = Some(value.to_owned()),
                "interactionMode" => draft.thread.interaction_mode = Some(value.to_owned()),
                _ => {}
            }
            if let Some(store) = &mut self.draft_store {
                store.put_pending(&self.environment_scope, draft.clone());
            }
            self.schedule_draft_save(cx);
            cx.notify();
            return;
        }
        let payload = commands::set_thread_mode(kind, &id, field, value);
        self.dispatch(payload, cx);
    }

    /// Revert the working tree to a completed turn's checkpoint. The first
    /// click arms the action; the second dispatches it.
    fn revert_checkpoint(&mut self, turn_count: u64, cx: &mut Context<Self>) {
        let Some(id) = self.selected_thread.clone() else {
            return;
        };
        if self.confirm_revert != Some(turn_count) {
            self.confirm_revert = Some(turn_count);
            self.status =
                "Click again to restore the working tree to this checkpoint".into();
            cx.notify();
            return;
        }
        self.confirm_revert = None;
        let payload = commands::revert_command(&id, turn_count);
        self.dispatch(payload, cx);
    }

    /// Answer the provider's pending approval request.
    fn respond_approval(&mut self, request_id: &str, decision: &str, cx: &mut Context<Self>) {
        let Some(id) = self.selected_thread.clone() else {
            return;
        };
        let payload = commands::approval_response(&id, request_id, decision);
        self.dispatch(payload, cx);
    }

    /// Begin a provider sign-in flow and surface its authorization URL.
    fn start_provider_auth(&mut self, instance_id: &str, cx: &mut Context<Self>) {
        self.auth_state = None;
        self.perform(
            "provider.auth.start",
            json!({"instanceId": instance_id, "callbackMode": "client"}),
            None,
            cx,
        );
    }

    /// Finish a provider sign-in flow with a pasted callback URL.
    fn complete_provider_auth(&mut self, cx: &mut Context<Self>) {
        let Some(state) = self.auth_state.clone() else {
            return;
        };
        let instance = state["instanceId"].as_str().unwrap_or_default().to_owned();
        let flow = state["flowId"].as_str().unwrap_or_default().to_owned();
        let callback = self.auth_callback.read(cx).value().trim().to_owned();
        if callback.is_empty() {
            self.status = "Paste the callback URL first".into();
            cx.notify();
            return;
        }
        self.perform(
            "provider.auth.complete",
            json!({"instanceId": instance, "flowId": flow, "callbackUrl": callback}),
            None,
            cx,
        );
    }

    /// Open the platform file picker, upload each selected file, and stage it as
    /// an attachment for the next turn.
    fn attach_files(&mut self, cx: &mut Context<Self>) {
        let Some(sender) = self.commands.clone() else {
            self.status = "Not connected".into();
            cx.notify();
            return;
        };
        let origin = self.environment_scope.clone();
        let receiver = cx.prompt_for_paths(gpui_kit::PathPromptOptions {
            files: true,
            directories: false,
            multiple: true,
            prompt: Some("Attach files".into()),
        });
        self.pending_actions += 1;
        self.status = "Uploading attachments…".into();
        cx.notify();
        cx.spawn(async move |entity, cx| {
            let paths = match receiver.await {
                Ok(Ok(Some(paths))) => paths,
                _ => Vec::new(),
            };
            for path in paths {
                let name = path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("attachment")
                    .to_owned();
                let bytes = match std::fs::read(&path) {
                    Ok(bytes) => bytes,
                    Err(error) => {
                        let _ = entity.update(cx, |app, cx| {
                            app.status = format!("Could not read {name}: {error}");
                            cx.notify();
                        });
                        continue;
                    }
                };
                let mime = attachment_mime(&name).to_owned();
                let size = bytes.len() as u64;
                let input = json!({
                    "type": if mime.starts_with("image/") { "image" } else { "file" },
                    "name": name,
                    "mimeType": mime,
                    "sizeBytes": size,
                });
                let attachment = match connection::call(&sender, "attachments.createUploadUrl", input).await {
                    Ok(value) => {
                        let id = value["attachmentId"].as_str().unwrap_or_default().to_owned();
                        let relative = value["relativeUrl"].as_str().unwrap_or_default().to_owned();
                        match connection::upload(&sender, format!("{origin}{relative}"), mime.clone(), bytes).await {
                            Ok(()) => Some(json!({"id": id, "name": name, "mimeType": mime, "sizeBytes": size})),
                            Err(error) => {
                                let _ = entity.update(cx, |app, cx| {
                                    app.status = error;
                                    cx.notify();
                                });
                                None
                            }
                        }
                    }
                    Err(error) => {
                        let _ = entity.update(cx, |app, cx| {
                            app.status = error;
                            cx.notify();
                        });
                        None
                    }
                };
                if let Some(attachment) = attachment {
                    let label = attachment["name"].as_str().unwrap_or("attachment").to_owned();
                    let _ = entity.update(cx, |app, cx| {
                        app.attachments.push(attachment);
                        app.status = format!("Attached {label}");
                        cx.notify();
                    });
                }
            }
            let _ = entity.update(cx, |app, cx| {
                app.pending_actions = app.pending_actions.saturating_sub(1);
                cx.notify();
            });
        })
        .detach();
    }

    /// Commit, push, and open a pull request for the selected thread. Streams the
    /// server's progress events and reports the final phase.
    fn push_and_create_pr(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.selected_thread.clone() else {
            self.status = "Select a thread first".into();
            cx.notify();
            return;
        };
        let cwd = self
            .shell
            .threads
            .iter()
            .chain(self.pending_threads.iter().map(|draft| &draft.thread))
            .find(|thread| thread.id == id)
            .and_then(|thread| thread.worktree_path.clone())
            .filter(|path| !path.is_empty())
            .unwrap_or_else(|| self.workspace.read(cx).value().to_string());
        if cwd.trim().is_empty() {
            self.status = "Select a project or open a worktree first".into();
            cx.notify();
            return;
        }
        let Some(sender) = self.commands.clone() else {
            self.status = "Not connected".into();
            cx.notify();
            return;
        };
        self.pending_actions += 1;
        self.status = "Running git action\u{2026}".into();
        cx.notify();
        cx.spawn(async move |entity, cx| {
            let result = connection::call_stream(
                &sender,
                "git.runStackedAction",
                json!({
                    "actionId": uuid::Uuid::new_v4().to_string(),
                    "cwd": cwd,
                    "action": "commit_push_pr",
                    "featureBranch": true,
                    "threadId": id,
                }),
            )
            .await;
            let _ = entity.update(cx, |app, cx| {
                app.pending_actions = app.pending_actions.saturating_sub(1);
                match result {
                    Ok(events) => {
                        app.status = events
                            .last()
                            .and_then(|event| event["phase"].as_str())
                            .map(|phase| format!("Git action finished ({phase})"))
                            .unwrap_or_else(|| "Git action finished".into());
                    }
                    Err(error) => app.status = error,
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Open the selected thread's worktree (or the project root) in the first
    /// available editor, preferring Zed to match the app's design language.
    fn open_in_editor(&mut self, cx: &mut Context<Self>) {
        let cwd = self
            .selected_thread
            .as_ref()
            .and_then(|id| {
                self.shell
                    .threads
                    .iter()
                    .chain(self.pending_threads.iter().map(|draft| &draft.thread))
                    .find(|thread| &thread.id == id)
            })
            .and_then(|thread| thread.worktree_path.clone())
            .filter(|path| !path.is_empty())
            .unwrap_or_else(|| self.workspace.read(cx).value().to_string());
        if cwd.trim().is_empty() {
            self.status = "Select a project or open a worktree first".into();
            cx.notify();
            return;
        }
        let editor = self
            .available_editors
            .iter()
            .find(|editor| editor.as_str() == "zed")
            .or_else(|| {
                self.available_editors
                    .iter()
                    .find(|editor| editor.as_str() != "file-manager")
            })
            .cloned()
            .unwrap_or_else(|| "zed".into());
        self.perform(
            "shell.openInEditor",
            json!({"cwd": cwd, "editor": editor}),
            None,
            cx,
        );
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
        if let Some(size) = self.saved_window {
            window_state::save(size);
        }
    }
}

impl Render for NativeApp {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let background = cx.theme().background;
        let foreground = cx.theme().foreground;

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
            // Inputs re-propagate Enter after emitting PressEnter; consume it here so an
            // unhandled keystroke can never fall through to text insertion after a send.
            .on_action(cx.listener(|_, _: &gpui_kit::component::input::Enter, _, _| {}))
            .on_action(cx.listener(|app, _: &DismissOverlay, window, cx| {
                if !app.dismiss_overlay(window, cx) {
                    cx.propagate();
                }
            }))
            .on_action(cx.listener(|app, _: &PreviousThread, window, cx| {
                app.step_thread(false, window, cx)
            }))
            .on_action(cx.listener(|app, _: &NextThread, window, cx| {
                app.step_thread(true, window, cx)
            }))
            .on_action(cx.listener(|app, _: &OpenSettings, _, cx| {
                app.settings_open = !app.settings_open;
                cx.notify();
            }))
            .on_action(cx.listener(|app, _: &ToggleRightPanel, _, cx| {
                app.inspector_open = !app.inspector_open;
                cx.notify();
            }))
            .flex()
            .flex_col()
            .size_full()
            .bg(background)
            .text_color(foreground)
            .child(self.render_title_bar(cx))
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_h_0()
                    .child(self.render_sidebar(cx))
                    // Workspace: the conversation is primary; controls live in the inspector.
                    .child(
                        div()
                            .id("native-workspace")
                            .min_w_0()
                            .flex()
                            .flex_1()
                            .h_full()
                            .child(
                                h_resizable("workspace-split")
                                    .child(
                                        resizable_panel().child(
                                            div()
                                                .flex()
                                                .flex_col()
                                                .flex_1()
                                                .min_w_0()
                                                .h_full()
                                                .relative()
                                                .child(self.render_thread_header(cx))
                                                .child(self.render_conversation(cx))
                                                .child(self.render_composer(cx))
                                                .child(self.render_status_bar(cx))
                                                .into_any_element(),
                                        ),
                                    )
                                    .child(
                                        resizable_panel()
                                            .child(self.render_inspector(cx).into_any_element()),
                                    ),
                            ),
                    ),
            )
            .children(self.render_overlay_layer(cx))
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
        KeyBinding::new("escape", DismissOverlay, Some("NativeApp")),
        KeyBinding::new("cmd-shift-[", PreviousThread, Some("NativeApp")),
        KeyBinding::new("cmd-shift-]", NextThread, Some("NativeApp")),
        KeyBinding::new("cmd-,", OpenSettings, Some("NativeApp")),
        KeyBinding::new("cmd-alt-b", ToggleRightPanel, Some("NativeApp")),
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
        KeyBinding::new("down", SlashNext, Some("NativeSlashMenu > Input")),
        KeyBinding::new("up", SlashPrevious, Some("NativeSlashMenu > Input")),
        KeyBinding::new("enter", SlashRun, Some("NativeSlashMenu > Input")),
        KeyBinding::new("escape", SlashClear, Some("NativeSlashMenu > Input")),
    ]);
}

fn main() {
    gpui_kit::application()
        .with_assets(gpui_kit::assets::Assets)
        .run(move |cx: &mut App| {
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
                    gpui_kit::MenuItem::action("Previous thread", PreviousThread),
                    gpui_kit::MenuItem::action("Next thread", NextThread),
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
                    gpui_kit::MenuItem::action("Settings", OpenSettings),
                    gpui_kit::MenuItem::action("Toggle right panel", ToggleRightPanel),
                    gpui_kit::MenuItem::action("Workspace files", OpenFiles),
                    gpui_kit::MenuItem::action("Search contents", OpenContentSearch),
                ],
            },
        ]);
        let window_size = window_state::load()
            .map(|saved| size(px(saved.width), px(saved.height)))
            .unwrap_or_else(|| size(px(1280.), px(900.)));
        let bounds = Bounds::centered(None, window_size, cx);
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
