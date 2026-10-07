use gpui_kit::component::{
    ActiveTheme as _,
    input::{InputEvent, InputState, TextareaState},
    resizable::{ResizableState, h_resizable, resizable_panel},
    TitleBar,
};
use gpui_kit::{
    App, Bounds, Context, Entity, FocusHandle, KeyBinding, Pixels, Point,
    Subscription, Task, Window, WindowBounds, WindowOptions, div, prelude::*, px, size,
};
use serde_json::Value;
use t3code_desktop_gpui::{
    auth, connection, credentials, drafts, files, notifications, palette, providers, queue,
    shell, sidebar, thread, window_state, workspaces,
};
// Used only by ui_tests through `use super::*`.
#[cfg(test)]
use gpui_kit::{Focusable as _, component::input::Position};
#[cfg(test)]
use serde_json::json;
#[cfg(test)]
use t3code_desktop_gpui::commands;

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
        ToggleRightPanel,
        ToggleSidebar,
        SteerPrompt,
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

mod app;
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
    /// The composer's "@" token (byte range and query) whose file menu is open.
    mention: Option<(std::ops::Range<usize>, String)>,
    mention_entries: Vec<files::Entry>,
    mention_index: usize,
    mention_loading: bool,
    mention_epoch: u64,
    mention_task: Option<Task<()>>,
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
    inspector_width: f32,
    sidebar_open: bool,
    sidebar_width: f32,
    /// Split states the app replaces whenever their container changes size.
    sidebar_split: Entity<ResizableState>,
    workspace_split: Entity<ResizableState>,
    thread_menu_open: bool,
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
    /// The open user-input request the answer drafts below belong to.
    user_input_request: Option<String>,
    /// Chosen option values and custom-answer inputs, by question id.
    user_input_selected: std::collections::HashMap<String, Vec<String>>,
    user_input_custom: std::collections::HashMap<String, Entity<InputState>>,
    attachments: Vec<Value>,
    auth_state: Option<Value>,
    auth_callback: Entity<InputState>,
    right_tab: RightTab,
    available_editors: Vec<String>,
    /// The persisted window state; None when the app runs without persistence.
    saved_window: Option<window_state::WindowState>,
    context_meter_hidden: bool,
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
    /// Why the last branch listing came back without rows.
    refs_error: Option<String>,
    /// The trigger the branch picker hangs off while it is open.
    branch_picker: Option<&'static str>,
    branch_query: Entity<InputState>,
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
    /// The toast stack, oldest first.
    toasts: Vec<notifications::Toast>,
    toast_id: u64,
    /// Whether the window has OS focus; a focused window still notifies for other threads.
    window_active: bool,
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
        let branch_query =
            cx.new(|cx| single_line_input(window, cx).placeholder("Search branches…"));
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
        subscriptions.push(cx.subscribe_in(
            &branch_query,
            window,
            |app, _, event, window, cx| match event {
                InputEvent::Change => cx.notify(),
                InputEvent::PressEnter { .. } => {
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
        ));
        subscriptions.push(cx.observe_window_activation(window, |app, window, _| {
            app.window_active = window.is_window_active();
        }));
        subscriptions.push(cx.observe_window_bounds(window, |app, window, cx| {
            app.reset_splits(cx);
            if let gpui_kit::WindowBounds::Windowed(bounds) = window.window_bounds()
                && let Some(state) = &mut app.saved_window
            {
                state.width = bounds.size.width / px(1.);
                state.height = bounds.size.height / px(1.);
            }
        }));
        let mut app = Self {
            focus: cx.focus_handle(),
            palette_open: false,
            palette_query,
            palette_index: 0,
            slash_index: 0,
            mention: None,
            mention_entries: vec![],
            mention_index: 0,
            mention_loading: false,
            mention_epoch: 0,
            mention_task: None,
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
            user_input_request: None,
            user_input_selected: Default::default(),
            user_input_custom: Default::default(),
            attachments: Vec::new(),
            auth_state: None,
            auth_callback: cx.new(|cx| {
                single_line_input(window, cx).placeholder("Paste the authorization callback URL")
            }),
            right_tab: RightTab::Inspector,
            available_editors: Vec::new(),
            saved_window: None,
            context_meter_hidden: false,
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
            refs_error: None,
            branch_picker: None,
            branch_query,
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
            toasts: Vec::new(),
            toast_id: 0,
            window_active: window.is_window_active(),
            _subscriptions: subscriptions,
        };
        let entity = cx.entity().downgrade();
        let handle = window.window_handle();
        cx.on_system_notification_response(move |response, cx| {
            let _ = handle.update(cx, |_, window, cx| {
                entity.update(cx, |app, cx| {
                    app.open_notified_thread(&response.tag, window, cx)
                })
            });
        });
        if auto_connect {
            let state = window_state::load();
            app.context_meter_hidden = state.context_meter_hidden;
            app.sidebar_open = state.sidebar_open;
            app.sidebar_width = state.sidebar_width;
            app.inspector_open = state.right_panel_open;
            app.inspector_width = state.right_panel_width;
            app.saved_window = Some(state);
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
        if let Some(mut state) = self.saved_window {
            state.context_meter_hidden = self.context_meter_hidden;
            window_state::save(state);
        }
    }
}

impl Render for NativeApp {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
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
                                .children(self.render_toasts(cx))
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
                            .child(self.render_inspector(cx).into_any_element()),
                    ),
            )
            .into_any_element();

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
            .on_action(cx.listener(|app, _: &SteerPrompt, window, cx| app.steer_prompt(window, cx)))
            .on_action(cx.listener(|app, _: &ToggleRightPanel, _, cx| {
                app.set_inspector_open(!app.inspector_open, cx)
            }))
            .on_action(cx.listener(|app, _: &ToggleSidebar, window, cx| {
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
                                    .child(self.render_sidebar(cx).into_any_element()),
                            )
                            .child(resizable_panel().child(workspace)),
                    )
                } else {
                    row.child(workspace)
                }
            }))
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
}

fn main() {
    gpui_kit::application()
        .with_assets(gpui_kit::assets::Assets)
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
                    gpui_kit::MenuItem::action("Toggle sidebar", ToggleSidebar),
                    gpui_kit::MenuItem::action("Toggle right panel", ToggleRightPanel),
                    gpui_kit::MenuItem::action("Workspace files", OpenFiles),
                    gpui_kit::MenuItem::action("Search contents", OpenContentSearch),
                ],
            },
        ]);
        let window_size = window_state::load()
            .size()
            .map(|(width, height)| size(px(width), px(height)))
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
