//! Durable device-local PTY presentation, never a launch/environment reconstruction.
use super::Drawer;
use crate::NativeApp;
use gpui_kit::Context;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, io, path::PathBuf};

#[derive(Clone, Default, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct Group {
    ids: Vec<String>,
    #[serde(default)]
    vertical: bool,
}
#[derive(Clone, Default, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct Layout {
    groups: Vec<Group>,
    active: Option<String>,
}
impl Layout {
    fn select(&mut self, id: &str) {
        if !self
            .groups
            .iter()
            .any(|group| group.ids.iter().any(|member| member == id))
        {
            self.groups.push(Group {
                ids: vec![id.into()],
                vertical: false,
            });
        }
        self.active = Some(id.into());
    }
    fn split(&mut self, ids: Vec<String>, vertical: bool) {
        for group in &mut self.groups {
            group.ids.retain(|id| !ids.contains(id));
        }
        self.groups.retain(|group| !group.ids.is_empty());
        self.active = ids.last().cloned();
        self.groups.push(Group { ids, vertical });
    }
    fn unsplit(&mut self, active: &str) {
        if let Some(index) = self
            .groups
            .iter()
            .position(|group| group.ids.iter().any(|id| id == active))
        {
            let group = self.groups.remove(index);
            for id in group.ids {
                self.groups.push(Group {
                    ids: vec![id],
                    vertical: false,
                });
            }
        }
    }
    fn reconcile(&mut self, valid: &[String]) {
        let mut assigned = Vec::new();
        self.groups.truncate(16);
        for group in &mut self.groups {
            group.ids.retain(|id| {
                if !valid.contains(id) || assigned.contains(id) || assigned.len() >= 16 {
                    return false;
                }
                assigned.push(id.clone());
                true
            });
            group.ids.truncate(4);
        }
        self.groups.retain(|group| !group.ids.is_empty());
        if self.active.as_ref().is_none_or(|id| !valid.contains(id)) {
            self.active = self
                .groups
                .first()
                .and_then(|group| group.ids.first())
                .cloned();
        }
    }
}

