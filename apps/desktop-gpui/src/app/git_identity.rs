use crate::NativeApp;
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::{AppContext as _, Context, Entity, Subscription, Window};
#[derive(Default)]
pub(crate) struct HeaderRename {
    pub input: Option<Entity<InputState>>,
    pub view: std::cell::OnceCell<Entity<crate::views::thread_header::HeaderView>>,
    pub overflow_open: bool,
    pub overflow_previous_focus: Option<gpui_kit::FocusHandle>,
    pub menu_position: Option<gpui_kit::Point<gpui_kit::Pixels>>,
    pub overflow_focus: std::cell::OnceCell<gpui_kit::FocusHandle>,
    pub overflow_bounds: std::rc::Rc<std::cell::Cell<Option<gpui_kit::Bounds<gpui_kit::Pixels>>>>,
    target: Option<(String, String, String)>,
    subscription: Option<Subscription>,
}
impl NativeApp {
    pub(crate) fn open_header_project_settings(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(project) = self
            .shell
            .projects
            .iter()
            .find(|p| Some(&p.id) == self.selected_project.as_ref())
            .cloned()
        else {
            return;
        };
        self.close_thread_menu();
        self.sidebar_state.rename_target = None;
        self.sidebar_state.project_settings_target =
            Some((self.environment_scope.clone(), project.id));
        self.sidebar_rename
            .update(cx, |input, cx| input.set_value(project.title, window, cx));
        cx.notify();
    }
    pub(crate) fn begin_header_rename(
        &mut self,
        original: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(id) = self.selected_thread.clone() else {
            return;
        };
        let input =
            cx.new(|cx| crate::single_line_input(window, cx).default_value(original.clone()));
        let subscription = cx.subscribe_in(&input, window, |app, _, event, window, cx| {
            if matches!(event, InputEvent::PressEnter { .. } | InputEvent::Blur) {
                app.finish_header_rename(true, window, cx);
            }
        });
        self.header_rename = HeaderRename {
            input: Some(input.clone()),
            target: Some((self.environment_scope.clone(), id, original)),
            subscription: Some(subscription),
            view: self.header_rename.view.clone(),
            overflow_open: self.header_rename.overflow_open,
            overflow_previous_focus: self.header_rename.overflow_previous_focus.clone(),
            menu_position: self.header_rename.menu_position,
            overflow_focus: self.header_rename.overflow_focus.clone(),
            overflow_bounds: self.header_rename.overflow_bounds.clone(),
        };
        input.update(cx, |input, cx| input.focus(window, cx));
        cx.notify();
    }
    pub(crate) fn finish_header_rename(
        &mut self,
        save: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((environment, id, original)) = self.header_rename.target.take() else {
            return;
        };
        let value = self
            .header_rename
            .input
            .take()
            .map(|input| input.read(cx).value().trim().to_owned())
            .unwrap_or_default();
        self.header_rename.subscription = None;
        if save && value.is_empty() {
            self.report_error("Thread title cannot be empty", cx);
            return;
        }
        if save
            && !value.is_empty()
            && value != original
            && self.environment_scope == environment
            && self.selected_thread.as_ref() == Some(&id)
        {
            self.title
                .update(cx, |input, cx| input.set_value(value, window, cx));
            self.thread_lifecycle(&id, "thread.meta.update", cx);
        }
        cx.notify();
    }
}
