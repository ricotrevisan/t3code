//! The right panel: a tab strip over the Inspector accordion, Changes, and Agents.
//! The Inspector shows one open section at a time; `reveal_inspector` is the one
//! way entry points elsewhere in the window bring a section into view.

use gpui_kit::assets::IconName;
use gpui_kit::base::Selectable;
use gpui_kit::component::{
    ActiveTheme as _, Sizable,
    button::{Button, ButtonVariants as _},
};
use gpui_kit::{AnyElement, App, Context, Div, Window, div, prelude::*};
use serde_json::json;
use t3code_desktop_gpui::connection;

use super::list_row;
use crate::{InspectorSection, NativeApp, RightTab};

mod actions;
mod branches;
mod clone;
mod files;
mod panels;
mod project;
mod thread;

const SECTIONS: [(InspectorSection, &str, &str); 6] = [
    (InspectorSection::Project, "section-project", "Project"),
    (
        InspectorSection::Worktrees,
        "section-worktrees",
        "Branches & worktrees",
    ),
    (
        InspectorSection::ProjectActions,
        "section-project-actions",
        "Project actions",
    ),
    (InspectorSection::Clone, "section-clone", "Clone"),
    (InspectorSection::Files, "files", "Files"),
    (InspectorSection::Thread, "section-thread", "Thread"),
];

/// A short visible label stacked above an input.
fn field(label: &'static str, input: impl IntoElement, cx: &App) -> Div {
    div()
        .flex()
        .flex_col()
        .gap_1()
        .flex_1()
        .min_w_0()
        .child(
            div()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(label),
        )
        .child(input)
}

impl NativeApp {
    /// Opens `section` (or closes the open one with `None`) and runs the side
    /// effects that belong to entering or leaving a section.
    pub(crate) fn set_inspector_section(
        &mut self,
        section: Option<InspectorSection>,
        cx: &mut Context<Self>,
    ) {
        let previous = self.inspector_section;
        self.inspector_section = section;
        if previous == section {
            cx.notify();
            return;
        }
        if previous == Some(InspectorSection::Clone) {
            self.enqueue_subscription(connection::Command::Unsubscribe("project-clones".into()), cx);
        }
        match section {
            Some(InspectorSection::Clone) => {
                self.enqueue_subscription(connection::Command::Subscribe {
                    key: "project-clones".into(), tag: "subscribeProjectClones".into(), payload: json!({}),
                }, cx);
            }
            Some(InspectorSection::ProjectActions) => self.project_actions(None, cx),
            Some(InspectorSection::Worktrees) => self.worktree_operation("vcs.listRefs", false, cx),
            _ => {}
        }
        cx.notify();
    }

    /// Shows the right panel on the Inspector tab with `section` open and its
    /// first input focused.
    pub(crate) fn reveal_inspector(
        &mut self,
        section: InspectorSection,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.set_inspector_open(true, cx);
        self.browser_panel.update(cx, |panel, cx| panel.deactivate(cx));
        self.right_tab = RightTab::Inspector;
        // An already open section reloads what opening it would have loaded.
        let already_open = self.inspector_section == Some(section);
        self.set_inspector_section(Some(section), cx);
        if already_open {
            match section {
                InspectorSection::ProjectActions => self.project_actions(None, cx),
                InspectorSection::Worktrees => self.worktree_operation("vcs.listRefs", false, cx),
                _ => {}
            }
        }
        let input = match section {
            InspectorSection::Project => &self.title,
            InspectorSection::Worktrees => &self.worktree_ref,
            InspectorSection::ProjectActions => &self.action_name,
            InspectorSection::Clone => &self.clone_remote,
            InspectorSection::Files => &self.content_query,
            InspectorSection::Thread => &self.custom_snooze,
        };
        input.update(cx, |input, cx| input.focus(window, cx));
        cx.notify();
    }

