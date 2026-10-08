//! Explicit bindings override Kit's root Tab traversal while the PTY has focus.
use gpui_kit::{Action, App, KeyBinding};
use serde::Deserialize;
#[derive(Clone, PartialEq, Eq, Deserialize, Action)]
#[action(namespace = terminal, no_json)]
pub(crate) struct TerminalKey {
    pub key: String,
    pub control: bool,
}
pub(crate) fn bind(cx: &mut App) {
    let context = Some("NativeApp > NativeTerminal");
    for key in [
        "enter",
        "backspace",
        "tab",
        "escape",
        "up",
        "down",
        "left",
        "right",
        "home",
        "end",
        "delete",
        "pageup",
        "pagedown",
    ] {
        cx.bind_keys([KeyBinding::new(
            key,
            TerminalKey {
                key: key.into(),
                control: false,
            },
            context,
        )]);
    }
    for key in b'a'..=b'z' {
        let key = char::from(key).to_string();
        cx.bind_keys([KeyBinding::new(
            &format!("ctrl-{key}"),
            TerminalKey { key, control: true },
            context,
        )]);
    }
}
