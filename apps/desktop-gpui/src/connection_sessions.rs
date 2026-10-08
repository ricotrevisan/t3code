//! Multi-environment transport ownership. Selection is presentation, not cancellation.
//!
//! A stream identity survives automatic reconnect; receipt ownership does not.
//! Callers must validate `Owner` again before applying an asynchronous receipt.
//! This deliberately does not move checkout, queue or attachment ownership into
//! the transport: their existing target-specific guards remain authoritative.
use crate::{
    connection::{Command, Phase, Session, Update},
    shell::Shell,
    thread::ThreadDetail,
};
use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Owner {
    pub environment_id: String,
    pub instance: u64,
    pub generation: u64,
}

pub struct EnvironmentSession {
    owner: Owner,
    session: Session,
    receiving: bool,
    pub phase: Phase,
    pub shell: Shell,
    pub detail: ThreadDetail,
    pub selected_thread: Option<String>,
}
impl EnvironmentSession {
    pub fn owner(&self) -> Owner {
        self.owner.clone()
    }
    pub fn ready(&self) -> bool {
        self.receiving
            && !self.session.commands.is_closed()
            && self.phase.ready()
            && self.shell.synchronized
    }
}

/// Opaque stream delivery; `apply` rejects deliveries from removed/replaced sessions.
pub struct Incoming {
    environment_id: String,
    instance: u64,
    update: Result<Update, async_channel::RecvError>,
}

pub struct Event {
    pub owner: Owner,
    /// Rechecked when consumed, not captured when the socket emitted the event.
    pub selected: bool,
    pub kind: EventKind,
}
pub enum EventKind {
    Phase,
    Shell,
    Thread,
    Resynchronizing,
    Status(String),
    Error(String),
    Subscription {
        key: String,
        item: Result<serde_json::Value, String>,
    },
    Closed,
}

