mod connection;
mod protocol;
mod shell;

use gpui::{
    App, Application, Bounds, Context, Task, Window, WindowBounds, WindowOptions, div, prelude::*,
    px, rgb, size,
};
use serde::Deserialize;

#[derive(Deserialize)]
struct Baseline {
    revision: String,
    scenarios: Vec<serde_json::Value>,
}

struct NativeApp {
    baseline: Baseline,
    shell: shell::Shell,
    status: String,
    selected_thread: Option<String>,
    connection_task: Option<Task<()>>,
}

impl NativeApp {
    fn new(cx: &mut Context<Self>) -> Self {
        let mut app = Self {
            baseline: serde_json::from_str(include_str!("../parity/baseline.json"))
                .expect("checked-in parity baseline must be valid"),
            shell: shell::Shell::default(),
            status: "Not connected — configure T3_NATIVE_SERVER and T3_NATIVE_ACCESS_TOKEN".into(),
            selected_thread: None,
            connection_task: None,
        };
        app.connect(cx);
        app
    }

    fn disconnect(&mut self, cx: &mut Context<Self>) {
        self.connection_task = None;
        self.shell = shell::Shell::default();
        self.selected_thread = None;
        self.status = "Disconnected".into();
        cx.notify();
    }

    fn connect(&mut self, cx: &mut Context<Self>) {
        self.disconnect(cx);
        self.status =
            "Not connected — configure T3_NATIVE_SERVER and T3_NATIVE_ACCESS_TOKEN".into();
        // Environment input is a development bootstrap, not the final pairing UX.
        if let (Ok(base), Ok(token)) = (
            std::env::var("T3_NATIVE_SERVER"),
            std::env::var("T3_NATIVE_ACCESS_TOKEN"),
        ) {
            match connection::ConnectionOptions::new(&base, token) {
                Ok(options) => {
                    let receiver = connection::start(options);
                    self.connection_task = Some(cx.spawn(async move |entity, cx| {
                        while let Ok(update) = receiver.recv().await {
                            let keep_running = entity.update(cx, |app, cx| {
                                match update {
                                    connection::Update::Status(status) => app.status = status,
                                    connection::Update::Shell(item) => {
                                        if let Err(error) = app.shell.apply(item) {
                                            app.status =
                                                format!("Invalid shell projection: {error}");
                                            cx.notify();
                                            return false;
                                        }
                                        if app.shell.synchronized {
                                            app.status = "Connected · shell synchronized".into();
                                        }
                                        if let Some(id) = &app.selected_thread
                                            && !app
                                                .shell
                                                .threads
                                                .iter()
                                                .any(|thread| &thread.id == id)
                                        {
                                            app.selected_thread = None;
                                        }
                                    }
                                }
                                cx.notify();
                                true
                            });
                            if !matches!(keep_running, Ok(true)) {
                                break;
                            }
                        }
                    }));
                }
                Err(message) => self.status = message.into(),
            }
        }
        cx.notify();
    }
}

impl Render for NativeApp {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mut project_rows = Vec::new();
        for project in &self.shell.projects {
            let mut row = div()
                .flex()
                .flex_col()
                .gap_2()
                .child(div().text_color(rgb(0xa5b4fc)).child(project.title.clone()))
                .child(div().text_sm().child(project.workspace_root.clone()));
            for thread in
                self.shell.threads.iter().filter(|thread| {
                    thread.project_id == project.id && thread.archived_at.is_none()
                })
            {
                let id = thread.id.clone();
                row = row.child(
                    div()
                        .id(gpui::SharedString::from(format!("thread-{id}")))
                        .px_3()
                        .py_2()
                        .rounded_md()
                        .bg(rgb(if self.selected_thread.as_ref() == Some(&id) {
                            0x383d4a
                        } else {
                            0x262a33
                        }))
                        .cursor_pointer()
                        .child(thread.title.clone())
                        .on_click(cx.listener(move |app, _, _, cx| {
                            app.selected_thread = Some(id.clone());
                            cx.notify();
                        })),
                );
            }
            project_rows.push(row);
        }
        let selected = self
            .selected_thread
            .as_ref()
            .and_then(|id| self.shell.threads.iter().find(|thread| &thread.id == id));
        div().flex().size_full().bg(rgb(0x16181d)).text_color(rgb(0xe5e7eb))
            .child(div().id("project-sidebar").flex().flex_col().w(px(300.)).h_full()
                .overflow_y_scroll().p_4().gap_4().bg(rgb(0x20232b))
                .child("T3 Code · Native").child("Projects and threads").children(project_rows))
            .child(div().flex().flex_col().flex_1().p_8().gap_4()
                .child(div().text_xl().child(selected.map(|t| t.title.clone()).unwrap_or_else(|| "Rust / GPUI macOS port".into())))
                .child(self.status.clone())
                .child(div().flex().gap_3()
                    .child(div().id("connect").px_3().py_2().bg(rgb(0x383d4a)).rounded_md().cursor_pointer().child("Connect / retry")
                        .on_click(cx.listener(|app, _, _, cx| app.connect(cx))))
                    .child(div().id("disconnect").px_3().py_2().bg(rgb(0x383d4a)).rounded_md().cursor_pointer().child("Disconnect")
                        .on_click(cx.listener(|app, _, _, cx| app.disconnect(cx)))))
                .child(format!("{} projects · {} threads · sequence {}", self.shell.projects.len(), self.shell.threads.len(), self.shell.sequence.map(|s| s.to_string()).unwrap_or_else(|| "—".into())))
                .child(selected.map(|thread| format!("Selected thread: {}", thread.id)).unwrap_or_else(|| "Select a thread from the sidebar".into()))
                .child("Read-only shell slice. Conversation rendering, composer and actions are not implemented yet.")
                .child(format!("Reference fork: {}", self.baseline.revision))
                .child(format!("{} discovery scenarios — behavioral parity remains unverified", self.baseline.scenarios.len())))
    }
}

fn main() {
    Application::new().run(move |cx: &mut App| {
        let bounds = Bounds::centered(None, size(px(1100.), px(760.)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..Default::default()
            },
            |_, cx| cx.new(NativeApp::new),
        )
        .expect("could not open native window");
        cx.activate(true);
    });
}
