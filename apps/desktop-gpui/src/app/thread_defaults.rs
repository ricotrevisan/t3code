//! Every creation entry shares this capture/load/resolve boundary. Async reads
//! cannot create a draft after navigation to a different project or checkout.
use crate::NativeApp;
use gpui_kit::{Context, Window};
use serde_json::{Value, json};
use t3code_desktop_gpui::{connection, drafts, project_actions, workspaces};

pub(super) struct ResolvedCreation {
    pub defaults: project_actions::ThreadDefaults,
    pub choice: project_actions::DraftDefaultsOverride,
    pub project_file: Option<Value>,
    pub project_file_loaded: bool,
    pub base_branch: Option<String>,
}

impl NativeApp {
    pub(crate) fn choose_next_workspace(&mut self, worktree: bool) {
        let project = self.selected_project.clone().unwrap_or_default();
        if self.next_thread_defaults.environment != self.environment_scope
            || self.next_thread_defaults.project != project
        {
            self.next_thread_defaults = project_actions::DraftDefaultsOverride {
                environment: self.environment_scope.clone(),
                project,
                ..Default::default()
            };
        }
        self.next_thread_defaults.workspace_mode =
            Some(if worktree { "worktree" } else { "local" }.into());
        self.prepare_new_worktree = worktree;
    }

    pub(crate) fn choose_next_setup(&mut self, origin: Option<bool>, setup: Option<bool>) {
        let project = self.selected_project.clone().unwrap_or_default();
        if self.next_thread_defaults.environment != self.environment_scope
            || self.next_thread_defaults.project != project
        {
            self.next_thread_defaults = project_actions::DraftDefaultsOverride {
                environment: self.environment_scope.clone(),
                project,
                ..Default::default()
            };
        }
        if let Some(value) = origin {
            self.start_from_origin = value;
            self.next_thread_defaults.start_from_origin = Some(value);
        }
        if let Some(value) = setup {
            self.run_setup_script = value;
            self.next_thread_defaults.run_setup_script = Some(value);
        }
    }

