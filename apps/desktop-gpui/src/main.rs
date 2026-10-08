use gpui_kit::component::{
    ActiveTheme as _, TitleBar,
    input::{InputEvent, InputState, TextareaState},
    resizable::{ResizableState, h_resizable, resizable_panel},
};
use gpui_kit::{
    App, Context, Entity, FocusHandle, Focusable as _, KeyBinding, Pixels, Point, Subscription, Task,
    Window, WindowOptions, div, prelude::*, px, size,
};
use serde_json::Value;
use t3code_desktop_gpui::{
    auth, connection, credentials, drafts, files, notifications, palette, providers, queue, shell,
    sidebar, thread, window_state, workspaces,
};
// Used only by ui_tests through `use super::*`.
#[cfg(test)]
use gpui_kit::component::input::Position;
#[cfg(test)]
use serde_json::json;
#[cfg(test)]
use t3code_desktop_gpui::commands;

gpui_kit::actions!(
    native,
    [
        UndoThreadNavigation,
        ThreadSearchSubmit,
        ThreadSearchNext,
        ThreadSearchPrevious,
        ThreadSearchCancel,
        ContentSubmit,
        ContentNext,
        ContentPrevious,
        OpenFiles,
        OpenContentSearch,
        OpenPreferredEditor,
        OpenPullRequests,
        TogglePalette,
        ClosePalette,
        PaletteNext,
        PalettePrevious,
        BranchNext,
        BranchPrevious,
        BranchPick,
        SlashNext,
        SlashPrevious,
        SlashRun,
        SlashClear,
        MentionNext,
        MentionPrevious,
        MentionPick,
        MentionClose,
        NewThread,
        StashPrompt,
        ToggleSettle,
        TogglePin,
        DismissOverlay,
        PreviousThread,
        NextThread,
        OpenSettings,
        ToggleTerminal,
        ToggleRightPanel,
        ToggleSidebar,
        SteerPrompt,
        QuitNative,
        ZoomIn,
        ZoomOut,
        ZoomReset
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

// Kit's default bundle omits application icons such as Paperclip and ShieldOff.
const NATIVE_ASSETS: gpui_kit::assets::AllAssets = gpui_kit::assets::AllAssets;

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
    PullRequests,
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

mod app;
mod platform_shell;
#[path = "app/connection_health_config.rs"]
mod server_config_projection;
#[path = "surfaces/agents/view.rs"]
mod runtime_agents;
#[path = "surfaces/usage/view.rs"]
mod runtime_usage;
#[path = "surfaces/pull_requests/mod.rs"]
mod runtime_pull_requests;
#[path = "surfaces/terminal/mod.rs"]
mod runtime_terminal;
mod conversation_presentation;
mod composer_state;
mod composer_thumbnail;
mod theme;
mod settings_appearance;
mod settings_theme_library;
mod settings_fonts;
mod settings_preferences;
mod settings_keybindings;
mod settings_scopes;
mod settings_clone;
mod settings_environments;
mod environment_pages;
mod environment_sidebar;
mod settings_provider;
mod settings_provider_interaction;
mod settings_palette;
mod settings_projects;
mod settings_project_actions;
#[cfg(test)]
#[path = "ui_tests/settings_fixc.rs"]
mod settings_fixc;
#[cfg(test)]
mod ui_tests;
mod views;

fn single_line_input(window: &mut Window, cx: &mut Context<InputState>) -> InputState {
    // GPUI's single-line shaper panics if native text insertion includes a newline.
    InputState::new(window, cx).validate(|text, _| !text.contains(['\n', '\r']))
}

struct NativeApp {
    environment_page_id: gpui_kit::EntityId,
    environment_hub: std::rc::Rc<std::cell::RefCell<environment_pages::Hub>>,
    environment_pages: Vec<Entity<NativeApp>>,
    sessions: t3code_desktop_gpui::connection_sessions::Registry,
    environment_identity_verified: bool,
    focus: FocusHandle,
    palette_open: bool,
    palette_query: Entity<InputState>,
    palette_index: usize,
    palette_cache: std::cell::RefCell<settings_palette::Cache>,
    palette_scroll: gpui_kit::UniformListScrollHandle,
    settings_shortcut_focus: FocusHandle,
    /// Highlighted row of the composer's "/" menu.
    slash_index: usize,
    /// The composer's "@" token (byte range and query) whose file menu is open.
    mention: Option<(std::ops::Range<usize>, String)>,
    mention_entries: Vec<files::Entry>,
    mention_index: usize,
    mention_loading: bool,
    mention_epoch: u64,
    mention_task: Option<Task<()>>,
    previous_focus: Option<FocusHandle>,
    environment_return_focus: std::rc::Rc<std::cell::RefCell<Option<FocusHandle>>>,
    shell: shell::Shell,
    archived_shell: shell::Shell,
    detail: thread::ThreadDetail,
    diffs: app::diffs::DiffState,
    agents_surface: runtime_agents::SurfaceState,
    pull_requests: runtime_pull_requests::State,
    terminal_drawer: runtime_terminal::Drawer,
    status: String,
    status_kind: notifications::Outcome,
    connection_phase: connection::Phase,
    environment_name: Option<String>,
    storage_error: Option<String>,
    selected_project: Option<String>,
    selected_thread: Option<String>,
    sidebar_view: sidebar::View,
    sidebar_state: sidebar::State,
    /// Actions already handled for the current keystroke; see `once_per_keystroke`.
    keystroke_actions: std::collections::HashSet<&'static str>,
    keystroke_event: Option<platform_shell::keystrokes::EventIdentity>,
    /// Off in the headless harness, which has no native menu to double-dispatch.
    keystroke_guard: bool,
    sidebar_rows: views::r13_b_sidebar_rows::Rows,
    server_config: server_config_projection::Projection,
    thread_undo: app::thread_undo::UndoState,
    sidebar_clock: Option<Task<()>>,
    sidebar_rename: Entity<InputState>,
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
    composer_state: composer_state::ComposerState,
    settings_open: bool,
    settings_query: Entity<InputState>,
    theme_editor: settings_theme_library::Editor,
    font_editor: settings_fonts::Editor,
    provider_auth_editor: settings_provider_interaction::Editor,
    settings_provider_details: Vec<Value>,
    settings_provider_task: Option<Task<()>>,
    subscription_enqueues: std::collections::HashMap<String, Task<()>>,
    settings_provider_busy: Option<u64>,
    scoped_pages: settings_scopes::ScopedPages,
    repository_chooser: settings_clone::RepositoryChooser,
    directory_browser: settings_projects::DirectoryBrowser,
    action_extras: settings_project_actions::ActionExtras,
    /// Activity disclosures the user toggled, keyed by turn.
    activity_expanded: std::collections::HashMap<String, bool>,
    inspector_open: bool,
    inspector_width: f32,
    sidebar_open: bool,
    sidebar_width: f32,
    /// Split states the app replaces whenever their container changes size.
    sidebar_split: Entity<ResizableState>,
    workspace_split: Entity<ResizableState>,
    thread_menu_open: bool,
    git_header: app::git_actions::GitHeader,
    git_live: app::git_live::LiveGit,
    header_action_preferences: app::git_header_actions::ActionPreferences,
    git_picker: app::git_picker::PickerNavigation,
    editor_choice: app::git_editors::EditorChoice,
    header_rename: app::git_identity::HeaderRename,
    /// The sidebar row whose secondary click opened the thread menu, and the
    /// pointer position it opens at. None while the header menu is open.
    thread_menu_row: Option<(String, Point<Pixels>)>,
    collapsed_projects: Vec<String>,
    collapsed_groups: Vec<String>,
    /// The conversation's virtual list. It follows the tail until the user
    /// scrolls up, and resumes once they scroll back to the newest row.
    conversation_scroll: gpui_kit::ListState,
    /// The rows `conversation_scroll` currently holds, diffed on each render.
    conversation_rows: std::rc::Rc<Vec<t3code_desktop_gpui::conversation::TimelineRow>>,
    conversation_presentation: conversation_presentation::ConversationPresentation,
    /// The open user-input request the answer drafts below belong to.
    user_input_request: Option<String>,
    /// Chosen option values and custom-answer inputs, by question id.
    user_input_selected: std::collections::HashMap<String, Vec<String>>,
    user_input_custom: std::collections::HashMap<String, Entity<InputState>>,
    attachments: Vec<Value>,
    attachment_drafts: app::attachments::AttachmentDrafts,
    auth_state: Option<Value>,
    auth_callback: Entity<InputState>,
    right_tab: RightTab,
    browser_panel: Entity<t3code_desktop_gpui::surfaces::browser::BrowserPanel>,
    available_editors: Vec<String>,
    /// The persisted window state; None when the app runs without persistence.
    saved_window: Option<window_state::WindowState>,
    context_meter_hidden: bool,
    notification_preferences: notifications::Preferences,
    inspector_section: Option<InspectorSection>,
    project_settings: Option<Value>,
    action_name: Entity<InputState>,
    action_command: Entity<InputState>,
    action_edit_id: Option<String>,
    action_setup: bool,
    action_wait: bool,
    action_busy: bool,
    action_task: Option<Task<()>>,
    action_runs: Vec<app::project_actions::ActionRun>,
    checkout_operations: Vec<app::checkout_operations::CheckoutOperation>,
    checkout_stops: app::checkout_receipts::CheckoutStops,
    checkout_recipes: Vec<app::checkout_recipes::CheckoutRecipe>,
    confirm_action_delete: Option<String>,
    search: Entity<InputState>,
    search_results: Vec<Value>,
    pending_threads: Vec<drafts::PendingThread>,
    prepare_new_worktree: bool,
    next_thread_defaults: t3code_desktop_gpui::project_actions::DraftDefaultsOverride,
    new_thread_loading: bool,
    pending_defaults_loading: Option<String>,
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
    /// Why the last branch listing came back without rows.
    refs_error: Option<String>,
    /// The trigger the branch picker hangs off while it is open.
    branch_picker: Option<&'static str>,
    branch_query: Entity<InputState>,
    branch_workflow: app::branch_workflows::BranchWorkflow,
    branch_workflow_input: Entity<InputState>,
    branch_workflow_error: Option<String>,
    branch_workflow_busy: bool,
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
    /// Prompts waiting for their thread's turn to finish, by thread.
    queues: queue::Queues,
    queue_details: std::collections::HashMap<String, app::queue_subscriptions::QueueDetail>,
    queue_subscription_epoch: u64,
    follow_up_behavior: queue::FollowUpBehavior,
    /// The toast stack, oldest first.
    toasts: Vec<notifications::Toast>,
    notification_tracker: notifications::Tracker,
    notification_badge: std::collections::HashSet<String>,
    notification_platform: t3code_desktop_gpui::notification_platform::Platform,
    toast_id: u64,
    thread_required_toast: Option<u64>,
    thread_required_status: bool,
    /// Whether the window has OS focus; a focused window still notifies for other threads.
    window_active: bool,
    _subscriptions: Vec<Subscription>,
    #[cfg(target_os = "macos")]
    _visible_frame_guard: Option<platform_shell::visible_frame::Guard>,
}

impl NativeApp {
    /// macOS delivers a menu item's key equivalent to the menu as well as to the
    /// focused view while a text input has focus, so one keystroke can dispatch the
    /// same menu-bound action twice. Toggles then cancel out and mutations repeat.
    /// Returns false only for duplicate delivery of the same native key event.
    fn once_per_keystroke(
        &mut self,
        action: &'static str,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        platform_shell::keystrokes::accept(self, action, _window, cx)
    }

    fn new(window: &mut Window, cx: &mut Context<Self>, auto_connect: bool) -> Self {
        Self::new_scoped(window, cx, auto_connect, true)
    }

    fn new_scoped(window: &mut Window, cx: &mut Context<Self>, auto_connect: bool, notification_handler: bool) -> Self {
        theme::observe(window, cx);
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
        let branch_query =
            cx.new(|cx| single_line_input(window, cx).placeholder("Search branches…"));
        let branch_workflow_input = cx.new(|cx| {
            single_line_input(window, cx).placeholder("Branch name, base branch, or PR reference")
        });
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
        subscriptions.push(cx.subscribe(&search, |app, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) { app.thread_search_changed(cx); }
        }));
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
                    app.remember_composer_draft(cx);
                    app.update_mention(cx);
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
                        app.palette_scroll.scroll_to_item(0, gpui_kit::ScrollStrategy::Nearest);
                        cx.notify();
                    }
                    InputEvent::PressEnter { .. } => app.submit_palette(window, cx),
                    _ => {}
                },
            ),
        );
        subscriptions.push(cx.subscribe_in(
            &branch_workflow_input,
            window,
            |app, _, event, window, cx| {
                use app::branch_workflows::BranchWorkflow;
                match event {
                    InputEvent::Change => {
                        app.branch_workflow_error = None;
                        cx.notify();
                    }
                    InputEvent::PressEnter { .. } => match app.branch_workflow {
                        BranchWorkflow::Create => app.run_branch_workflow(false, window, cx),
                        BranchWorkflow::PullRequest => app.run_branch_workflow(true, window, cx),
                        BranchWorkflow::Environment => {
                            app.select_branch_environment(true, window, cx)
                        }
                        BranchWorkflow::Browse => {}
                    },
                    _ => {}
                }
            },
        ));
        subscriptions.push(
            cx.subscribe_in(
                &branch_query,
                window,
                |app, _, event, window, cx| match event {
                    InputEvent::Change => cx.notify(),
                    InputEvent::PressEnter { .. } => {
                        if app.branch_workflow != app::branch_workflows::BranchWorkflow::Browse {
                            return;
                        }
                        let first = app
                            .branch_picker_rows(cx)
                            .first()
                            .map(|row| (row.name.clone(), row.worktree_path.clone()));
                        if let Some((branch, path)) = first {
                            app.pick_checkout(branch, path, window, cx);
                        }
                    }
                    _ => {}
                },
            ),
        );
        subscriptions.push(
            cx.subscribe(&file_contents, |app, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change)
                    && app
                        .file_editor
                        .dirty(app.file_contents.read(cx).value().as_str())
                {
                    app.cancel_checkout_interest();
                    cx.notify();
                }
            }),
        );
        subscriptions.push(cx.on_focus(&composer.read(cx).focus_handle(cx), window, |app, _, cx| {
            app.browser_panel.update(cx, |panel, _| panel.release_native_focus());
        }));
        subscriptions.push(cx.observe_window_activation(window, |app, window, cx| {
            app.window_active = window.is_window_active();
            if app.window_active && app.environment_is_selected() {
                app.clear_notification_badge(cx);
                app.refresh_notification_authorization(cx);
            }
        }));
        subscriptions.push(cx.observe_window_bounds(window, |app, window, cx| {
            app.reset_splits(cx);
            if let Some(state) = &mut app.saved_window {
                platform_shell::remember_bounds(state, window);
            }
        }));
        let mut app = Self {
            environment_page_id: cx.entity_id(),
            environment_hub: std::rc::Rc::new(std::cell::RefCell::new(environment_pages::Hub::new(cx.entity().downgrade()))),
            environment_pages: Vec::new(),
            sessions: Default::default(),
            environment_identity_verified: false,
            focus: cx.focus_handle(),
            palette_open: false,
            palette_query,
            palette_index: 0,
            palette_cache: Default::default(),
            palette_scroll: gpui_kit::UniformListScrollHandle::new(),
            settings_shortcut_focus: cx.focus_handle(),
            slash_index: 0,
            mention: None,
            mention_entries: vec![],
            mention_index: 0,
            mention_loading: false,
            mention_epoch: 0,
            mention_task: None,
            previous_focus: None,
            environment_return_focus: Default::default(),
            shell: shell::Shell::default(),
            archived_shell: shell::Shell::default(),
            detail: thread::ThreadDetail::default(),
            diffs: app::diffs::DiffState::default(),
            agents_surface: runtime_agents::SurfaceState::default(),
            pull_requests: runtime_pull_requests::State::default(),
            terminal_drawer: runtime_terminal::Drawer::new(window, cx),
            status: "Not connected".into(),
            status_kind: notifications::Outcome::Info,
            connection_phase: connection::Phase::Offline,
            environment_name: None,
            storage_error: None,
            selected_project: None,
            selected_thread: None,
            sidebar_view: sidebar::View::Active,
            sidebar_state: sidebar::State::default(),
            keystroke_actions: Default::default(),
            keystroke_event: None,
            keystroke_guard: !cfg!(test),
            sidebar_rows: views::r13_b_sidebar_rows::Rows::default(),
            server_config: server_config_projection::Projection::default(),
            thread_undo: app::thread_undo::UndoState::default(),
            sidebar_clock: None,
            sidebar_rename: cx.new(|cx| single_line_input(window, cx).placeholder("Thread title")),
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
            settings_query: Self::create_settings_query(window, cx),
            theme_editor: settings_theme_library::Editor::new(window, cx),
            font_editor: settings_fonts::Editor::new(window, cx),
            provider_auth_editor: settings_provider_interaction::Editor::new(window, cx),
            settings_provider_details: vec![],
            settings_provider_task: None,
            subscription_enqueues: Default::default(),
            settings_provider_busy: None,
            scoped_pages: settings_scopes::ScopedPages::default(),
            repository_chooser: settings_clone::RepositoryChooser::default(),
            directory_browser: settings_projects::DirectoryBrowser::default(),
            action_extras: settings_project_actions::ActionExtras::default(),
            catalog_open: false,
            composer_state: Default::default(),
            settings_open: false,
            activity_expanded: Default::default(),
            inspector_open: true,
            inspector_width: window_state::RIGHT_PANEL_WIDTH.1,
            sidebar_open: true,
            sidebar_width: window_state::SIDEBAR_WIDTH.1,
            sidebar_split: cx.new(|_| ResizableState::default()),
            workspace_split: cx.new(|_| ResizableState::default()),
            thread_menu_open: false,
            thread_menu_row: None,
            collapsed_projects: Vec::new(),
            collapsed_groups: Vec::new(),
            conversation_scroll: {
                let list = gpui_kit::ListState::new(0, gpui_kit::ListAlignment::Top, px(400.));
                list.set_follow_mode(gpui_kit::FollowMode::Tail);
                list
            },
            conversation_rows: Default::default(),
            conversation_presentation: Default::default(),
            user_input_request: None,
            user_input_selected: Default::default(),
            user_input_custom: Default::default(),
            attachments: Vec::new(),
            attachment_drafts: Default::default(),
            auth_state: None,
            auth_callback: cx.new(|cx| {
                single_line_input(window, cx).placeholder("Paste the authorization callback URL")
            }),
            right_tab: RightTab::Inspector,
            browser_panel: {
                let panel = cx.new(|cx| t3code_desktop_gpui::surfaces::browser::BrowserPanel::new(window, cx));
                cx.observe(&panel, |_, _, cx| cx.notify()).detach();
                panel
            },
            available_editors: Vec::new(),
            git_header: Default::default(),
            git_live: Default::default(),
            header_action_preferences: app::git_header_actions::ActionPreferences::restore(),
            git_picker: Default::default(),
            editor_choice: app::git_editors::EditorChoice::restore(),
            header_rename: Default::default(),
            saved_window: None,
            context_meter_hidden: false,
            notification_preferences: notifications::Preferences::default(),
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
            action_runs: Vec::new(),
            checkout_operations: app::checkout_operations::restore(),
            checkout_stops: Default::default(),
            checkout_recipes: app::checkout_recipes::restore(),
            confirm_action_delete: None,
            search,
            search_results: vec![],
            pending_threads: vec![],
            prepare_new_worktree: false,
            next_thread_defaults: Default::default(),
            new_thread_loading: false,
            pending_defaults_loading: None,
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
            refs_error: None,
            branch_picker: None,
            branch_query,
            branch_workflow: app::branch_workflows::BranchWorkflow::Browse,
            branch_workflow_input,
            branch_workflow_error: None,
            branch_workflow_busy: false,
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
            queues: queue::Queues::default(),
            queue_details: std::collections::HashMap::new(),
            queue_subscription_epoch: 0,
            follow_up_behavior: queue::FollowUpBehavior::default(),
            toasts: Vec::new(),
            notification_tracker: notifications::Tracker::default(),
            notification_badge: std::collections::HashSet::new(),
            notification_platform: t3code_desktop_gpui::notification_platform::Platform::new(
                !auto_connect,
            ),
            toast_id: 0,
            thread_required_toast: None,
            thread_required_status: false,
            window_active: window.is_window_active(),
            _subscriptions: subscriptions,
            #[cfg(target_os = "macos")]
            _visible_frame_guard: if notification_handler { platform_shell::visible_frame::Guard::install(window, cx) } else { None },
        };
        let entity = cx.entity().downgrade();
        let handle = window.window_handle();
        if notification_handler { cx.on_system_notification_response(move |response, cx| {
            let _ = handle.update(cx, |_, window, cx| {
                entity.update(cx, |app, cx| {
                    app.route_environment_notification(&response.tag, window, cx)
                })
            });
        });
        }
        // Keyboard shortcuts dispatch along the focus path, so an unfocused window
        // has none; start in the composer, where the user types first anyway.
        if notification_handler { app.composer.update(cx, |input, cx| input.focus(window, cx)); }
        if auto_connect {
            let state = window_state::load();
            app.context_meter_hidden = state.context_meter_hidden;
            app.notification_preferences = state.notifications;
            app.follow_up_behavior = state.follow_up_behavior;
            app.sidebar_open = state.sidebar_open;
            app.sidebar_width = state.sidebar_width;
            app.inspector_open = state.right_panel_open;
            app.inspector_width = state.right_panel_width;
            app.terminal_drawer.height = state.terminal_height;
            app.saved_window = Some(state);
            if let Err(error) = app.sidebar_state.load_local(drafts::default_path().with_file_name("sidebar.json")) { app.storage_error = Some(format!("Sidebar preferences cannot be read: {error}")); }
            match drafts::Drafts::open(drafts::default_path()) {
                Ok(store) => app.draft_store = Some(store),
                Err(_) => app.storage_error = Some(
                    "Draft storage cannot be read; existing file preserved, persistence disabled"
                        .into(),
                ),
            }
            let saved = app.environment_choices(cx);
            let initial_saved = saved.iter().find(|(origin, _, enabled)| *enabled && origin == &app.environment_scope)
                .or_else(|| if std::env::var_os("T3_NATIVE_SERVER").is_none() { saved.iter().find(|(_, _, enabled)| *enabled) } else { None });
            if let Some((origin, _, _)) = initial_saved {
                app.environment_scope = origin.clone();
                app.server.update(cx, |input, cx| input.set_value(origin.clone(), window, cx));
                if std::env::var_os("T3_NATIVE_ACCESS_TOKEN").is_some() && std::env::var("T3_NATIVE_SERVER").ok().and_then(|base| auth::origin(&base).ok()).as_ref() == Some(origin) { app.connect(cx); }
                else { app.credential_operation(credentials::Operation::Load, cx); }
            } else { app.connect(cx); }
            app.refresh_notification_authorization(cx);
        }
        app
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
        self.remember_layout();
        if let Err(error) = self.sidebar_state.save_local() { eprintln!("Could not flush sidebar preferences: {error}"); }
        if let Some(mut state) = self.saved_window {
            state.follow_up_behavior = self.follow_up_behavior;
            state.context_meter_hidden = self.context_meter_hidden;
            state.notifications = self.notification_preferences;
            state.terminal_height = self.terminal_drawer.height;
            window_state::save(state);
        }
    }
}

