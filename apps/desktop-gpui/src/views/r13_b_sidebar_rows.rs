//! Stable row entities reuse their painted subtree on unrelated app renders.
//! The first layout measures the real two-line row; no fixed height or assumed
//! typography is imposed on the sidebar.
use crate::{NativeApp, settings_appearance};
use gpui_kit::{
    AnyElement, Context, Entity, IntoElement, Pixels, Render, StyleRefinement, WeakEntity, Window,
    canvas, div, prelude::*,
};
use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    rc::Rc,
};
use t3code_desktop_gpui::{connection, shell::Thread};

#[derive(PartialEq)]
struct Inputs {
    thread: Thread,
    minute: i64,
    selected: bool,
    multi_selected: bool,
    unread: bool,
    woke: bool,
    snoozed: bool,
    ready: bool,
    offline: bool,
    queued: bool,
    priority: bool,
    project_title: Option<String>,
    generation: u64,
    ui_size: u32,
    ui_family: String,
    zoom: i8,
}

#[derive(Default)]
pub(crate) struct Rows {
    entries: RefCell<HashMap<(String, String), Entity<Row>>>,
    #[cfg(test)]
    constructions: Rc<Cell<usize>>,
}

struct Row {
    app: WeakEntity<NativeApp>,
    inputs: Inputs,
    height: Rc<Cell<Option<Pixels>>>,
    #[cfg(test)]
    constructions: Rc<Cell<usize>>,
}

impl Rows {
    pub(crate) fn retain(&self, environment: &str, threads: &[Thread]) {
        let ids: std::collections::HashSet<_> = threads.iter().map(|row| row.id.as_str()).collect();
        self.entries
            .borrow_mut()
            .retain(|(origin, id), _| origin == environment && ids.contains(id.as_str()));
    }

    #[cfg(test)]
    pub(crate) fn constructions(&self) -> usize {
        self.constructions.get()
    }
}

impl NativeApp {
    pub(crate) fn cached_sidebar_row(
        &self,
        thread: &Thread,
        now: chrono::DateTime<chrono::Utc>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let preferences = settings_appearance::current(cx);
        let inputs = Inputs {
            thread: thread.clone(),
            minute: now.timestamp() / 60,
            selected: self.selected_thread.as_ref() == Some(&thread.id),
            multi_selected: self.sidebar_state.selection_environment == self.environment_scope
                && self.sidebar_state.selection.contains(&thread.id),
            unread: self.sidebar_state.unread(&self.environment_scope, thread),
            woke: self
                .sidebar_state
                .woke(&self.environment_scope, thread, now),
            snoozed: t3code_desktop_gpui::sidebar::snoozed(thread, now),
            ready: self.connection_ready(),
            offline: matches!(
                self.connection_phase,
                connection::Phase::Offline
                    | connection::Phase::Reconnecting { .. }
                    | connection::Phase::Error(_)
            ),
            queued: self.queues.has_queued(&thread.id),
            priority: self.sidebar_state.priority,
            project_title: self
                .shell
                .projects
                .iter()
                .find(|project| project.id == thread.project_id)
                .map(|project| project.title.clone()),
            generation: self.generation,
            ui_size: preferences.font_size.to_bits(),
            ui_family: crate::settings_fonts::current(cx).families[0].clone(),
            zoom: preferences.zoom,
        };
        let key = (self.environment_scope.clone(), thread.id.clone());
        let mut entries = self.sidebar_rows.entries.borrow_mut();
        let entity = if let Some(entity) = entries.get(&key) {
            entity.update(cx, |row, cx| {
                if row.inputs != inputs {
                    row.inputs = inputs;
                    row.height.set(None);
                    cx.notify();
                }
            });
            entity.clone()
        } else {
            let parent = cx.entity().downgrade();
            #[cfg(test)]
            let constructions = self.sidebar_rows.constructions.clone();
            let entity = cx.new(|_| Row {
                app: parent,
                inputs,
                height: Rc::new(Cell::new(None)),
                #[cfg(test)]
                constructions,
            });
            entries.insert(key, entity.clone());
            entity
        };
        let height = entity.read(cx).height.get();
        match height {
            Some(height) => entity
                .cached(
                    StyleRefinement::default()
                        .w_full()
                        .h(height)
                        .flex_shrink_0(),
                )
                .into_any_element(),
            None => entity.into_any_element(),
        }
    }
}

impl Render for Row {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        #[cfg(test)]
        self.constructions.set(self.constructions.get() + 1);
        let row = self
            .app
            .update(cx, |app, cx| {
                app.render_sidebar_row(&self.inputs.thread, chrono::Utc::now(), cx)
                    .into_any_element()
            })
            .unwrap_or_else(|_| div().into_any_element());
        let height = self.height.clone();
        div()
            .relative()
            .w_full()
            .flex_shrink_0()
            .child(
                canvas(
                    move |bounds, _, _| height.set(Some(bounds.size.height)),
                    |_, _, _, _| {},
                )
                .absolute()
                .size_full(),
            )
            .child(row)
    }
}
