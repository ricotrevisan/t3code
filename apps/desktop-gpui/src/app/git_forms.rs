use crate::NativeApp;
use gpui_kit::component::{
    Disableable, Selectable, Sizable,
    button::{Button, ButtonVariants},
    checkbox::Checkbox,
    input::{Input, InputState},
};
use gpui_kit::{AppContext as _, Context, Entity, Window, div, prelude::*};
use serde_json::{Value, json};
use t3code_desktop_gpui::connection;

pub(crate) struct GitForm {
    pub action: String,
    pub input: Entity<InputState>,
    pub paths: Vec<String>,
    pub excluded: Vec<String>,
    pub providers: Vec<String>,
    pub provider: Option<String>,
    pub public: bool,
    pub discovering: bool,
    pub remote: Option<Entity<InputState>>,
    pub https: bool,
}
impl NativeApp {
    pub(crate) fn dismiss_git_header(&mut self, cx: &mut Context<Self>) {
        self.git_header.open = false;
        self.git_header.form = None;
        self.git_header.confirm = None;
        cx.notify();
    }
    pub(crate) fn record_git_progress(&mut self, event: &Value, cx: &mut Context<Self>) {
        let text = match event["kind"].as_str().unwrap_or("") {
            "action_started" => "Git action started".into(),
            "phase_started" => event["label"]
                .as_str()
                .unwrap_or("Git phase started")
                .into(),
            "hook_started" => format!(
                "Running hook: {}",
                event["hookName"].as_str().unwrap_or("hook")
            ),
            "hook_output" => format!(
                "{}: {}",
                event["stream"].as_str().unwrap_or("output"),
                event["text"].as_str().unwrap_or("")
            ),
            "hook_finished" => format!(
                "Hook {} finished · exit {}",
                event["hookName"].as_str().unwrap_or("hook"),
                event["exitCode"]
            ),
            "action_finished" => "Git action finished".into(),
            "action_failed" => format!(
                "Git action failed: {}",
                event["message"].as_str().unwrap_or("unknown error")
            ),
            _ => return,
        };
        // Retain bounded output; hook logs must not grow the menu indefinitely.
        self.git_header
            .progress
            .push(text.chars().take(2048).collect());
        if self.git_header.progress.len() > 24 {
            self.git_header.progress.remove(0);
        }
        cx.notify();
    }
    pub(crate) fn begin_git_form(
        &mut self,
        action: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let input = cx.new(|cx| {
            crate::single_line_input(window, cx).placeholder(if action == "publish" {
                "owner/repository"
            } else {
                "Commit message (optional; blank generates one)"
            })
        });
        let paths = self
            .git_header
            .status
            .as_ref()
            .and_then(|s| s["workingTree"]["files"].as_array())
            .into_iter()
            .flatten()
            .filter_map(|f| f["path"].as_str().map(str::to_owned))
            .collect();
        self.git_header.open = true;
        self.git_header.form = Some(GitForm {
            action: action.into(),
            input: input.clone(),
            paths,
            excluded: Vec::new(),
            providers: Vec::new(),
            provider: None,
            public: false,
            discovering: action == "publish",
            remote: (action == "publish")
                .then(|| cx.new(|cx| crate::single_line_input(window, cx).default_value("origin"))),
            https: false,
        });
        input.update(cx, |input, cx| input.focus(window, cx));
        if action == "publish"
            && let Some(sender) = self.commands.clone()
        {
            let generation = self.generation;
            let context = self.checkout_context();
            let epoch = self.git_header.epoch;
            cx.spawn(async move |entity, cx| {
                let result =
                    connection::call(&sender, "server.discoverSourceControl", json!({})).await;
                let _ = entity.update(cx, |app, cx| {
                    if app.generation != generation
                        || app.checkout_context() != context
                        || app.git_header.epoch != epoch
                    {
                        return;
                    }
                    if let Some(form) = app
                        .git_header
                        .form
                        .as_mut()
                        .filter(|f| f.action == "publish")
                    {
                        form.discovering = false;
                        match result {
                            Ok(value) => {
                                form.providers = value["sourceControlProviders"]
                                    .as_array()
                                    .into_iter()
                                    .flatten()
                                    .filter(|p| {
                                        p["status"] == "available"
                                            && p["auth"]["status"] != "unauthenticated"
                                    })
                                    .filter_map(|p| {
                                        p["kind"]
                                            .as_str()
                                            .filter(|p| {
                                                matches!(*p, "github" | "gitlab" | "forgejo")
                                            })
                                            .map(str::to_owned)
                                    })
                                    .collect();
                                form.provider = form.providers.first().cloned();
                            }
                            Err(error) => app.git_header.error = Some(error),
                        }
                    }
                    cx.notify();
                });
            })
            .detach();
        }
        cx.notify();
    }
    pub(crate) fn submit_git_form(&mut self, cx: &mut Context<Self>) {
        if self.pending_actions > 0
            || !self.connection_ready()
            || self.git_header.context.as_ref() != Some(&self.checkout_context())
        {
            return;
        }
        let Some(form) = self.git_header.form.as_ref() else {
            return;
        };
        let message = form.input.read(cx).value().trim().to_owned();
        if form.action != "publish" {
            let paths: Vec<_> = form
                .paths
                .iter()
                .filter(|p| !form.excluded.contains(p))
                .cloned()
                .collect();
            if paths.is_empty() {
                return;
            }
            let action = form.action.clone();
            self.git_header.commit_message = (!message.is_empty()).then_some(message);
            self.git_header.file_paths = Some(paths);
            self.git_header.form = None;
            self.run_git_action(&action, false, cx);
            return;
        }
        let Some(provider) = form.provider.clone() else {
            return;
        };
        if !valid_repository(&message) {
            self.git_header.error = Some("Enter owner/repository".into());
            cx.notify();
            return;
        }
        let (Some(sender), Some(cwd)) = (self.commands.clone(), self.selected_workspace()) else {
            return;
        };
        let visibility = if form.public { "public" } else { "private" };
        let protocol = if form.https { "https" } else { "ssh" };
        let remote = form
            .remote
            .as_ref()
            .map(|input| input.read(cx).value().trim().to_owned())
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| "origin".into());
        let generation = self.generation;
        let context = self.checkout_context();
        self.pending_actions += 1;
        self.git_header.error = None;
        self.git_header.progress = vec![format!("Publishing {message} · {visibility}")];
        cx.spawn(async move |entity, cx| {
            let result = connection::call(&sender, "sourceControl.publishRepository", json!({"cwd":cwd,"provider":provider,"repository":message,"visibility":visibility,"remoteName":remote,"protocol":protocol})).await;
            let _ = entity.update(cx, |app, cx| {
                app.pending_actions = app.pending_actions.saturating_sub(1);
                if app.generation != generation || app.checkout_context() != context { cx.notify(); return; }
                match result {
                    Ok(value) => {
                        let text = if value["status"] == "pushed" { "Repository published" } else { "Remote added; commit and push to publish code" };
                        app.git_header.form = None;
                        app.git_header.status = None;
                        app.git_header.progress = vec![text.into()];
                        app.git_header.result_url = value["repository"]["url"].as_str().filter(|url| url.starts_with("https://")).map(str::to_owned);
                        app.report_success(text, cx);
                    }
                    Err(error) => { app.git_header.error = Some(error.clone()); app.report_error(error, cx); }
                }
                cx.notify();
            });
        }).detach();
        cx.notify();
    }
    pub(crate) fn render_git_form(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let form = self.git_header.form.as_ref().unwrap();
        let publish = form.action == "publish";
        let busy = self.pending_actions > 0;
        let valid = if publish {
            form.provider.is_some() && valid_repository(form.input.read(cx).value().trim())
        } else {
            form.paths.len() > form.excluded.len()
        };
        div().flex().flex_col().gap_2()
            .child(if publish { "Publish repository" } else { "Commit changes" })
            .when_some(self.git_header.status.as_ref().and_then(|status| status["refName"].as_str()).map(str::to_owned), |view, branch| view.child(div().text_xs().child(format!("Branch: {branch}"))))
            .when(self.git_header.status.as_ref().is_some_and(|status| status["isDefaultRef"] == true), |view| view.child("Default branch"))
            .child(Input::new(&form.input).small().disabled(busy))
            .when(publish, |view| view
                .child(if form.discovering { "Checking provider readiness…" } else if form.providers.is_empty() { "No ready publishing provider. Configure source control credentials on this environment." } else { "Creates a remote repository and pushes this checkout. Review visibility before publishing." })
                .children(form.providers.iter().map(|provider| {
                    let selected = form.provider.as_ref() == Some(provider);
                    let provider = provider.clone();
                    Button::new(gpui_kit::SharedString::from(format!("git-publish-{provider}"))).small().ghost().label(provider.clone()).selected(selected).disabled(busy).on_click(cx.listener(move |app, _, _, cx| { if let Some(form) = app.git_header.form.as_mut() { form.provider = Some(provider.clone()); } cx.notify(); }))
                }))
                .when_some(form.remote.as_ref(), |view, input| view.child("Remote name").child(Input::new(input).small().disabled(busy)))
                .child(Checkbox::new("git-publish-https").label("Use HTTPS (default: SSH)").checked(form.https).disabled(busy).on_click(cx.listener(|app, checked: &bool, _, cx| { if let Some(form) = app.git_header.form.as_mut() { form.https = *checked; } cx.notify(); })))
                .child(Checkbox::new("git-publish-public").label("Public repository (default: private)").checked(form.public).disabled(busy).on_click(cx.listener(|app, checked: &bool, _, cx| { if let Some(form) = app.git_header.form.as_mut() { form.public = *checked; } cx.notify(); }))))
            .when(!publish, |view| view.children(form.paths.iter().map(|path| {
                let path = path.clone();
                Checkbox::new(gpui_kit::SharedString::from(format!("git-file-{path}"))).debug_selector({ let path = path.clone(); move || format!("git-file-{path}") }).label(path.clone()).checked(!form.excluded.contains(&path)).disabled(busy).on_click(cx.listener(move |app, checked: &bool, _, cx| {
                    if let Some(form) = app.git_header.form.as_mut() { form.excluded.retain(|p| p != &path); if !checked { form.excluded.push(path.clone()); } } cx.notify();
                }))
            })))
            .child(Button::new("git-form-submit").debug_selector(|| "git-form-submit".into()).small().primary().label(match form.action.as_str() { "publish" => "Create repository & publish", "commit_push" => "Commit & push selected files", "commit_push_pr" => "Commit, push & create PR", _ => "Commit selected files" }).disabled(busy || !valid || !self.connection_ready()).on_click(cx.listener(|app, _, _, cx| app.submit_git_form(cx))))
            .child(Button::new("git-form-cancel").small().ghost().label("Cancel").disabled(busy).on_click(cx.listener(|app, _, _, cx| { app.git_header.form = None; cx.notify(); })))
    }
}
fn valid_repository(value: &str) -> bool {
    let parts: Vec<_> = value.split('/').collect();
    parts.len() >= 2
        && parts.iter().all(|p| {
            !p.is_empty()
                && p.chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
                && *p != "."
                && *p != ".."
        })
}