impl Render for NativeApp {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.ensure_environment_pages(_window, cx);
        if let Some(page) = self.presented_environment(cx) {
            return div().size_full().child(page).into_any_element();
        }
        self.clamp_terminal_height(_window);
        self.terminal_synchronize(cx);
        self.sync_git_status(cx);
        self.sync_browser_panel(cx);
        let background = cx.theme().background;
        let foreground = cx.theme().foreground;
        // Workspace: the conversation is primary; controls live in the inspector.
        let workspace = div()
            .id("native-workspace")
            .min_w_0()
            .flex()
            .flex_1()
            .h_full()
            .child(
                h_resizable("workspace-split")
                    .with_state(&self.workspace_split)
                    .on_resize({
                        let app = cx.entity().downgrade();
                        move |split, _, cx| {
                            let Some(&width) = split.read(cx).sizes().get(1) else {
                                return;
                            };
                            let _ = app.update(cx, |app, _| {
                                app.inspector_width = window_state::clamp_width(
                                    width / px(1.),
                                    window_state::RIGHT_PANEL_WIDTH,
                                );
                                app.remember_layout();
                            });
                        }
                    })
                    .child(
                        resizable_panel().size_range(px(360.)..px(f32::MAX)).child(
                            div()
                                .flex()
                                .flex_col()
                                .flex_1()
                                .min_w_0()
                                .h_full()
                                .relative()
                                .child(self.render_thread_header(cx))
                                .child(self.render_conversation_with_toasts(cx))
                                .child(self.render_composer(cx))
                                .children(self.render_terminal(cx))
                                .child(self.render_status_bar(_window, cx))
                                .into_any_element(),
                        ),
                    )
                    .child(
                        resizable_panel()
                            .visible(self.inspector_open)
                            .size(px(self.inspector_width))
                            .size_range(
                                px(window_state::RIGHT_PANEL_WIDTH.0)
                                    ..px(window_state::RIGHT_PANEL_WIDTH.2),
                            )
                            .flex_none()
                            .child(self.render_inspector(cx)),
                    ),
            )
            .into_any_element();

