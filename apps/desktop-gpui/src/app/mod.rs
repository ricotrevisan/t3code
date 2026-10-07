//! `NativeApp` behaviour, one `impl NativeApp` block per job. The struct, its
//! constructor, actions, key bindings, and the `Render` impl stay in `main.rs`.

mod composer;
mod connection;
pub(crate) mod diffs;
mod files;
mod models;
mod notifications;
mod overlays;
mod projects;
mod threads;
mod worktrees;
