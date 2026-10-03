//! Native entry points share command identities with the chat controls.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    AddProject,
    NewThread,
    Rename,
    Archive,
    Unarchive,
    Settle,
    Unsettle,
    Snooze,
    CustomSnooze,
    Unsnooze,
    Pin,
    Unpin,
    Delete,
    Search,
    Stash,
    RestoreStash,
    Interrupt,
    Stop,
    Connect,
    Disconnect,
}

pub struct Entry {
    pub command: Command,
    pub label: &'static str,
    pub needs_thread: bool,
}
pub const ENTRIES: &[Entry] = &[
    Entry {
        command: Command::AddProject,
        label: "Add project",
        needs_thread: false,
    },
    Entry {
        command: Command::NewThread,
        label: "New thread",
        needs_thread: false,
    },
    Entry {
        command: Command::Rename,
        label: "Rename thread",
        needs_thread: true,
    },
    Entry {
        command: Command::Archive,
        label: "Archive thread",
        needs_thread: true,
    },
    Entry {
        command: Command::Unarchive,
        label: "Unarchive thread",
        needs_thread: true,
    },
    Entry {
        command: Command::Settle,
        label: "Settle thread",
        needs_thread: true,
    },
    Entry {
        command: Command::Unsettle,
        label: "Unsettle thread",
        needs_thread: true,
    },
    Entry {
        command: Command::Snooze,
        label: "Snooze thread for one hour",
        needs_thread: true,
    },
    Entry {
        command: Command::CustomSnooze,
        label: "Custom snooze thread",
        needs_thread: true,
    },
    Entry {
        command: Command::Unsnooze,
        label: "Unsnooze thread",
        needs_thread: true,
    },
    Entry {
        command: Command::Pin,
        label: "Pin thread",
        needs_thread: true,
    },
    Entry {
        command: Command::Unpin,
        label: "Unpin thread",
        needs_thread: true,
    },
    Entry {
        command: Command::Delete,
        label: "Delete thread (confirmation required)",
        needs_thread: true,
    },
    Entry {
        command: Command::Search,
        label: "Search thread history",
        needs_thread: false,
    },
    Entry {
        command: Command::Stash,
        label: "Stash prompt",
        needs_thread: true,
    },
    Entry {
        command: Command::RestoreStash,
        label: "Restore stashed prompt",
        needs_thread: true,
    },
    Entry {
        command: Command::Interrupt,
        label: "Interrupt turn",
        needs_thread: true,
    },
    Entry {
        command: Command::Stop,
        label: "Stop provider session",
        needs_thread: true,
    },
    Entry {
        command: Command::Connect,
        label: "Connect or retry",
        needs_thread: false,
    },
    Entry {
        command: Command::Disconnect,
        label: "Disconnect",
        needs_thread: false,
    },
];

pub fn matching(query: &str, has_thread: bool) -> Vec<&'static Entry> {
    let terms: Vec<_> = query.split_whitespace().map(str::to_lowercase).collect();
    ENTRIES
        .iter()
        .filter(|entry| {
            (!entry.needs_thread || has_thread)
                && terms
                    .iter()
                    .all(|term| entry.label.to_lowercase().contains(term))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn missing_thread_excludes_destructive_actions_but_navigation_remains() {
        assert!(matching("delete", false).is_empty());
        assert_eq!(
            matching("search history", false)[0].command,
            Command::Search
        );
        assert_eq!(matching("DELETE THREAD", true)[0].command, Command::Delete);
        assert_eq!(matching("unarchive", true)[0].command, Command::Unarchive);
        assert!(matching("not-a-command", true).is_empty());
    }
}