        div()
            .relative()
            .key_context("NativeApp")
            .capture_key_down(cx.listener(|app, _, _, cx| platform_shell::keystrokes::begin_key(app, cx)))
            .capture_any_mouse_down(cx.listener(|app, event, window, cx| {
                platform_shell::input_trace::root_mouse(app, event, window, cx);
                app.browser_panel.update(cx, |panel, _| panel.release_native_focus());
            }))
            .capture_any_mouse_up(cx.listener(|_, _, _, cx| platform_shell::input_trace::root_mouse_up(cx)))
            .map(|root| platform_shell::window_actions(root, cx))
            .track_focus(&self.focus)
            .on_action(cx.listener(|app, _: &UndoThreadNavigation, _, cx| app.undo_thread_navigation(cx)))
            .on_action(cx.listener(|app, _: &ContentSubmit, window, cx| {
                if app
                    .content_results
                    .as_ref()
                    .is_some_and(|results| !results.matches.is_empty())
                {
                    app.open_content_hit(app.content_index, window, cx);
                } else {
                    app.search_contents(cx);
                }
            }))
            .on_action(cx.listener(|app, _: &ContentNext, _, cx| app.move_content_hit(true, cx)))
            .on_action(
                cx.listener(|app, _: &ContentPrevious, _, cx| app.move_content_hit(false, cx)),
            )
            .on_action(cx.listener(|app, _: &OpenPreferredEditor, _, cx| app.open_in_editor(cx)))
            .on_action(cx.listener(|app, _: &OpenPullRequests, window, cx| { if !app.once_per_keystroke("OpenPullRequests", window, cx) { return; } app.open_pull_requests(window, cx); }))
            .on_action(cx.listener(|app, _: &OpenFiles, window, cx| { if !app.once_per_keystroke("OpenFiles", window, cx) { return; }
                app.execute_command(palette::Command::Files, window, cx)
            }))
            .on_action(cx.listener(|app, _: &OpenContentSearch, window, cx| { if !app.once_per_keystroke("OpenContentSearch", window, cx) { return; }
                app.execute_command(palette::Command::ContentSearch, window, cx)
            }))
            .on_action(
                cx.listener(|app, _: &TogglePalette, window, cx| { if !app.once_per_keystroke("TogglePalette", window, cx) { return; } app.toggle_palette(window, cx); }),
            )
            .on_action(
                cx.listener(|app, _: &ClosePalette, window, cx| app.close_palette(window, cx)),
            )
            .on_action(cx.listener(|app, _: &PaletteNext, _, cx| app.move_palette(true, cx)))
            .on_action(cx.listener(|app, _: &PalettePrevious, _, cx| app.move_palette(false, cx)))
            .on_action(cx.listener(|app, _: &NewThread, window, cx| { if !app.once_per_keystroke("NewThread", window, cx) { return; }
                app.execute_command(palette::Command::NewThread, window, cx)
            }))
            .on_action(cx.listener(|app, _: &StashPrompt, window, cx| { if !app.once_per_keystroke("StashPrompt", window, cx) { return; }
                let restore = app.composer.read(cx).value().trim().is_empty()
                    && app.attachments.is_empty();
                app.stash_prompt(restore, window, cx)
            }))
            .on_action(
                cx.listener(|app, _: &ToggleSettle, window, cx| { if !app.once_per_keystroke("ToggleSettle", window, cx) { return; } app.toggle_thread_property(false, cx); }),
            )
            .on_action(
                cx.listener(|app, _: &TogglePin, window, cx| { if !app.once_per_keystroke("TogglePin", window, cx) { return; } app.toggle_thread_property(true, cx); }),
            )
            // Inputs re-propagate Enter after emitting PressEnter; consume it here so an
            // unhandled keystroke can never fall through to text insertion after a send.
            .on_action(cx.listener(|_, _: &gpui_kit::component::input::Enter, _, _| {}))
            .on_action(cx.listener(|app, _: &DismissOverlay, window, cx| {
                if app.git_header.open {
                    app.dismiss_git_header(cx);
                    app.focus.focus(window, cx);
                    return;
                }
                if !app.dismiss_overlay(window, cx) {
                    cx.propagate();
                }
            }))
            .on_action(
                cx.listener(|app, _: &PreviousThread, window, cx| { if !app.once_per_keystroke("PreviousThread", window, cx) { return; }
                    app.step_thread(false, window, cx)
                }),
            )
            .on_action(
                cx.listener(|app, _: &NextThread, window, cx| { if !app.once_per_keystroke("NextThread", window, cx) { return; } app.step_thread(true, window, cx); }),
            )
            .on_action(cx.listener(|app, _: &ZoomIn, window, cx| { if !app.once_per_keystroke("ZoomIn", window, cx) { return; } app.change_scale(Some(1), window, cx); }))
            .on_action(cx.listener(|app, _: &ZoomOut, window, cx| { if !app.once_per_keystroke("ZoomOut", window, cx) { return; } app.change_scale(Some(-1), window, cx); }))
            .on_action(cx.listener(|app, _: &ZoomReset, window, cx| { if !app.once_per_keystroke("ZoomReset", window, cx) { return; } app.change_scale(None, window, cx); }))
            .on_action(cx.listener(|app, _: &OpenSettings, window, cx| { if !app.once_per_keystroke("OpenSettings", window, cx) { return; } app.toggle_settings(window, cx); }))
            .on_action(cx.listener(|app, _: &SteerPrompt, window, cx| app.steer_prompt(window, cx)))
            .on_action(cx.listener(|app, _: &ToggleTerminal, window, cx| { if !app.once_per_keystroke("ToggleTerminal", window, cx) { return; }
                if app.shell_terminal_available() { app.toggle_terminal(window, cx); }
            }))
            .on_action(cx.listener(|app, _: &ToggleRightPanel, window, cx| { if !app.once_per_keystroke("ToggleRightPanel", window, cx) { return; }
                app.set_inspector_open(!app.inspector_open, cx)
            }))
            .on_action(cx.listener(|app, _: &ToggleSidebar, window, cx| { if !app.once_per_keystroke("ToggleSidebar", window, cx) { return; }
                app.execute_command(palette::Command::ToggleSidebar, window, cx)
            }))
            .flex()
            .flex_col()
            .size_full()
            .bg(background)
            .text_color(foreground)
            .child(self.render_title_bar(cx))
            .child(div().flex().flex_1().min_h_0().map(|row| {
                if self.sidebar_open {
                    row.child(
                        h_resizable("sidebar-split")
                            .with_handle_appearance(platform_shell::sidebar_handle(cx))
                            .with_state(&self.sidebar_split)
                            .on_resize({
                                let app = cx.entity().downgrade();
                                move |split, _, cx| {
                                    let Some(&width) = split.read(cx).sizes().first() else {
                                        return;
                                    };
                                    let _ = app.update(cx, |app, _| {
                                        app.sidebar_width = window_state::clamp_width(
                                            width / px(1.),
                                            window_state::SIDEBAR_WIDTH,
                                        );
                                        app.remember_layout();
                                    });
                                }
                            })
                            .child(
                                resizable_panel()
                                    .size(px(self.sidebar_width))
                                    .size_range(
                                        px(window_state::SIDEBAR_WIDTH.0)
                                            ..px(window_state::SIDEBAR_WIDTH.2),
                                    )
                                    .flex_none()
                                    .child(self.render_environment_sidebar(cx)),
                            )
                            .child(resizable_panel().size_range(px(640.)..px(f32::MAX)).child(workspace)),
                    )
                } else {
                    row.child(workspace)
                }
            }))
            .children(self.render_overlay_layer(cx))
            .children(platform_shell::input_trace::listener(cx))
            .into_any_element()
    }
}

