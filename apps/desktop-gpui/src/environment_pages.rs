//! Retained environment entities are the feature-state boundary. Navigation never
//! moves a checkout, draft, receipt task or input entity into another environment.
//! The window root retains pages; the shared index holds only weak references.
use crate::NativeApp;
use gpui_kit::{AppContext, Context, Entity, WeakEntity, Window};
use std::{collections::BTreeMap, rc::Rc};
use t3code_desktop_gpui::{connection, connection_sessions, credentials, notifications};

pub(crate) struct Hub {
    root: WeakEntity<NativeApp>,
    pub(crate) pages: BTreeMap<String, WeakEntity<NativeApp>>,
    selected: Option<String>,
    phases: BTreeMap<String, String>,
    badge_counts: BTreeMap<String, usize>,
    pub(crate) all_environments: bool,
    pub(crate) group_by_machine: bool,
    pub(crate) alerts: Vec<(String, String)>,
}
impl Hub {
    pub(crate) fn new(root: WeakEntity<NativeApp>) -> Self {
        Self {
            root,
            pages: Default::default(),
            selected: None,
            phases: Default::default(),
            badge_counts: Default::default(),
            all_environments: false,
            group_by_machine: false,
            alerts: Vec::new(),
        }
    }
}
impl NativeApp {
    pub(crate) fn notify_environment_root(&self, cx: &mut Context<Self>) {
        let root = self.environment_hub.borrow().root.clone();
        if root.entity_id() == cx.entity_id() {
            cx.notify();
        } else {
            cx.defer(move |cx| {
                let _ = root.update(cx, |_, cx| cx.notify());
            });
        }
    }
    pub(crate) fn sync_environment_badge(&mut self) {
        let mut hub = self.environment_hub.borrow_mut();
        hub.badge_counts.insert(
            self.environment_scope.clone(),
            self.notification_badge.len(),
        );
        self.notification_platform
            .badge(hub.badge_counts.values().sum());
    }
    pub(crate) fn clear_environment_alerts(&self, cx: &mut Context<Self>) {
        self.environment_hub.borrow_mut().alerts.retain(|(tag, _)| {
            notifications::thread_from_tag(tag, &self.environment_scope).is_none()
        });
        self.notify_environment_root(cx);
    }
    pub(crate) fn background_environment_alert(
        &self,
        thread: &str,
        message: String,
        cx: &mut Context<Self>,
    ) {
        let tag = notifications::tag(&self.environment_scope, thread);
        let mut hub = self.environment_hub.borrow_mut();
        hub.alerts.retain(|(stored, _)| stored != &tag);
        hub.alerts.push((
            tag,
            format!(
                "{} · {message}",
                self.environment_name
                    .as_deref()
                    .unwrap_or(&self.environment_scope)
            ),
        ));
        if hub.alerts.len() > 32 {
            hub.alerts.remove(0);
        }
        let selected = hub
            .selected
            .as_ref()
            .and_then(|id| hub.pages.get(id))
            .cloned();
        drop(hub);
        if let Some(selected) = selected
            && selected.entity_id() != cx.entity_id()
        {
            let _ = selected.update(cx, |_, cx| cx.notify());
        }
        self.notify_environment_root(cx);
    }
    pub(crate) fn environment_is_selected(&self) -> bool {
        let hub = self.environment_hub.borrow();
        hub.selected
            .as_ref()
            .and_then(|origin| hub.pages.get(origin))
            .is_none_or(|page| page.entity_id() == self.environment_page_id)
    }
    pub(crate) fn environment_phase(&self, origin: &str) -> String {
        if origin == self.environment_scope {
            return self.connection_state_text().to_owned();
        }
        self.environment_hub
            .borrow()
            .phases
            .get(origin)
            .cloned()
            .unwrap_or_else(|| "Offline".into())
    }
    pub(crate) fn publish_environment_phase(&self, cx: &mut Context<Self>) {
        self.environment_hub.borrow_mut().phases.insert(
            self.environment_scope.clone(),
            self.connection_state_text().to_owned(),
        );
        self.notify_environment_root(cx);
    }
    pub(crate) fn presented_environment(&self, cx: &Context<Self>) -> Option<Entity<Self>> {
        let hub = self.environment_hub.borrow();
        if hub.root.entity_id() != cx.entity_id() {
            return None;
        }
        hub.selected
            .as_ref()
            .and_then(|id| hub.pages.get(id))
            .and_then(WeakEntity::upgrade)
            .filter(|page| page.entity_id() != cx.entity_id())
    }
    fn register_environment_page(&self, cx: &Context<Self>) {
        let mut hub = self.environment_hub.borrow_mut();
        let was_selected = hub
            .selected
            .as_ref()
            .and_then(|origin| hub.pages.get(origin))
            .is_some_and(|page| page.entity_id() == cx.entity_id());
        hub.pages
            .retain(|_, page| page.entity_id() != cx.entity_id());
        hub.pages
            .insert(self.environment_scope.clone(), cx.entity().downgrade());
        if hub.selected.is_none() || was_selected {
            hub.selected = Some(self.environment_scope.clone());
        }
    }
    pub(crate) fn ensure_environment_pages(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.environment_hub.borrow().root.entity_id() != cx.entity_id() {
            return;
        }
        self.register_environment_page(cx);
        // Headless fixtures never read credentials or launch network workers.
        if self.saved_window.is_none() {
            return;
        }
        for (origin, _, enabled) in self.environment_choices(cx) {
            if enabled && !self.environment_hub.borrow().pages.contains_key(&origin) {
                self.create_environment_page(&origin, window, cx);
            }
        }
    }
    pub(crate) fn create_environment_page(
        &mut self,
        origin: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<Self> {
        self.register_environment_page(cx);
        if let Some(page) = self
            .environment_hub
            .borrow()
            .pages
            .get(origin)
            .and_then(WeakEntity::upgrade)
        {
            return page;
        }
        let hub = Rc::clone(&self.environment_hub);
        let store = self.draft_store.clone();
        let preferences = self.notification_preferences;
        let follow_up = self.follow_up_behavior;
        let page = cx.new(|cx| {
            let mut page = Self::new_scoped(window, cx, false, false);
            page.environment_hub = hub;
            page.environment_scope = origin.to_owned();
            page.server.update(cx, |input, cx| {
                input.set_value(origin.to_owned(), window, cx)
            });
            page.draft_store = store;
            page.notification_preferences = preferences;
            page.follow_up_behavior = follow_up;
            page.use_dev_credential = self.use_dev_credential;
            page.register_environment_page(cx);
            if !cfg!(test) {
                page.credential_operation(credentials::Operation::Load, cx);
            }
            page
        });
        let root = self.environment_hub.borrow().root.clone();
        if root.entity_id() == cx.entity_id() {
            self.environment_pages.push(page.clone());
        } else {
            let retained = page.clone();
            let _ = root.update(cx, |root, _| root.environment_pages.push(retained));
        }
        page
    }
    pub(crate) fn present_environment_page(
        &mut self,
        origin: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.hide_presented_environment(cx);
        let page = self.create_environment_page(origin, window, cx);
        self.environment_hub.borrow_mut().selected = Some(origin.to_owned());
        if page.entity_id() == cx.entity_id() {
            self.composer
                .update(cx, |input, cx| input.focus(window, cx));
        } else {
            page.update(cx, |page, cx| {
                page.composer
                    .update(cx, |input, cx| input.focus(window, cx))
            });
        }
        self.notify_environment_root(cx);
    }
    fn hide_presented_environment(&mut self, cx: &mut Context<Self>) {
        let selected = {
            let hub = self.environment_hub.borrow();
            hub.selected
                .as_ref()
                .and_then(|id| hub.pages.get(id))
                .cloned()
        };
        if let Some(selected) = selected {
            if selected.entity_id() == cx.entity_id() {
                self.browser_panel
                    .update(cx, |panel, _| panel.sync_visibility(false, true));
            } else {
                let _ = selected.update(cx, |app, cx| {
                    app.browser_panel
                        .update(cx, |panel, _| panel.sync_visibility(false, true))
                });
            }
        }
    }
    pub(crate) fn can_retire_environment(&mut self, origin: &str, cx: &mut Context<Self>) -> bool {
        let page = self
            .environment_hub
            .borrow()
            .pages
            .get(origin)
            .and_then(WeakEntity::upgrade);
        let blocked = |app: &NativeApp, cx: &gpui_kit::App| {
            app.file_editor
                .dirty(app.file_contents.read(cx).value().as_str())
                || app.pending_actions > 0
                || app.checkout_transition_pending()
        };
        let busy = page.is_some_and(|page| {
            if page.entity_id() == cx.entity_id() {
                blocked(self, cx)
            } else {
                blocked(page.read(cx), cx)
            }
        });
        if busy {
            self.report_error("Save or discard file edits and finish in-flight work before disabling or forgetting this environment", cx);
            return false;
        }
        true
    }
    pub(crate) fn retire_environment_page(&mut self, origin: &str, cx: &mut Context<Self>) {
        let page = self.environment_hub.borrow().pages.get(origin).cloned();
        if let Some(page) = page {
            if page.entity_id() == cx.entity_id() {
                self.disconnect(cx);
            } else {
                let _ = page.update(cx, |app, cx| {
                    app.browser_panel
                        .update(cx, |panel, _| panel.sync_visibility(false, true));
                    app.disconnect(cx);
                    app.publish_environment_phase(cx);
                });
            }
        }
        self.publish_environment_phase(cx);
    }
    pub(crate) fn resume_environment_page(&mut self, origin: &str, cx: &mut Context<Self>) {
        let page = self.environment_hub.borrow().pages.get(origin).cloned();
        if let Some(page) = page
            && !cfg!(test)
        {
            if page.entity_id() == cx.entity_id() {
                self.credential_operation(credentials::Operation::Load, cx);
            } else {
                let _ = page.update(cx, |app, cx| {
                    app.credential_operation(credentials::Operation::Load, cx)
                });
            }
        }
    }
    pub(crate) fn route_environment_notification(
        &mut self,
        tag: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let target = self
            .environment_hub
            .borrow()
            .pages
            .iter()
            .find_map(|(origin, page)| {
                notifications::thread_from_tag(tag, origin).map(|_| (origin.clone(), page.clone()))
            });
        let Some((origin, page)) = target else {
            self.open_notified_thread(tag, window, cx);
            return;
        };
        self.hide_presented_environment(cx);
        if page.entity_id() == cx.entity_id() {
            self.open_notified_thread(tag, window, cx);
        } else {
            let _ = page.update(cx, |page, cx| page.open_notified_thread(tag, window, cx));
        }
        self.environment_hub.borrow_mut().selected = Some(origin);
        self.notify_environment_root(cx);
    }

    pub(crate) fn start_owned_session(
        &mut self,
        session: connection::Session,
        cx: &mut Context<Self>,
    ) {
        self.apply_connection_phase(connection::Phase::Connecting, cx);
        let generation = self.generation;
        self.connection_task = Some(cx.spawn(async move |entity, cx| {
            // Registry keys are server identities from an authenticated receipt,
            // not display labels, origins, or stale saved metadata.
            let config = session
                .call("server.getConfig", serde_json::json!({}))
                .await;
            let installed = entity.update(cx, |app, cx| {
                if app.generation != generation {
                    session.commands.close();
                    session.updates.close();
                    return false;
                }
                let id = config
                    .as_ref()
                    .ok()
                    .and_then(|config| config["environment"]["environmentId"].as_str())
                    .filter(|id| !id.trim().is_empty())
                    .map(str::to_owned);
                let Some(id) = id else {
                    session.commands.close();
                    session.updates.close();
                    app.apply_connection_phase(
                        connection::Phase::Error(config.err().unwrap_or_else(|| {
                            "Server did not provide an environment identity".into()
                        })),
                        cx,
                    );
                    return false;
                };
                app.install_environment_session(id, session, cx);
                if let Ok(config) = config {
                    app.accept_config_snapshot(config, cx);
                }
                true
            });
            if !matches!(installed, Ok(true)) {
                return;
            }
            loop {
                let Ok(receive) = entity.update(cx, |app, _| app.sessions.receive()) else {
                    break;
                };
                let Some(incoming) = receive.await else {
                    break;
                };
                let applied =
                    entity.update(cx, |app, cx| app.apply_environment_incoming(incoming, cx));
                if !matches!(applied, Ok(true)) {
                    break;
                }
            }
        }));
    }
    pub(crate) fn install_environment_session(
        &mut self,
        id: String,
        session: connection::Session,
        cx: &mut Context<Self>,
    ) {
        let duplicate = self.environment_hub.borrow().pages.values().any(|page| {
            page.entity_id() != cx.entity_id()
                && page
                    .upgrade()
                    .is_some_and(|page| page.read(cx).sessions.get(&id).is_some())
        });
        if duplicate {
            session.commands.close();
            session.updates.close();
            self.apply_connection_phase(
                connection::Phase::Error(
                    "This server environment is already connected through another saved endpoint"
                        .into(),
                ),
                cx,
            );
            return;
        }
        match self.sessions.insert(id.clone(), session) {
            Ok(owner) => {
                let _ = self.sessions.select(&id);
                self.commands = self.sessions.commands(&owner);
                self.publish_environment_phase(cx);
            }
            Err(error) => self.report_error(error, cx),
        }
    }
    pub(crate) fn apply_environment_incoming(
        &mut self,
        incoming: connection_sessions::Incoming,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some((event, update)) = self.sessions.apply_with_update(incoming) else {
            return true;
        };
        if !self.sessions.owns(&event.owner) {
            self.generation += 1;
            self.environment_identity_verified = false;
            self.commands = None;
            self.clear_notifications(cx);
            self.clear_checkout_stops();
            self.clear_queue_subscriptions();
            self.action_runs_connection_lost();
            let phase = self
                .sessions
                .get(&event.owner.environment_id)
                .map(|session| session.phase.clone())
                .unwrap_or(connection::Phase::Offline);
            self.apply_connection_phase(phase, cx);
            self.publish_environment_phase(cx);
            return false;
        }
        if matches!(event.kind, connection_sessions::EventKind::Resynchronizing) {
            // Per-page UI epochs remain monotonic; navigation never changes them.
            self.generation += 1;
            self.environment_identity_verified = false;
            self.pending_actions = 0;
            self.perform("server.getConfig", serde_json::json!({}), None, cx);
        }
        if let Some(update) = update
            && let Err(error) = self.apply_connection_update(update, cx)
        {
            self.sessions.remove(&event.owner.environment_id);
            self.commands = None;
            self.apply_connection_phase(connection::Phase::Error(error), cx);
            self.publish_environment_phase(cx);
            return false;
        }
        self.publish_environment_phase(cx);
        cx.notify();
        true
    }
    pub(crate) fn verify_environment_config(
        &mut self,
        config: &serde_json::Value,
        cx: &mut Context<Self>,
    ) -> bool {
        if let Some(session) = self.sessions.selected() {
            if config["environment"]["environmentId"].as_str()
                != Some(session.owner().environment_id.as_str())
            {
                self.disconnect(cx);
                self.apply_connection_phase(connection::Phase::Error("Environment identity changed; reconnect explicitly before sending more work".into()), cx);
                return false;
            }
            self.environment_identity_verified = true;
            if self.shell.synchronized {
                self.connection_phase = connection::Phase::Ready;
            }
        }
        true
    }
    pub(crate) fn send_thread_selection(&mut self, thread: Option<String>) -> Result<(), String> {
        if let Some(session) = self.sessions.selected() {
            let owner = session.owner();
            self.sessions.select_thread(&owner, thread)
        } else if let Some(sender) = &self.commands {
            // Existing single-environment command-channel harness.
            sender
                .try_send(connection::Command::SelectThread(thread))
                .map_err(|error| error.to_string())
        } else {
            Err("Not connected".into())
        }
    }
}
