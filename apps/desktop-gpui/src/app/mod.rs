//! `NativeApp` behaviour, one `impl NativeApp` block per job. The struct, its
//! constructor, actions, key bindings, and the `Render` impl stay in `main.rs`.

pub(crate) mod attachments;
pub(crate) mod branch_workflows;
pub(crate) mod checkout_operations;
pub(crate) mod checkout_receipts;
pub(crate) mod checkout_recipes;
mod composer;
mod connection;
mod connection_health;
pub(crate) mod diffs;
mod files;
pub(crate) mod git_actions;
pub(crate) mod git_live;
pub(crate) mod git_header_actions;
mod git_diff_controls;
pub(crate) mod git_forms;
pub(crate) mod git_picker;
pub(crate) mod git_editors;
pub(crate) mod git_identity;
mod models;
mod notifications;
mod overlays;
pub(crate) mod project_actions;
mod projects;
pub(crate) mod queue_subscriptions;
mod threads;
mod thread_policy_bulk;
mod thread_policy_cleanup;
mod thread_policy_drop;
pub(crate) mod thread_undo;
mod worktrees;