fn bind_native_keys(cx: &mut App) {
    runtime_terminal::keys::bind(cx);
    platform_shell::bind(cx);
    cx.bind_keys([
        KeyBinding::new("cmd-q", QuitNative, None),
        KeyBinding::new("cmd-z", UndoThreadNavigation, Some("NativeApp && !Input")),
        KeyBinding::new("enter", ThreadSearchSubmit, Some("NativeApp > NativeThreadSearch > Input")),
        KeyBinding::new("down", ThreadSearchNext, Some("NativeApp > NativeThreadSearch > Input")),
        KeyBinding::new("up", ThreadSearchPrevious, Some("NativeApp > NativeThreadSearch > Input")),
        KeyBinding::new("escape", ThreadSearchCancel, Some("NativeApp > NativeThreadSearch > Input")),
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
        KeyBinding::new("cmd-o", OpenPreferredEditor, Some("NativeApp")),
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
        KeyBinding::new("cmd-=", ZoomIn, Some("NativeApp")),
        KeyBinding::new("cmd--", ZoomOut, Some("NativeApp")),
        KeyBinding::new("cmd-0", ZoomReset, Some("NativeApp")),
        KeyBinding::new("cmd-j", ToggleTerminal, Some("NativeApp")),
        KeyBinding::new("cmd-alt-b", ToggleRightPanel, Some("NativeApp")),
        KeyBinding::new("cmd-b", ToggleSidebar, Some("NativeApp")),
        KeyBinding::new("cmd-shift-enter", SteerPrompt, Some("NativeApp")),
        KeyBinding::new(
            "escape",
            ClosePalette,
            Some("NativeApp > NativePalette > Input"),
        ),
        KeyBinding::new(
            "escape",
            DismissOverlay,
            Some("NativeApp > NativeBranchPicker > Input"),
        ),
        KeyBinding::new("escape", DismissOverlay, Some("NativeApp > NativeBranchBrowse > Input")),
        KeyBinding::new("down", BranchNext, Some("NativeApp > NativeBranchBrowse > Input")),
        KeyBinding::new("up", BranchPrevious, Some("NativeApp > NativeBranchBrowse > Input")),
        KeyBinding::new("enter", BranchPick, Some("NativeApp > NativeBranchBrowse > Input")),
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
        KeyBinding::new("down", MentionNext, Some("NativeMentionMenu > Input")),
        KeyBinding::new("up", MentionPrevious, Some("NativeMentionMenu > Input")),
        KeyBinding::new("enter", MentionPick, Some("NativeMentionMenu > Input")),
        KeyBinding::new("tab", MentionPick, Some("NativeMentionMenu > Input")),
        KeyBinding::new("escape", MentionClose, Some("NativeMentionMenu > Input")),
    ]);
    settings_keybindings::init(cx);
}

