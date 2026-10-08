//! Global navigation uses compound identities; repository grouping only uses a
//! server-supplied canonical repository key, never a matching title or path.
use crate::NativeApp;
use gpui_kit::component::{
    ActiveTheme, Selectable, Sizable,
    button::{Button, ButtonVariants},
};
use gpui_kit::{AnyElement, Context, SharedString, div, prelude::*};
use std::collections::BTreeMap;
use t3code_desktop_gpui::shell;

#[derive(Clone)]
struct Member {
    origin: String,
    label: String,
    project: shell::Project,
    threads: Vec<shell::Thread>,
}
impl NativeApp {
    fn sidebar_members(&self, cx: &Context<Self>) -> Vec<Member> {
        let pages: Vec<_> = self
            .environment_hub
            .borrow()
            .pages
            .iter()
            .map(|(origin, page)| (origin.clone(), page.clone()))
            .collect();
        let mut members = Vec::new();
        for (origin, page) in pages {
            let collect = |app: &NativeApp| {
                app.shell
                    .projects
                    .iter()
                    .map(|project| Member {
                        origin: origin.clone(),
                        label: app
                            .environment_name
                            .clone()
                            .unwrap_or_else(|| origin.clone()),
                        project: project.clone(),
                        threads: app
                            .shell
                            .threads
                            .iter()
                            .filter(|thread| {
                                thread.project_id == project.id && thread.archived_at.is_none()
                            })
                            .cloned()
                            .collect(),
                    })
                    .collect::<Vec<_>>()
            };
            if page.entity_id() == cx.entity_id() {
                members.extend(collect(self));
            } else if let Some(page) = page.upgrade() {
                members.extend(collect(page.read(cx)));
            }
        }
        members
    }
    pub(crate) fn render_environment_sidebar(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let mut visible_origins: std::collections::BTreeSet<_> = self
            .environment_choices(cx)
            .into_iter()
            .filter(|(_, _, enabled)| *enabled)
            .map(|(origin, _, _)| origin)
            .collect();
        if self.commands.is_some() {
            visible_origins.insert(self.environment_scope.clone());
        }
        if visible_origins.len() < 2 {
            return self.render_sidebar(cx).into_any_element();
        }
        let (all, by_machine, alerts) = {
            let hub = self.environment_hub.borrow();
            (
                hub.all_environments,
                hub.group_by_machine,
                hub.alerts.clone(),
            )
        };
        let selector = Button::new("all-environments")
            .debug_selector(|| "all-environments".into())
            .label(if all {
                "Selected environment"
            } else {
                "All environments"
            })
            .ghost()
            .small()
            .selected(all)
            .on_click(cx.listener(|app, _, _, cx| {
                let mut hub = app.environment_hub.borrow_mut();
                hub.all_environments = !hub.all_environments;
                drop(hub);
                cx.notify();
            }));
        let mut view = div().flex().flex_col().h_full().min_w_0().child(selector);
        for (index, (tag, message)) in alerts.into_iter().enumerate() {
            view = view.child(
                Button::new(("environment-alert", index))
                    .debug_selector(move || format!("environment-alert-{index}"))
                    .label(message)
                    .ghost()
                    .small()
                    .on_click(cx.listener(move |app, _, window, cx| {
                        app.environment_hub
                            .borrow_mut()
                            .alerts
                            .retain(|(stored, _)| stored != &tag);
                        app.route_environment_notification(&tag, window, cx);
                    })),
            );
        }
        if !all {
            return view.child(self.render_sidebar(cx)).into_any_element();
        }
        view = view.child(
            Button::new("environment-grouping")
                .debug_selector(|| "environment-grouping".into())
                .label(if by_machine {
                    "Merge repositories"
                } else {
                    "Group by machine"
                })
                .ghost()
                .small()
                .on_click(cx.listener(|app, _, _, cx| {
                    let mut hub = app.environment_hub.borrow_mut();
                    hub.group_by_machine = !hub.group_by_machine;
                    drop(hub);
                    cx.notify();
                })),
        );
        let mut groups: BTreeMap<String, Vec<Member>> = BTreeMap::new();
        for member in self.sidebar_members(cx) {
            let key = member
                .project
                .repository_identity
                .as_ref()
                .and_then(|identity| identity["canonicalKey"].as_str())
                .filter(|key| !key.is_empty())
                .map(str::to_owned)
                .unwrap_or_else(|| format!("{}:{}", member.origin, member.project.id));
            let key = if by_machine {
                member.origin.clone()
            } else {
                key
            };
            groups.entry(key).or_default().push(member);
        }
        let muted = cx.theme().muted_foreground;
        let mut list = div()
            .id("all-environments-projects")
            .debug_selector(|| "all-environments-projects".into())
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .gap_2();
        for (index, (_, members)) in groups.into_iter().enumerate() {
            let title = if by_machine {
                members[0].label.clone()
            } else {
                members[0]
                    .project
                    .repository_identity
                    .as_ref()
                    .and_then(|identity| identity["displayName"].as_str())
                    .unwrap_or(&members[0].project.title)
                    .to_owned()
            };
            let mut group = div()
                .debug_selector(move || format!("environment-project-group-{index}"))
                .flex()
                .flex_col()
                .px_2()
                .gap_1()
                .child(div().text_sm().child(title));
            for member in members {
                group = group.child(div().text_xs().text_color(muted).child(if by_machine {
                    member.project.title.clone()
                } else {
                    member.label
                }));
                for thread in member.threads {
                    let origin = member.origin.clone();
                    let id = thread.id.clone();
                    let selector = format!("environment-thread-{}-{}", member.origin, id);
                    group = group.child(
                        Button::new(SharedString::from(selector.clone()))
                            .debug_selector(move || selector.clone())
                            .label(thread.title)
                            .ghost()
                            .small()
                            .selected(
                                self.environment_scope == origin
                                    && self.selected_thread.as_deref() == Some(&id),
                            )
                            .on_click(cx.listener(move |app, _, window, cx| {
                                if !app.can_change_checkout(cx) {
                                    return;
                                }
                                app.present_environment_page(&origin, window, cx);
                                let target =
                                    app.environment_hub.borrow().pages.get(&origin).cloned();
                                if let Some(target) = target {
                                    if target.entity_id() == cx.entity_id() {
                                        app.select_thread(&id, window, cx);
                                    } else {
                                        let _ = target.update(cx, |app, cx| {
                                            app.select_thread(&id, window, cx)
                                        });
                                    }
                                }
                            })),
                    );
                }
            }
            list = list.child(group);
        }
        view.child(list).into_any_element()
    }
}