    pub(crate) fn new_thread(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.new_thread_loading || !self.can_change_checkout(cx) {
            return;
        }
        let Some(project) = self
            .shell
            .projects
            .iter()
            .find(|p| Some(&p.id) == self.selected_project.as_ref())
            .cloned()
        else {
            self.report_error("Select a project first", cx);
            return;
        };
        let mut choice = self.next_thread_defaults.clone();
        if choice.environment != self.environment_scope || choice.project != project.id {
            choice = project_actions::DraftDefaultsOverride {
                environment: self.environment_scope.clone(),
                project: project.id.clone(),
                ..Default::default()
            };
        }
        if let Some((id, _, _)) = &self.new_thread_checkout
            && id == &project.id
        {
            // An existing checkout is an explicit destination, not a request
            // to prepare another worktree. Permissions still inherit.
            choice = project_actions::DraftDefaultsOverride {
                environment: self.environment_scope.clone(),
                project: project.id.clone(),
                workspace_mode: Some("local".into()),
                ..choice
            };
        }
        let settings = self.project_settings.clone();
        let needs_file = settings.as_ref().is_none_or(|s| {
            workspace_needs_file(
                s,
                &project.id,
                project.default_thread_env_mode.as_deref(),
                &choice,
                &self.environment_scope,
            )
        });
        let resolved = self.effective_thread_defaults(
            &project.id,
            project.default_thread_env_mode.as_deref(),
            None,
            Some(&choice),
        );
        let base = if choice.environment == self.environment_scope
            && choice.project == project.id
            && choice.workspace_mode.as_deref() == Some("worktree")
        {
            self.worktree_ref.read(cx).value().trim().to_owned()
        } else {
            String::new()
        };
        if !needs_file
            && let Ok(defaults) = &resolved
            && (defaults.workspace_mode != "worktree" || !base.is_empty())
        {
            self.new_thread_resolved(
                ResolvedCreation {
                    defaults: defaults.clone(),
                    choice,
                    project_file: None,
                    project_file_loaded: false,
                    base_branch: (!base.is_empty()).then_some(base),
                },
                window,
                cx,
            );
            return;
        }
        let Some(sender) = self.commands.clone() else {
            self.report_error(
                "Connect and load server defaults before creating a thread",
                cx,
            );
            return;
        };
        let environment = self.environment_scope.clone();
        let generation = self.generation;
        let selected = self.selected_thread.clone();
        let handle = window.window_handle();
        self.new_thread_loading = true;
        cx.notify();
        cx.spawn(async move |entity, cx| {
            let result = async {
                let settings = match settings {
                    Some(s) => s,
                    None => connection::call(&sender, "server.getSettings", json!({})).await?,
                };
                let file_loaded = workspace_needs_file(
                    &settings,
                    &project.id,
                    project.default_thread_env_mode.as_deref(),
                    &choice,
                    &environment,
                );
                let file = if file_loaded {
                    read_project_file(&sender, &project.workspace_root).await?
                } else {
                    None
                };
                let defaults = project_actions::thread_defaults(
                    Some(&settings),
                    &environment,
                    &project.id,
                    project.default_thread_env_mode.as_deref(),
                    file.as_ref(),
                    Some(&choice),
                )?;
                let base = if defaults.workspace_mode == "worktree" && base.is_empty() {
                    Some(read_base_branch(&sender, &project.workspace_root).await?)
                } else {
                    (!base.is_empty()).then_some(base)
                };
                Ok::<_, String>((settings, defaults, file, file_loaded, base))
            }
            .await;
            let _ = handle.update(cx, |_, window, cx| {
                entity.update(cx, |app, cx| {
                    app.new_thread_loading = false;
                    if app.generation != generation
                        || app.environment_scope != environment
                        || app.selected_project.as_deref() != Some(&project.id)
                        || app.selected_thread != selected
                    {
                        cx.notify();
                        return;
                    }
                    match result {
                        Ok((settings, _, file, file_loaded, base)) => {
                            // A settings write while reads were in flight wins.
                            if app.project_settings.is_none() {
                                app.project_settings = Some(settings);
                            }
                            match app.effective_thread_defaults(
                                &project.id,
                                project.default_thread_env_mode.as_deref(),
                                file.as_ref(),
                                Some(&choice),
                            ) {
                                Ok(defaults) => app.new_thread_resolved(
                                    ResolvedCreation {
                                        defaults,
                                        choice,
                                        project_file: file,
                                        project_file_loaded: file_loaded,
                                        base_branch: base,
                                    },
                                    window,
                                    cx,
                                ),
                                Err(e) => app.report_error(e, cx),
                            }
                        }
                        Err(e) => app.report_error(e, cx),
                    }
                    cx.notify();
                })
            });
        })
        .detach();
    }