fn main() {
    gpui_kit::application()
        .with_assets(NATIVE_ASSETS)
        .run(move |cx: &mut App| {
            gpui_kit::init(cx);
            theme::init(cx);
            bind_native_keys(cx);
            cx.set_app_identity("codes.t3.native", "T3 Code Native");
            cx.on_action(|_: &QuitNative, cx| cx.quit());
            cx.set_menus(vec![
                gpui_kit::Menu {
                    name: "T3 Code Native".into(),
                    disabled: false,
                    items: platform_shell::application_items().into_iter()
                        .chain([gpui_kit::MenuItem::action("Quit", QuitNative)]).collect(),
                },
                platform_shell::edit_menu(),
                platform_shell::window_menu(),
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
                        gpui_kit::MenuItem::action("Toggle sidebar", ToggleSidebar),
                        gpui_kit::MenuItem::action("Toggle terminal", ToggleTerminal),
                        gpui_kit::MenuItem::action("Toggle right panel", ToggleRightPanel),
                        gpui_kit::MenuItem::action("Pull requests", OpenPullRequests),
                        gpui_kit::MenuItem::action("Workspace files", OpenFiles),
                        gpui_kit::MenuItem::action("Search contents", OpenContentSearch),
                        gpui_kit::MenuItem::separator(),
                        gpui_kit::MenuItem::action("Actual Size", ZoomReset),
                        gpui_kit::MenuItem::action("Zoom In", ZoomIn),
                        gpui_kit::MenuItem::action("Zoom Out", ZoomOut),
                        gpui_kit::MenuItem::action("Toggle Full Screen", platform_shell::Fullscreen),
                    ],
                },
            ]);
            let (bounds, display_id) = platform_shell::initial_bounds(window_state::load(), cx);
            gpui_kit::open_window(
                WindowOptions {
                    window_bounds: Some(bounds),
                    display_id,
                    window_min_size: Some(size(px(848.), px(620.))),
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
