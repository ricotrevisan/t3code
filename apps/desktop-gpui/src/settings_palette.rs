//! Searchable global destinations share the same keyboard and click dispatch.
use crate::{NativeApp, settings_appearance::Appearance};
use gpui_kit::{Context, Window};
use t3code_desktop_gpui::palette;

#[derive(Clone)]
pub(crate) enum Target {
    Command(palette::Command),
    Thread(String),
    Project(String),
    NewThread(String),
    Appearance(Appearance),
    ToggleTheme,
    UsageLimits,
    Settings(&'static str),
}
#[derive(Clone)]
pub(crate) struct Item {
    pub label: String,
    pub target: Target,
}
enum Destination {
    Thread(usize, Option<usize>),
    Project(usize),
    NewThread(usize),
}
#[derive(Default)]
pub(crate) struct Cache {
    key: Option<(String, u64)>,
    destinations: Vec<(String, Destination)>,
    query_key: Option<(String, bool)>,
    matches: std::rc::Rc<Vec<Item>>,
}

impl NativeApp {
    pub(crate) fn palette_items(&self, cx: &Context<Self>) -> std::rc::Rc<Vec<Item>> {
        let query = self.palette_query.read(cx).value().trim().to_lowercase();
        let mut items = Vec::new();
        let key = (self.environment_scope.clone(), self.shell.revision);
        let mut cache = self.palette_cache.borrow_mut();
        if cache.key.as_ref() != Some(&key) {
            let projects = self
                .shell
                .projects
                .iter()
                .enumerate()
                .map(|(index, p)| (p.id.as_str(), index))
                .collect::<std::collections::HashMap<_, _>>();
            let mut recent = std::collections::HashMap::new();
            let mut threads = self
                .shell
                .threads
                .iter()
                .enumerate()
                .filter(|(_, t)| t.archived_at.is_none())
                .collect::<Vec<_>>();
            threads.sort_by(|(_, a), (_, b)| b.updated_at.cmp(&a.updated_at));
            for thread in &self.shell.threads {
                let time = recent
                    .entry(thread.project_id.as_str())
                    .or_insert(&thread.updated_at);
                if &thread.updated_at > *time {
                    *time = &thread.updated_at;
                }
            }
            let mut destinations = Vec::new();
            for (index, thread) in threads {
                let project_index = projects.get(thread.project_id.as_str()).copied();
                let project = project_index
                    .map(|i| self.shell.projects[i].title.as_str())
                    .unwrap_or("Project");
                destinations.push((
                    format!("{} · {project}", thread.title).to_lowercase(),
                    Destination::Thread(index, project_index),
                ));
            }
            let mut ordered = self.shell.projects.iter().enumerate().collect::<Vec<_>>();
            ordered.sort_by_key(|(_, p)| std::cmp::Reverse(recent.get(p.id.as_str()).copied()));
            for (index, project) in ordered {
                destinations.push((
                    format!("Open project: {}", project.title).to_lowercase(),
                    Destination::Project(index),
                ));
                destinations.push((
                    format!("New thread in {}", project.title).to_lowercase(),
                    Destination::NewThread(index),
                ));
            }
            cache.destinations = destinations;
            cache.key = Some(key);
            cache.query_key = None;
        }
        let query_key = (query.clone(), self.selected_thread.is_some());
        if cache.query_key.as_ref() == Some(&query_key) {
            return cache.matches.clone();
        }
        // Filter the cached search index before materializing destination results.
        items.extend(
            cache
                .destinations
                .iter()
                .filter(|(search, _)| search.contains(&query))
                .map(|(_, destination)| match *destination {
                    Destination::Thread(index, project) => {
                        let thread = &self.shell.threads[index];
                        let project = project
                            .map(|i| self.shell.projects[i].title.as_str())
                            .unwrap_or("Project");
                        Item {
                            label: format!("{} · {project}", thread.title),
                            target: Target::Thread(thread.id.clone()),
                        }
                    }
                    Destination::Project(index) => {
                        let project = &self.shell.projects[index];
                        Item {
                            label: format!("Open project: {}", project.title),
                            target: Target::Project(project.id.clone()),
                        }
                    }
                    Destination::NewThread(index) => {
                        let project = &self.shell.projects[index];
                        Item {
                            label: format!("New thread in {}", project.title),
                            target: Target::NewThread(project.id.clone()),
                        }
                    }
                }),
        );
        items.extend(
            palette::matching("", self.selected_thread.is_some())
                .into_iter()
                .map(|entry| Item {
                    label: entry.label.into(),
                    target: Target::Command(entry.command),
                }),
        );
        items.push(Item { label: "Open usage limits".into(), target: Target::UsageLimits });
        items.push(Item {
            label: "Toggle theme".into(),
            target: Target::ToggleTheme,
        });
        for mode in Appearance::ALL {
            items.push(Item {
                label: format!(
                    "Theme: {}{}",
                    mode.label(),
                    match mode {
                        Appearance::Light => " — Zed One Light",
                        Appearance::Dark => " — Zed One Dark",
                        Appearance::System => " — follow OS appearance",
                    }
                ),
                target: Target::Appearance(mode),
            });
        }
        for category in [
            "Appearance",
            "General",
            "Providers",
            "Connection",
            "Notifications",
            "Keybindings",
            "About",
        ] {
            items.push(Item {
                label: format!("Settings: {category}"),
                target: Target::Settings(category),
            });
        }
        items.retain(|item| item.label.to_lowercase().contains(&query));
        cache.query_key = Some(query_key);
        cache.matches = std::rc::Rc::new(items);
        cache.matches.clone()
    }
    pub(crate) fn run_palette_item(
        &mut self,
        target: Target,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.close_palette(window, cx);
        match target {
            Target::Command(command) => self.execute_command(command, window, cx),
            Target::Thread(id) => self.select_thread(&id, window, cx),
            Target::Project(id) => {
                if let Some(thread) = self
                    .shell
                    .threads
                    .iter()
                    .filter(|thread| thread.project_id == id && thread.archived_at.is_none())
                    .max_by_key(|thread| &thread.updated_at)
                    .map(|thread| thread.id.clone())
                {
                    self.select_thread(&thread, window, cx);
                } else {
                    self.palette_new_thread(id, window, cx);
                }
            }
            Target::NewThread(id) => {
                self.palette_new_thread(id, window, cx);
            }
            Target::Appearance(mode) => self.set_appearance(mode, window, cx),
            Target::ToggleTheme => {
                use gpui_kit::component::ActiveTheme;
                let mode = if cx.theme().is_dark() {
                    Appearance::Light
                } else {
                    Appearance::Dark
                };
                self.set_appearance(mode, window, cx);
            }
            Target::UsageLimits => self.toggle_usage_limits(cx),
            Target::Settings(category) => {
                self.open_settings(category, window, cx);
                self.refresh_settings_providers(false, cx);
            }
        }
    }
    /// Restore provisional context on every rejected validation.
    fn palette_new_thread(&mut self, id: String, window: &mut Window, cx: &mut Context<Self>) {
        if !self.can_change_checkout(cx) {
            return;
        }
        let previous_project = self.selected_project.replace(id);
        let previous_thread = self.selected_thread.clone();
        self.new_thread(window, cx);
        if self.selected_thread == previous_thread {
            self.selected_project = previous_project;
        }
    }
    pub(crate) fn submit_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(item) = self.palette_items(cx).get(self.palette_index) {
            self.run_palette_item(item.target.clone(), window, cx);
        }
    }
    fn set_appearance(&mut self, mode: Appearance, window: &mut Window, cx: &mut Context<Self>) {
        if let Err(error) =
            crate::theme::update(window, cx, |preferences| preferences.appearance = mode)
        {
            self.report_error(format!("Could not save appearance: {error}"), cx);
        }
        cx.notify();
    }
}
