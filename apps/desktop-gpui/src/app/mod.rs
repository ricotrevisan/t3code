//! `NativeApp` behaviour, one `impl NativeApp` block per job. The struct, its
//! constructor, actions, key bindings, and the `Render` impl stay in `main.rs`.

pub(crate) mod attachments;
pub(crate) mod branch_workflows;
pub(crate) mod checkout_operations;
pub(crate) mod checkout_receipts;
pub(crate) mod checkout_recipes;
mod composer;
mod connection;
pub(crate) mod diffs;
mod files;
mod models;
mod notifications;
mod overlays;
pub(crate) mod project_actions;
mod projects;
pub(crate) mod queue_subscriptions;
mod threads;
mod worktrees;
