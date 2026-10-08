use gpui_kit::component::{
    ActiveTheme as _, Disableable, Sizable,
    button::{Button, ButtonVariants as _},
    input::Input,
};
use gpui_kit::{Context, div, prelude::*};
use t3code_desktop_gpui::palette;

use super::field;
use crate::NativeApp;

impl NativeApp {
    pub(super) fn render_project_section(
        &self,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let current_browser = self.directory_browser.generation == self.generation;
        div()
            .flex()
            .flex_col()
            .gap_2()
            .child(if self.directory_browser.adding {
                "Add project on this environment"
            } else {
                "Selected project"
            })
            .when(self.directory_browser.adding, |view| {
                view.child(
                    Button::new("project-cancel-add")
                        .debug_selector(|| "project-cancel-add".into())
                        .label("Cancel")
                        .ghost()
                        .small()
                        .on_click(cx.listener(|app, _, window, cx| {
                            app.directory_browser.adding = false;
                            app.directory_browser.task = None;
                            app.directory_browser.request += 1;
                            app.directory_browser.loading = false;
                            app.directory_browser.parent = None;
                            app.directory_browser.entries.clear();
                            app.directory_browser.error = None;
                            if let Some(project) = app
                                .selected_project
                                .as_ref()
                                .and_then(|id| {
                                    app.shell.projects.iter().find(|project| &project.id == id)
                                })
                                .cloned()
                            {
                                app.title.update(cx, |input, cx| {
                                    input.set_value(project.title, window, cx)
                                });
                                app.workspace.update(cx, |input, cx| {
                                    input.set_value(project.workspace_root, window, cx)
                                });
                            }
                            cx.notify();
                        })),
                )
            })
            .child(field("Project title", Input::new(&self.title).small(), cx))
            .when(self.directory_browser.adding, |view| {
                view.child(field(
                    "Project directory",
                    Input::new(&self.workspace).small(),
                    cx,
                ))
                .child(
                    div().flex().items_center().gap_2().child(
                        Button::new("project-browse")
                            .debug_selector(|| "project-browse".into())
                            .label("Browse…")
                            .tooltip("Browse folders on the connected environment")
                            .secondary()
                            .small()
                            .disabled(self.directory_browser.loading || !self.connection_ready())
                            .on_click(
                                cx.listener(|app, _, _, cx| app.browse_project_directory(None, cx)),
                            ),
                    ),
                )
            })
            .when(!self.directory_browser.adding, |view| {
                view.when_some(
                    self.selected_project.as_ref().and_then(|id| {
                        self.shell.projects.iter().find(|project| &project.id == id)
                    }),
                    |view, project| {
                        view.child(
                            div()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child(project.workspace_root.clone()),
                        )
                    },
                )
            })
            .when(current_browser && self.directory_browser.loading, |view| {
                view.child("Loading directories…")
            })
            .when_some(
                self.directory_browser
                    .error
                    .clone()
                    .filter(|_| current_browser),
                |view, error| {
                    view.child(div().text_xs().text_color(cx.theme().danger).child(error))
                },
            )
            .when_some(
                self.directory_browser
                    .parent
                    .clone()
                    .filter(|_| current_browser),
                |view, parent| {
                    let select_path = parent.clone();
                    let up = crate::settings_projects::parent_directory(&parent);
                    view.child(div().text_xs().child(parent)).child(
                        div()
                            .flex()
                            .flex_wrap()
                            .gap_2()
                            .when_some(up, |view, up| {
                                view.child(
                                    Button::new("project-browse-up")
                                        .label("Up")
                                        .ghost()
                                        .small()
                                        .on_click(cx.listener(move |app, _, _, cx| {
                                            app.browse_project_directory(Some(up.clone()), cx)
                                        })),
                                )
                            })
                            .child(
                                Button::new("project-use-folder")
                                    .debug_selector(|| "project-use-folder".into())
                                    .label("Use this folder")
                                    .secondary()
                                    .small()
                                    .on_click(cx.listener(move |app, _, window, cx| {
                                        app.workspace.update(cx, |input, cx| {
                                            input.set_value(select_path.clone(), window, cx)
                                        });
                                        if app.title.read(cx).value().trim().is_empty() {
                                            let title = select_path
                                                .trim_end_matches(['/', '\\'])
                                                .rsplit(['/', '\\'])
                                                .next()
                                                .unwrap_or("Project");
                                            app.title.update(cx, |input, cx| {
                                                input.set_value(title, window, cx)
                                            });
                                        }
                                        cx.notify();
                                    })),
                            ),
                    )
                },
            )
            .child(
                div()
                    .id("project-directories")
                    .max_h_48()
                    .overflow_y_scroll()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .children(
                        self.directory_browser
                            .entries
                            .iter()
                            .filter(|_| current_browser)
                            .enumerate()
                            .map(|(index, entry)| {
                                let path =
                                    entry["fullPath"].as_str().unwrap_or_default().to_owned();
                                Button::new(("project-directory", index))
                                    .label(entry["name"].as_str().unwrap_or("Directory").to_owned())
                                    .ghost()
                                    .small()
                                    .on_click(cx.listener(move |app, _, _, cx| {
                                        app.browse_project_directory(Some(path.clone()), cx)
                                    }))
                            }),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap_2()
                    .items_center()
                    .when(self.directory_browser.adding, |view| {
                        view.child(
                            Button::new("add-project")
                                .debug_selector(|| "add-project".into())
                                .label("Add project")
                                .small()
                                .disabled((self.pending_actions > 0) || !self.connection_ready())
                                .primary()
                                .on_click(
                                    cx.listener(|app, _, _, cx| app.create_project_from_fields(cx)),
                                ),
                        )
                    })
                    .when(!self.directory_browser.adding, |view| {
                        view.child(
                            Button::new("new-thread")
                                .debug_selector(|| "new-thread".into())
                                .small()
                                .label("New thread")
                                .secondary()
                                .on_click(cx.listener(|app, _, window, cx| {
                                    app.execute_command(palette::Command::NewThread, window, cx)
                                })),
                        )
                        .child(
                            Button::new("rename-project")
                                .small()
                                .disabled(self.pending_actions > 0 || !self.connection_ready())
                                .debug_selector(|| "rename-project".into())
                                .label("Rename")
                                .ghost()
                                .on_click(
                                    cx.listener(|app, _, _, cx| app.update_project(false, cx)),
                                ),
                        )
                    }),
            )
            .when(!self.directory_browser.adding, |view| {
                view.child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .gap_2()
                        .child(
                            Button::new("project-start-add")
                                .debug_selector(|| "project-start-add".into())
                                .label("Add project…")
                                .ghost()
                                .small()
                                .disabled(self.pending_actions > 0 || !self.connection_ready())
                                .on_click(cx.listener(|app, _, window, cx| {
                                    app.open_add_project(window, cx)
                                })),
                        )
                        .child(
                            Button::new("remove-project")
                                .small()
                                .disabled(self.pending_actions > 0 || !self.connection_ready())
                                .debug_selector(|| "remove-project".into())
                                .label(
                                    if self.confirm_project_removal == self.selected_project
                                        && self.selected_project.is_some()
                                    {
                                        "Confirm removal"
                                    } else {
                                        "Remove project"
                                    },
                                )
                                .danger()
                                .outline()
                                .on_click(
                                    cx.listener(|app, _, _, cx| app.update_project(true, cx)),
                                ),
                        ),
                )
            })
    }
}
