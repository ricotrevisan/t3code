//! Provider repository lookup feeds the existing receipt-tracked clone workflow.
use crate::NativeApp;
use gpui_kit::component::{
    Disableable, Selectable, Sizable,
    button::{Button, ButtonVariants},
};
use gpui_kit::{Context, Task, Window, div, prelude::*};
use serde_json::json;
use t3code_desktop_gpui::connection;
#[derive(Default)]
pub(crate) struct RepositoryChooser {
    pub provider: Option<&'static str>,
    pub busy: bool,
    pub generation: u64,
    pub environment: String,
    pub epoch: u64,
    pub resolved: Option<String>,
    task: Option<Task<()>>,
}
impl NativeApp {
    pub(crate) fn render_repository_chooser(
        &self,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let busy = self.repository_chooser.busy
            && self.repository_chooser.generation == self.generation
            && self.repository_chooser.environment == self.environment_scope;
        div().flex().flex_col().gap_1()
            .child("Clone source · enter a Git URL, or select a provider and enter owner/repository")
            .child(div().flex().flex_wrap().gap_1().children([None, Some("github"), Some("gitlab"), Some("bitbucket")].into_iter().enumerate().map(|(index, provider)| {
                Button::new(("clone-source", index)).debug_selector(move || format!("clone-source-{index}"))
                    .label(provider.unwrap_or("Git URL")).ghost().small().selected(self.repository_chooser.provider == provider)
                    .on_click(cx.listener(move |app, _, _, cx| {
                        app.repository_chooser.epoch += 1;
                        app.repository_chooser.provider = provider;
                        app.repository_chooser.busy = false;
                        app.repository_chooser.task = None;
                        app.repository_chooser.resolved = None;
                        cx.notify();
                    }))
            })))
            .when(self.repository_chooser.provider.is_some(), |view| view.child(
                Button::new("clone-lookup").debug_selector(|| "clone-lookup".into()).label(if busy { "Looking up…" } else { "Look up repository" }).ghost().small().disabled(!self.connection_ready() || busy)
                    .on_click(cx.listener(|app, _, window, cx| app.lookup_clone_repository(window, cx)))))
            .when_some((self.repository_chooser.generation == self.generation && self.repository_chooser.environment == self.environment_scope).then_some(self.repository_chooser.resolved.clone()).flatten(), |view, name| view.child(format!("Resolved repository: {name} · review URL and destination before starting clone")))
    }
    fn lookup_clone_repository(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(provider) = self.repository_chooser.provider else {
            return;
        };
        let repository = self.clone_remote.read(cx).value().trim().to_owned();
        if repository.is_empty()
            || (self.repository_chooser.busy
                && self.repository_chooser.generation == self.generation
                && self.repository_chooser.environment == self.environment_scope)
        {
            return;
        }
        let Some(sender) = self.commands.clone().filter(|_| self.connection_ready()) else {
            return;
        };
        let generation = self.generation;
        let environment = self.environment_scope.clone();
        self.repository_chooser.environment = environment.clone();
        self.repository_chooser.epoch += 1;
        let epoch = self.repository_chooser.epoch;
        self.repository_chooser.busy = true;
        self.repository_chooser.generation = generation;
        let window = window.window_handle();
        self.repository_chooser.task = Some(cx.spawn(async move |entity, cx| {
            let result = connection::call(
                &sender,
                "sourceControl.lookupRepository",
                json!({"provider":provider,"repository":repository}),
            )
            .await;
            let _ = window.update(cx, |_, window, cx| {
                entity.update(cx, |app, cx| {
                    if app.generation != generation
                        || app.environment_scope != environment
                        || app.repository_chooser.epoch != epoch
                    {
                        return;
                    }
                    app.repository_chooser.busy = false;
                    if app.clone_remote.read(cx).value().trim() != repository {
                        cx.notify();
                        return;
                    }
                    match result {
                        Ok(value) => {
                            let remote = value["sshUrl"]
                                .as_str()
                                .filter(|s| !s.is_empty())
                                .or_else(|| value["url"].as_str())
                                .map(str::to_owned);
                            if let Some(remote) = remote {
                                app.clone_remote
                                    .update(cx, |input, cx| input.set_value(remote, window, cx));
                                app.repository_chooser.resolved =
                                    value["nameWithOwner"].as_str().map(str::to_owned);
                                app.repository_chooser.provider = None;
                            } else {
                                app.report_error("Repository lookup returned no clone URL", cx);
                            }
                        }
                        Err(error) => app.report_error(error, cx),
                    }
                    cx.notify();
                })
            });
        }));
        cx.notify();
    }
}