#[derive(Default)]
pub(super) struct Layouts {
    records: BTreeMap<String, Layout>,
    pub path: Option<PathBuf>,
    blocked: bool,
}
impl Layouts {
    pub fn unavailable() -> Self {
        Self {
            blocked: true,
            ..Self::default()
        }
    }
    pub fn load(path: PathBuf) -> io::Result<Self> {
        let records = Self::read(&path)?;
        Ok(Self {
            records,
            path: Some(path),
            blocked: false,
        })
    }
    fn read(path: &std::path::Path) -> io::Result<BTreeMap<String, Layout>> {
        match std::fs::read(path) {
            Ok(bytes) => serde_json::from_slice(&bytes).map_err(io::Error::other),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(BTreeMap::new()),
            Err(error) => Err(error),
        }
    }
    fn key(scope: &str, thread: &str) -> String {
        serde_json::to_string(&(scope, thread)).unwrap()
    }
    fn get(&self, scope: &str, thread: &str) -> Layout {
        self.records
            .get(&Self::key(scope, thread))
            .cloned()
            .unwrap_or_default()
    }
    fn set(&mut self, scope: &str, thread: &str, layout: Layout) -> io::Result<()> {
        if self.blocked {
            return Err(io::Error::other(
                "Terminal layout storage could not be read; existing file preserved",
            ));
        }
        let key = Self::key(scope, thread);
        if let Some(path) = &self.path {
            // Pages have independent stores, but one GPUI UI-thread writer. Merge
            // the current owner into the latest document, not a page's old copy.
            let mut records = Self::read(path)?;
            records.insert(key.clone(), layout.clone());
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let temp = path.with_extension(format!("{}.tmp", uuid::Uuid::new_v4()));
            std::fs::write(
                &temp,
                serde_json::to_vec(&records).map_err(io::Error::other)?,
            )?;
            if let Err(error) = std::fs::rename(&temp, path) {
                let _ = std::fs::remove_file(temp);
                return Err(error);
            }
        }
        self.records.insert(key, layout);
        Ok(())
    }
}
impl Drawer {
    #[cfg(test)]
    pub(crate) fn test_layout_storage(&mut self, path: PathBuf) {
        self.layouts = Layouts::load(path).unwrap();
    }
    pub(super) fn apply_layout(&mut self, scope: &str, thread: &str) {
        let layout = self.layouts.get(scope, thread);
        let active = self.active.as_ref();
        let group = layout
            .groups
            .iter()
            .find(|group| active.is_some_and(|id| group.ids.contains(id)));
        self.vertical = group.is_some_and(|group| group.vertical);
        self.split = group
            .filter(|group| group.ids.len() > 1)
            .map(|group| (scope.into(), thread.into(), group.ids.clone()));
    }
}
impl NativeApp {
    fn terminal_store_layout(
        &mut self,
        scope: &str,
        thread: &str,
        layout: Layout,
        cx: &mut Context<Self>,
    ) {
        if let Err(error) = self.terminal_drawer.layouts.set(scope, thread, layout) {
            self.terminal_drawer.local_error = Some(format!("Terminal layout not saved: {error}"));
        }
        if scope == self.environment_scope && Some(thread) == self.selected_thread.as_deref() {
            self.terminal_drawer.apply_layout(scope, thread);
        }
        cx.notify();
    }
    pub(super) fn terminal_remove_layout(
        &mut self,
        scope: &str,
        thread: &str,
        id: &str,
        cx: &mut Context<Self>,
    ) {
        let mut layout = self.terminal_drawer.layouts.get(scope, thread);
        for group in &mut layout.groups {
            group.ids.retain(|member| member != id);
        }
        layout.groups.retain(|group| !group.ids.is_empty());
        if layout.active.as_deref() == Some(id) {
            layout.active = layout
                .groups
                .first()
                .and_then(|group| group.ids.first())
                .cloned();
        }
        if scope == self.environment_scope
            && Some(thread) == self.selected_thread.as_deref()
            && self.terminal_drawer.active.as_deref() == Some(id)
        {
            self.terminal_drawer.active = layout.active.clone();
        }
        self.terminal_store_layout(scope, thread, layout, cx);
    }
    pub(super) fn terminal_select_tab(&mut self, id: &str, cx: &mut Context<Self>) {
        let Some(thread) = self.selected_thread.clone() else {
            return;
        };
        let scope = self.environment_scope.clone();
        let mut layout = self.terminal_drawer.layouts.get(&scope, &thread);
        layout.select(id);
        self.terminal_drawer.composition = None;
        self.terminal_drawer.active = Some(id.into());
        self.terminal_store_layout(&scope, &thread, layout, cx);
        cx.notify();
    }
    pub(super) fn terminal_record_split(
        &mut self,
        ids: Vec<String>,
        vertical: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(thread) = self.selected_thread.clone() else {
            return;
        };
        let scope = self.environment_scope.clone();
        let mut layout = self.terminal_drawer.layouts.get(&scope, &thread);
        layout.split(ids, vertical);
        self.terminal_store_layout(&scope, &thread, layout, cx);
    }
    pub(super) fn terminal_unsplit(&mut self, cx: &mut Context<Self>) {
        let Some(tab) = self.terminal_active() else {
            return;
        };
        let (scope, thread, id) = (tab.scope.clone(), tab.thread.clone(), tab.id.clone());
        let mut layout = self.terminal_drawer.layouts.get(&scope, &thread);
        layout.unsplit(&id);
        self.terminal_store_layout(&scope, &thread, layout, cx);
        cx.notify();
    }
    pub(super) fn terminal_restore_layout(&mut self, cx: &mut Context<Self>) {
        let Some(thread) = self.selected_thread.clone() else {
            return;
        };
        let scope = self.environment_scope.clone();
        let layout = self.terminal_drawer.layouts.get(&scope, &thread);
        if let Some(id) = layout.active.filter(|id| {
            self.terminal_drawer
                .tabs
                .iter()
                .any(|tab| tab.scope == scope && tab.thread == thread && &tab.id == id)
        }) {
            self.terminal_drawer.active = Some(id);
        }
        self.terminal_drawer.apply_layout(&scope, &thread);
        cx.notify();
    }
    pub(super) fn terminal_reconcile_layout(
        &mut self,
        terminals: &[serde_json::Value],
        cx: &mut Context<Self>,
    ) {
        let Some(thread) = self.selected_thread.clone() else {
            return;
        };
        let scope = self.environment_scope.clone();
        let valid = terminals
            .iter()
            .filter(|summary| summary["threadId"] == thread)
            .filter_map(|summary| summary["terminalId"].as_str().map(str::to_owned))
            .collect::<Vec<_>>();
        let mut layout = self.terminal_drawer.layouts.get(&scope, &thread);
        let old = layout.clone();
        layout.reconcile(&valid);
        if layout != old {
            self.terminal_store_layout(&scope, &thread, layout, cx);
        }
        self.terminal_restore_layout(cx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn r16_t_durable_groups_restore_direction_active_and_merge_environment_pages() {
        let root = std::env::temp_dir().join(format!("r16-t-{}", uuid::Uuid::new_v4()));
        let path = root.join("layouts.json");
        let mut a = Layouts::load(path.clone()).unwrap();
        let mut b = Layouts::load(path.clone()).unwrap();
        let mut layout = Layout::default();
        layout.split(vec!["one".into(), "two".into()], true);
        a.set("office", "t", layout.clone()).unwrap();
        b.set(
            "lab",
            "t",
            Layout {
                active: Some("one".into()),
                ..Layout::default()
            },
        )
        .unwrap();
        let restored = Layouts::load(path).unwrap();
        assert_eq!(restored.get("office", "t"), layout);
        assert_eq!(restored.get("lab", "t").active.as_deref(), Some("one"));
        assert_eq!(restored.get("office", "other"), Layout::default());
        layout.reconcile(&["two".into()]);
        assert_eq!(layout.groups[0].ids, vec!["two"]);
        layout.unsplit("two");
        assert!(!layout.groups[0].vertical);
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn r16_t_layout_failed_write_and_corrupt_document_preserve_previous_record() {
        let mut layouts = Layouts::default();
        let mut layout = Layout::default();
        layout.select("old");
        layouts.set("e", "t", layout.clone()).unwrap();
        layouts.path = Some(PathBuf::from("/dev/null/no.json"));
        let mut replacement = Layout::default();
        replacement.select("new");
        assert!(layouts.set("e", "t", replacement).is_err());
        assert_eq!(layouts.get("e", "t"), layout);
        let root = std::env::temp_dir().join(format!("r16-t-corrupt-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("layouts.json");
        std::fs::write(&path, b"not json").unwrap();
        layouts.path = Some(path.clone());
        assert!(layouts.set("e", "t", Layout::default()).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"not json");
        assert_eq!(layouts.get("e", "t"), layout);
        std::fs::remove_dir_all(root).unwrap();
    }
}