    /// Restored/implicit drafts may need a file/base after settings change.
    /// Capture before loading; newer text, choices, navigation and receipts win.
    pub(crate) fn ensure_pending_send_defaults(
        &mut self,
        queued: Option<bool>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(draft) = self
            .pending_threads
            .iter()
            .find(|d| {
                Some(&d.thread.id) == self.selected_thread.as_ref()
                    && !self.shell.threads.iter().any(|t| t.id == d.thread.id)
            })
            .cloned()
        else {
            return false;
        };
        let Some(project) = self
            .shell
            .projects
            .iter()
            .find(|p| p.id == draft.thread.project_id)
            .cloned()
        else {
            return false;
        };
        let settings = self.project_settings.clone();
        let needs_file = !draft.project_file_loaded
            && settings.as_ref().is_none_or(|s| {
                workspace_needs_file(
                    s,
                    &project.id,
                    project.default_thread_env_mode.as_deref(),
                    &draft.defaults_override,
                    &self.environment_scope,
                )
            });
        let needs_base = self
            .pending_defaults(&draft)
            .is_ok_and(|d| d.workspace_mode == "worktree")
            && draft.base_branch.is_none()
            && draft.prepare_worktree.is_none();
        if settings.is_some() && !needs_file && !needs_base {
            return false;
        }
        if self.pending_defaults_loading.is_some() {
            return true;
        }
        let Some(sender) = self.commands.clone() else {
            self.report_error("Connect before loading draft defaults", cx);
            return true;
        };
        let environment = self.environment_scope.clone();
        let generation = self.generation;
        let text = self.composer.read(cx).value().to_string();
        let handle = window.window_handle();
        self.pending_defaults_loading = Some(draft.thread.id.clone());
        cx.spawn(async move |entity, cx| {
            let result = async {
                let settings = match settings.clone() {
                    Some(v) => v,
                    None => connection::call(&sender, "server.getSettings", json!({})).await?,
                };
                let file_needed = !draft.project_file_loaded
                    && workspace_needs_file(
                        &settings,
                        &project.id,
                        project.default_thread_env_mode.as_deref(),
                        &draft.defaults_override,
                        &environment,
                    );
                let file = if file_needed {
                    read_project_file(&sender, &project.workspace_root).await?
                } else {
                    draft.project_file.clone()
                };
                let defaults = project_actions::thread_defaults(
                    Some(&settings),
                    &environment,
                    &project.id,
                    project.default_thread_env_mode.as_deref(),
                    file.as_ref(),
                    Some(&draft.defaults_override),
                )?;
                let base = if defaults.workspace_mode == "worktree"
                    && draft.base_branch.is_none()
                    && draft.prepare_worktree.is_none()
                {
                    Some(read_base_branch(&sender, &project.workspace_root).await?)
                } else {
                    draft.base_branch.clone()
                };
                Ok::<_, String>((settings, file, file_needed, base))
            }
            .await;
            let _ = handle.update(cx, |_, window, cx| {
                entity.update(cx, |app, cx| {
                    if app.pending_defaults_loading.as_deref() == Some(&draft.thread.id) {
                        app.pending_defaults_loading = None;
                    }
                    if app.generation != generation
                        || app.environment_scope != environment
                        || app.project_settings != settings
                    {
                        return;
                    }
                    let Some(index) = app.pending_threads.iter().position(|d| {
                        d.thread.id == draft.thread.id
                            && d.defaults_override == draft.defaults_override
                    }) else {
                        return;
                    };
                    match result {
                        Ok((settings, file, file_needed, base)) => {
                            if app.project_settings.is_none() {
                                app.project_settings = Some(settings);
                            }
                            let pending = &mut app.pending_threads[index];
                            pending.project_file = file;
                            pending.project_file_loaded |= file_needed;
                            pending.base_branch = base;
                            if let Some(store) = &mut app.draft_store {
                                store.put_pending(&environment, pending.clone());
                            }
                            app.schedule_draft_save(cx);
                            if app.selected_thread.as_deref() == Some(&draft.thread.id)
                                && app.composer.read(cx).value().as_str() == text
                            {
                                if let Some(steer) = queued {
                                    app.queue_prompt(steer, window, cx);
                                } else {
                                    app.send_prompt(window, cx);
                                }
                            }
                        }
                        Err(error) => app.report_error(error, cx),
                    }
                    cx.notify();
                })
            });
        })
        .detach();
        cx.notify();
        true
    }

    pub(crate) fn accept_pending_thread(&mut self, id: &str, cx: &mut Context<Self>) {
        if !self.pending_threads.iter().any(|d| d.thread.id == id) {
            return;
        }
        self.pending_threads.retain(|d| d.thread.id != id);
        if let Some(store) = &mut self.draft_store {
            store.remove_pending(&self.environment_scope, id);
        }
        self.schedule_draft_save(cx);
        if self.selected_thread.as_deref() == Some(id) {
            let _ = self.send_thread_selection(Some(id.into()));
        }
    }

    pub(crate) fn pending_defaults(
        &self,
        draft: &drafts::PendingThread,
    ) -> Result<project_actions::ThreadDefaults, String> {
        let legacy = self
            .shell
            .projects
            .iter()
            .find(|p| p.id == draft.thread.project_id)
            .and_then(|p| p.default_thread_env_mode.as_deref());
        self.effective_thread_defaults(
            &draft.thread.project_id,
            legacy,
            draft.project_file.as_ref(),
            Some(&draft.defaults_override),
        )
    }