    pub(crate) fn render_inspector(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let border = cx.theme().border;
        let group_box = cx.theme().group_box;
        div()
            .id("inspector")
            .debug_selector(|| "inspector".into())
            .when(!self.inspector_open, |view| view.hidden())
            .flex()
            .flex_col()
            .gap_2()
            .size_full()
            .p_3()
            .bg(group_box)
            .border_l_1()
            .border_color(border)
            .child(
                div().flex().gap_1().children(
                    [
                        ("right-tab-inspector", "Inspector", RightTab::Inspector),
                        ("right-tab-changes", "Changes", RightTab::Changes),
                        ("right-tab-agents", "Agents", RightTab::Agents),
                        ("right-tab-pull-requests", "PRs", RightTab::PullRequests),
                    ]
                    .into_iter()
                    .map(|(id, label, tab)| {
                        Button::new(id)
                            .debug_selector(move || id.into())
                            .label(label)
                            .ghost()
                            .small()
                            .selected(self.right_tab == tab && !self.browser_panel.read(cx).browser_active())
                            .on_click(cx.listener(move |app, _, window, cx| {
                                app.browser_panel.update(cx, |panel, cx| panel.deactivate(cx));
                                if tab == RightTab::PullRequests {
                                    app.open_pull_requests(window, cx);
                                } else {
                                    app.right_tab = tab;
                                    cx.notify();
                                }
                            }))
                    }),
                ),
            )
            .child(self.browser_panel.clone())
            // Only the active tab's body is in the tree: a hidden body would still
            // take its flex_1 space and push the other panels below the fold.
            .when(self.right_tab == RightTab::Inspector && !self.browser_panel.read(cx).browser_active(), |view| {
                view.child(self.render_inspector_body(cx))
            })
            .when(self.right_tab == RightTab::Changes && !self.browser_panel.read(cx).browser_active(), |view| {
                view.child(self.render_changes_panel(cx))
            })
            .when(self.right_tab == RightTab::Agents && !self.browser_panel.read(cx).browser_active(), |view| {
                view.child(self.render_agents_panel(cx))
            })
            .when(self.right_tab == RightTab::PullRequests && !self.browser_panel.read(cx).browser_active(), |view| {
                view.child(self.render_pull_requests_panel(cx))
            })
    }

    fn render_inspector_body(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let danger = cx.theme().danger;
        let mut body = div()
            .id("inspector-body")
            .flex()
            .flex_col()
            .gap_2()
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .children(
                self.storage_error
                    .clone()
                    .map(|error| div().text_color(danger).child(error)),
            )
            .children(self.render_setup_card(cx));
        for (section, id, label) in SECTIONS {
            body = body.child(self.render_section(section, id, label, cx));
        }
        body
    }

    fn render_section(
        &self,
        section: InspectorSection,
        id: &'static str,
        label: &'static str,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let open = self.inspector_section == Some(section);
        let content: Option<AnyElement> = open.then(|| match section {
            InspectorSection::Project => self.render_project_section(cx).into_any_element(),
            InspectorSection::Worktrees => self.render_worktrees(cx).into_any_element(),
            InspectorSection::ProjectActions => self.render_project_actions(cx).into_any_element(),
            InspectorSection::Clone => self.render_clone(cx).into_any_element(),
            InspectorSection::Files => self.render_files(cx).into_any_element(),
            InspectorSection::Thread => self.render_thread_section(cx).into_any_element(),
        });
        div()
            .flex()
            .flex_col()
            .gap_2()
            .min_w_0()
            .p_1()
            .rounded_md()
            .border_1()
            .border_color(cx.theme().border)
            .child(
                list_row(
                    id,
                    Some(if open {
                        IconName::ChevronDown
                    } else {
                        IconName::ChevronRight
                    }),
                    label,
                    false,
                )
                .aria_expanded(open)
                .on_click(cx.listener(move |app, _, _, cx| {
                    let next = (app.inspector_section != Some(section)).then_some(section);
                    app.set_inspector_section(next, cx);
                })),
            )
            .children(content.map(|content| div().px_1().pb_1().child(content)))
    }
}
