//! Native shell integration, kept separate from the conversation renderer.
pub(crate) mod input_trace;
pub(crate) mod keystrokes;
#[cfg(target_os = "macos")]
pub(crate) mod visible_frame;
use crate::NativeApp;
use gpui_kit::{
    App, Bounds, Context, Div, KeyBinding, Menu, MenuItem, OsAction, PromptLevel, SystemMenuType,
    WindowBounds, actions, point, prelude::*, px, size,
};

actions!(
    platform_shell,
    [
        About,
        Hide,
        HideOthers,
        ShowAll,
        Minimize,
        Maximize,
        Fullscreen,
        CloseWindow
    ]
);

pub(crate) fn bind(cx: &mut App) {
    cx.on_action(|_: &Hide, cx| cx.hide());
    cx.on_action(|_: &HideOthers, cx| cx.hide_other_apps());
    cx.on_action(|_: &ShowAll, cx| cx.unhide_other_apps());
    cx.bind_keys([
        KeyBinding::new("cmd-h", Hide, None),
        KeyBinding::new("alt-cmd-h", HideOthers, None),
        KeyBinding::new("cmd-m", Minimize, None),
        KeyBinding::new("ctrl-cmd-f", Fullscreen, None),
        KeyBinding::new("cmd-w", CloseWindow, None),
    ]);
}

pub(crate) fn application_items() -> Vec<MenuItem> {
    vec![
        MenuItem::action("About T3 Code Native", About),
        MenuItem::action("Settings…", crate::OpenSettings),
        MenuItem::separator(),
        MenuItem::os_submenu("Services", SystemMenuType::Services),
        MenuItem::separator(),
        MenuItem::action("Hide T3 Code Native", Hide),
        MenuItem::action("Hide Others", HideOthers),
        MenuItem::action("Show All", ShowAll),
        MenuItem::separator(),
    ]
}

pub(crate) fn edit_menu() -> Menu {
    use gpui_kit::component::input::{Copy, Cut, Paste, Redo, SelectAll, Undo};
    Menu::new("Edit").items([
        MenuItem::os_action("Undo", Undo, OsAction::Undo),
        MenuItem::os_action("Redo", Redo, OsAction::Redo),
        MenuItem::separator(),
        MenuItem::os_action("Cut", Cut, OsAction::Cut),
        MenuItem::os_action("Copy", Copy, OsAction::Copy),
        MenuItem::os_action("Paste", Paste, OsAction::Paste),
        MenuItem::os_action("Select All", SelectAll, OsAction::SelectAll),
    ])
}

pub(crate) fn window_menu() -> Menu {
    Menu::new("Window").items([
        MenuItem::action("Minimize", Minimize),
        MenuItem::action("Zoom", Maximize),
        MenuItem::action("Close Window", CloseWindow),
    ])
}

pub(crate) fn window_actions(element: Div, cx: &mut Context<NativeApp>) -> Div {
    element
        .on_action(cx.listener(|_, _: &Minimize, window, _| window.minimize_window()))
        .on_action(cx.listener(|app, _: &Maximize, window, cx| {
            if app.once_per_keystroke("Maximize", window, cx) {
                window.zoom_window();
            }
        }))
        .on_action(cx.listener(|app, _: &Fullscreen, window, cx| {
            if app.once_per_keystroke("Fullscreen", window, cx) {
                window.toggle_fullscreen();
            }
        }))
        .on_action(cx.listener(|_, _: &CloseWindow, window, _| window.remove_window()))
        .on_action(cx.listener(|app, _: &About, window, cx| {
            if !app.once_per_keystroke("About", window, cx) {
                return;
            }
            drop(window.prompt(
                PromptLevel::Info,
                "T3 Code Native",
                Some(concat!("Native GPUI client · ", env!("CARGO_PKG_VERSION"))),
                &["OK"],
                cx,
            ));
        }))
}
use t3code_desktop_gpui::window_state::WindowState;

impl NativeApp {
    pub(crate) fn shell_terminal_available(&self) -> bool {
        self.terminal_drawer.open
            || (self.connection_ready()
                && self.selected_workspace().is_some()
                && self
                    .shell
                    .threads
                    .iter()
                    .any(|thread| Some(&thread.id) == self.selected_thread.as_ref()))
    }

