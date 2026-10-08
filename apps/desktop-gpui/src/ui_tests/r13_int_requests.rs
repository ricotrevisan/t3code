//! Separate automatic VCS housekeeping from feature requests without disabling
//! production synchronization. Retain every stream command for lifecycle assertions.
use super::*;
use std::cell::RefCell;
use std::collections::VecDeque;

pub(super) struct Requests {
    receiver: async_channel::Receiver<connection::Command>,
    pending: RefCell<VecDeque<connection::Command>>,
    vcs: RefCell<VecDeque<connection::Command>>,
}
impl From<async_channel::Receiver<connection::Command>> for Requests {
    fn from(receiver: async_channel::Receiver<connection::Command>) -> Self {
        Self {
            receiver,
            pending: RefCell::new(VecDeque::new()),
            vcs: RefCell::new(VecDeque::new()),
        }
    }
}
impl Requests {
    fn collect(&self) {
        while let Ok(command) = self.receiver.try_recv() {
            let vcs = match &command {
                connection::Command::Subscribe { key, tag, payload }
                    if key.starts_with("header-vcs:") =>
                {
                    assert_eq!(tag, "subscribeVcsStatus");
                    assert_eq!(payload.as_object().unwrap().len(), 1);
                    assert!(!payload["cwd"].as_str().unwrap().is_empty());
                    true
                }
                connection::Command::Unsubscribe(key) => key.starts_with("header-vcs:"),
                _ => false,
            };
            if vcs {
                self.vcs.borrow_mut().push_back(command);
            } else {
                self.pending.borrow_mut().push_back(command);
            }
        }
    }
    pub(super) fn try_recv(&self) -> Result<connection::Command, async_channel::TryRecvError> {
        self.collect();
        self.pending
            .borrow_mut()
            .pop_front()
            .ok_or(async_channel::TryRecvError::Empty)
    }
    pub(super) fn vcs(&self) -> connection::Command {
        self.collect();
        self.vcs
            .borrow_mut()
            .pop_front()
            .expect("expected automatic live VCS command")
    }
    pub(super) fn close(&self) -> bool {
        self.receiver.close()
    }
    pub(super) fn is_empty(&self) -> bool {
        self.collect();
        self.pending.borrow().is_empty()
    }
}