    /// Direct and queued first sends use the same captured owner/recipe. Modes
    /// come from the message (including explicit composer and durable plan choices).
    pub(crate) fn pending_bootstrap(
        &self,
        id: &str,
        payload: &Value,
    ) -> Result<Option<Value>, String> {
        let Some(draft) = self.pending_threads.iter().find(|d| d.thread.id == id) else {
            return Ok(None);
        };
        if self.shell.threads.iter().any(|t| t.id == id) {
            return Ok(None);
        }
        let defaults = self.pending_defaults(draft)?;
        let mut bootstrap = json!({"createThread":{"projectId":draft.thread.project_id,"title":draft.thread.title,"modelSelection":payload["modelSelection"],"runtimeMode":payload["runtimeMode"],"interactionMode":payload["interactionMode"],"branch":draft.thread.branch,"worktreePath":draft.thread.worktree_path,"createdAt":draft.created_at},"runSetupScript":defaults.run_setup_script});
        if defaults.workspace_mode == "worktree" {
            let root = self
                .shell
                .projects
                .iter()
                .find(|p| p.id == draft.thread.project_id)
                .ok_or("Draft project is unavailable")?;
            let base = draft
                .prepare_worktree
                .as_ref()
                .and_then(|v| v["baseBranch"].as_str())
                .or(draft.base_branch.as_deref())
                .ok_or("Workspace default changed: choose a base branch before sending")?;
            let mut recipe = draft
                .prepare_worktree
                .clone()
                .unwrap_or_else(|| json!({"projectCwd":root.workspace_root,"baseBranch":base}));
            recipe["startFromOrigin"] = json!(defaults.start_from_origin);
            bootstrap["prepareWorktree"] = recipe;
            bootstrap["createThread"]["branch"] = Value::Null;
            bootstrap["createThread"]["worktreePath"] = Value::Null;
        }
        Ok(Some(bootstrap))
    }
}
async fn read_project_file(
    sender: &async_channel::Sender<connection::Command>,
    root: &str,
) -> Result<Option<Value>, String> {
    match connection::call(
        sender,
        "projects.readFile",
        json!({"cwd":root,"relativePath":"t3.json"}),
    )
    .await
    {
        Ok(v) if v["truncated"] != true => Ok(v["contents"]
            .as_str()
            .and_then(|s| serde_json::from_str::<Value>(s).ok())),
        Ok(_) => Ok(None),
        Err(e) if e.contains("ENOENT") || e.to_lowercase().contains("not found") => Ok(None),
        Err(e) => Err(e),
    }
}
async fn read_base_branch(
    sender: &async_channel::Sender<connection::Command>,
    root: &str,
) -> Result<String, String> {
    let value = connection::call(
        sender,
        "vcs.listRefs",
        json!({"cwd":root,"refresh":true,"limit":100}),
    )
    .await?;
    let refs: workspaces::Refs =
        serde_json::from_value(value).map_err(|_| "Invalid base branch response")?;
    if !refs.is_repo {
        return Err("A new worktree requires a Git repository".into());
    }
    refs.refs
        .into_iter()
        .find(|r| r.current && !r.name.trim().is_empty())
        .map(|r| r.name)
        .ok_or_else(|| "Select a base branch for the new worktree".into())
}
fn workspace_needs_file(
    settings: &Value,
    project: &str,
    legacy: Option<&str>,
    choice: &project_actions::DraftDefaultsOverride,
    environment: &str,
) -> bool {
    if choice.environment == environment
        && choice.project == project
        && choice.workspace_mode.is_some()
    {
        return false;
    }
    let entry = &settings["projectSettingsOverrides"][project];
    if let Some(value) = entry.get("defaultThreadEnvMode") {
        return value.as_str().is_none();
    }
    if settings["projectSettingsFolded"] != true && legacy.is_some() {
        return false;
    }
    settings["defaultThreadEnvMode"].as_str().is_none()
}