    pub(crate) fn render_shell_terminal_control(
        &self,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        use gpui_kit::component::{
            Disableable as _, Selectable as _, Sizable as _,
            button::{Button, ButtonVariants as _},
        };
        Button::new("shell-terminal-toggle")
            .debug_selector(|| "shell-terminal-toggle".into())
            .icon(gpui_kit::assets::IconName::SquareTerminal)
            .ghost()
            .small()
            .selected(self.terminal_drawer.open)
            .disabled(!self.shell_terminal_available())
            .tooltip(if self.shell_terminal_available() {
                "Toggle terminal drawer ⌘J"
            } else {
                "Select a synchronized thread to open its terminal"
            })
            .on_click(cx.listener(|app, _, window, cx| app.toggle_terminal(window, cx)))
    }
}

/// Retain the kit divider appearance and add the reference double-click reset.
pub(crate) fn sidebar_handle(cx: &Context<NativeApp>) -> gpui_kit::base::ResizeHandleRenderer {
    let appearance = gpui_kit::component::resizable::resize_handle_appearance();
    let app = cx.entity().downgrade();
    std::rc::Rc::new(move |handle, window, cx| {
        let child = appearance(handle, window, cx)?;
        let app = app.clone();
        Some(
            gpui_kit::div()
                .id("sidebar-divider-reset")
                .debug_selector(|| "sidebar-divider-reset".into())
                .size_full()
                .child(child)
                .capture_any_mouse_down(move |event, _, cx| {
                    if event.button == gpui_kit::MouseButton::Left && event.click_count == 2 {
                        let _ = app.update(cx, |app, cx| {
                            app.sidebar_width = t3code_desktop_gpui::window_state::SIDEBAR_WIDTH.1;
                            app.remember_layout();
                            app.reset_splits(cx);
                            cx.notify();
                        });
                        cx.stop_propagation();
                    }
                })
                .into_any_element(),
        )
    })
}

pub(crate) fn initial_bounds(
    state: WindowState,
    cx: &App,
) -> (WindowBounds, Option<gpui_kit::DisplayId>) {
    #[cfg(target_os = "macos")]
    if let Some(([x, y, w, h], display)) = visible_frame::initial(state) {
        let bounds = Bounds::new(point(px(x), px(y)), size(px(w), px(h)));
        return (
            if state.maximized {
                WindowBounds::Maximized(bounds)
            } else {
                WindowBounds::Windowed(bounds)
            },
            Some(display),
        );
    }
    let primary = cx.primary_display().map(|display| display.id());
    let mut available = cx.displays();
    available.sort_by_key(|display| Some(display.id()) != primary);
    let displays: Vec<_> = available
        .iter()
        .map(|display| {
            let bounds = display.visible_bounds();
            [
                bounds.origin.x / px(1.),
                bounds.origin.y / px(1.),
                bounds.size.width / px(1.),
                bounds.size.height / px(1.),
            ]
        })
        .collect();
    let bounds = state
        .restore_rect(&displays)
        .map(|[x, y, w, h]| Bounds::new(point(px(x), px(y)), size(px(w), px(h))))
        .unwrap_or_else(|| Bounds::centered(None, size(px(1280.), px(900.)), cx));
    (
        if state.maximized {
            WindowBounds::Maximized(bounds)
        } else {
            WindowBounds::Windowed(bounds)
        },
        None,
    )
}

pub(crate) fn remember_bounds(state: &mut WindowState, window: &gpui_kit::Window) {
    #[cfg(target_os = "macos")]
    if visible_frame::remember(state, window) {
        return;
    }
    state.maximized = window.is_maximized();
    if !state.maximized
        && let WindowBounds::Windowed(bounds) = window.window_bounds()
    {
        state.x = Some(bounds.origin.x / px(1.));
        state.y = Some(bounds.origin.y / px(1.));
        state.width = bounds.size.width / px(1.);
        state.height = bounds.size.height / px(1.);
    }
}