#[derive(Default)]
pub struct Registry {
    sessions: BTreeMap<String, EnvironmentSession>,
    selected: Option<String>,
    next_instance: u64,
    last_received: Option<String>,
}
impl Registry {
    /// Identity must come from a verified environment discovery/config receipt,
    /// never from a label or an origin. Connecting an existing id is explicit.
    pub fn insert(
        &mut self,
        environment_id: String,
        session: Session,
    ) -> Result<Owner, &'static str> {
        if environment_id.trim().is_empty() {
            return Err("Missing environment identity");
        }
        if self.sessions.contains_key(&environment_id) {
            return Err("Environment already connected");
        }
        if self.sessions.len() >= 64 {
            return Err("Too many connected environments");
        }
        self.next_instance = self
            .next_instance
            .checked_add(1)
            .expect("session identity exhausted");
        let owner = Owner {
            environment_id: environment_id.clone(),
            instance: self.next_instance,
            generation: 0,
        };
        self.sessions.insert(
            environment_id,
            EnvironmentSession {
                owner: owner.clone(),
                session,
                receiving: true,
                phase: Phase::Connecting,
                shell: Shell::default(),
                detail: ThreadDetail::default(),
                selected_thread: None,
            },
        );
        Ok(owner)
    }
    pub fn get(&self, environment_id: &str) -> Option<&EnvironmentSession> {
        self.sessions.get(environment_id)
    }
    pub fn iter(&self) -> impl Iterator<Item = (&str, &EnvironmentSession)> {
        self.sessions
            .iter()
            .map(|(id, session)| (id.as_str(), session))
    }
    pub fn selected_id(&self) -> Option<&str> {
        self.selected.as_deref()
    }
    pub fn selected(&self) -> Option<&EnvironmentSession> {
        self.selected.as_ref().and_then(|id| self.sessions.get(id))
    }
    /// No unsubscribe, no socket disposal, and no generation change on navigation.
    pub fn select(&mut self, environment_id: &str) -> Result<Owner, &'static str> {
        let owner = self
            .sessions
            .get(environment_id)
            .ok_or("Environment is not connected")?
            .owner();
        self.selected = Some(environment_id.to_owned());
        Ok(owner)
    }
    pub fn owns(&self, owner: &Owner) -> bool {
        self.sessions
            .get(&owner.environment_id)
            .is_some_and(|session| session.receiving && session.owner == *owner)
    }
    pub fn selected_owns(&self, owner: &Owner) -> bool {
        self.selected_id() == Some(owner.environment_id.as_str()) && self.owns(owner)
    }
    /// Receipt producers capture this sender AND owner. Selection alone must not
    /// redirect an in-flight operation to another environment's sender.
    pub fn commands(&self, owner: &Owner) -> Option<async_channel::Sender<Command>> {
        self.sessions
            .get(&owner.environment_id)
            .filter(|_| self.owns(owner))
            .map(|session| session.session.commands.clone())
    }
    pub fn select_thread(
        &mut self,
        owner: &Owner,
        thread_id: Option<String>,
    ) -> Result<(), String> {
        if !self.owns(owner) {
            return Err("Stale connection owner".into());
        }
        let session = self
            .sessions
            .get_mut(&owner.environment_id)
            .expect("validated owner");
        session
            .session
            .commands
            .try_send(Command::SelectThread(thread_id.clone()))
            .map_err(|error| error.to_string())?;
        session.selected_thread = thread_id;
        session.detail = ThreadDetail::default();
        Ok(())
    }
    /// Explicit removal also closes retained sender clones. A forgotten endpoint
    /// is not credential revocation; callers still own that separate operation.
    pub fn remove(&mut self, environment_id: &str) -> bool {
        let Some(session) = self.sessions.remove(environment_id) else {
            return false;
        };
        session.session.commands.close();
        session.session.updates.close();
        if self.selected_id() == Some(environment_id) {
            self.selected = None;
        }
        true
    }
    /// Drain every session, including background environments, in one UI task.
    /// No polling/timer and no unbounded intermediary queue. `None` means all
    /// streams ended (or the registry is empty); do not spin after that result.
    pub async fn next(&mut self) -> Option<Event> {
        let incoming = self.receive().await?;
        self.apply(incoming)
    }
    /// Snapshot a cancellable, owned receive future inside an entity update;
    /// await it OUTSIDE the entity borrow, then call `apply` in another update.
    /// Only one consumer may drain a registry. Rebuild/cancel this future when
    /// membership changes so newly inserted sessions join the next wait.
    pub fn receive(&self) -> impl std::future::Future<Output = Option<Incoming>> + use<> {
        let mut sessions: Vec<_> = self
            .sessions
            .values()
            .filter(|session| session.receiving)
            .collect();
        if let Some(last) = &self.last_received {
            let next = sessions.partition_point(|session| &session.owner.environment_id <= last);
            let count = sessions.len();
            if count > 0 {
                sessions.rotate_left(next % count);
            }
        }
        let pending: Vec<_> = sessions
            .into_iter()
            .map(|session| {
                let id = session.owner.environment_id.clone();
                let instance = session.owner.instance;
                let receiver = session.session.updates.clone();
                Box::pin(async move {
                    Incoming {
                        environment_id: id,
                        instance,
                        update: receiver.recv().await,
                    }
                })
            })
            .collect();
        async move {
            if pending.is_empty() {
                return None;
            }
            let (incoming, _, _) = futures_util::future::select_all(pending).await;
            Some(incoming)
        }
    }
    /// Revalidate the stream instance before applying even its phase or error.
    pub fn apply(&mut self, incoming: Incoming) -> Option<Event> {
        let id = incoming.environment_id;
        let session = self.sessions.get_mut(&id)?;
        if !session.receiving || session.owner.instance != incoming.instance {
            return None;
        }
        self.last_received = Some(id.clone());
        let kind = match incoming.update {
            Err(_) => {
                session.receiving = false;
                session.session.commands.close();
                session.owner.generation += 1;
                session.phase = Phase::Offline;
                session.shell = Shell::default();
                session.detail = ThreadDetail::default();
                EventKind::Closed
            }
            Ok(Update::Resynchronizing) => {
                session.owner.generation += 1;
                session.phase = Phase::Syncing;
                session.shell = Shell::default();
                session.detail = ThreadDetail::default();
                EventKind::Resynchronizing
            }
            Ok(Update::Phase(phase)) => {
                session.phase = phase;
                EventKind::Phase
            }
            Ok(Update::Shell(item)) => match session.shell.apply(*item) {
                Ok(()) => {
                    session.phase = if session.shell.synchronized {
                        Phase::Ready
                    } else {
                        Phase::Syncing
                    };
                    EventKind::Shell
                }
                Err(error) => Self::projection_failed(session, error.into()),
            },
            Ok(Update::Thread { thread_id, item }) => {
                if session.selected_thread.as_deref() == Some(&thread_id) {
                    match session.detail.apply(&thread_id, item) {
                        Ok(()) => EventKind::Thread,
                        Err(error) => Self::projection_failed(session, error),
                    }
                } else {
                    EventKind::Thread
                }
            }
            Ok(Update::Status(status)) => EventKind::Status(status),
            Ok(Update::Error(error)) => EventKind::Error(error),
            Ok(Update::Subscription { key, item }) => EventKind::Subscription { key, item },
        };
        Some(Event {
            owner: session.owner(),
            selected: self.selected.as_deref() == Some(&id),
            kind,
        })
    }
    fn projection_failed(session: &mut EnvironmentSession, error: String) -> EventKind {
        session.receiving = false;
        session.session.commands.close();
        session.owner.generation += 1;
        session.session.updates.close();
        session.phase = Phase::Error(error.clone());
        session.shell = Shell::default();
        session.detail = ThreadDetail::default();
        EventKind::Error(format!("Projection failed — reconnect required: {error}"))
    }
}
impl Drop for Registry {
    fn drop(&mut self) {
        for session in self.sessions.values() {
            session.session.commands.close();
            session.session.updates.close();
        }
    }
}

#[cfg(test)]
#[path = "connection_sessions_fairness_tests.rs"]
mod fairness_tests;
#[cfg(test)]
#[path = "connection_sessions_tests.rs"]
mod tests;
