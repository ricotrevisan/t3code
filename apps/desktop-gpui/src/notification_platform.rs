//! Receipt-bearing delivery boundary. GPUI's registered UN center delegate still
//! owns clicks; unlike GPUI's void show API, we await authorization and acceptance.
use crate::notifications::{Event, NotificationMode};
use std::{
    cell::Cell,
    collections::{HashSet, VecDeque},
    rc::Rc,
};

/// Accepted means the OS accepted the request, not that a banner was seen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeliveryOutcome {
    Accepted,
    Denied,
    Unavailable,
    AuthorizationError,
    DeliveryError,
    Invalidated,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Authorization {
    #[default]
    Unknown,
    NotDetermined,
    Denied,
    Authorized,
    Unavailable,
}

pub struct Delivery {
    pub tag: String,
    generation: u64,
    current: Rc<Cell<u64>>,
    recording: Option<async_channel::Receiver<DeliveryOutcome>>,
}
impl Delivery {
    pub fn is_current(&self) -> bool {
        self.current.get() == self.generation
    }
    pub async fn send(&self, title: String, body: String) -> DeliveryOutcome {
        if !self.is_current() {
            return DeliveryOutcome::Invalidated;
        }
        if let Some(receiver) = &self.recording {
            let outcome = receiver
                .recv()
                .await
                .unwrap_or(DeliveryOutcome::DeliveryError);
            return if self.is_current() {
                outcome
            } else {
                DeliveryOutcome::Invalidated
            };
        }
        #[cfg(target_os = "macos")]
        {
            use block2::RcBlock;
            use objc2::runtime::Bool;
            use objc2_foundation::{NSBundle, NSError, NSString};
            use objc2_user_notifications::{
                UNAuthorizationOptions, UNMutableNotificationContent, UNNotificationRequest,
                UNUserNotificationCenter,
            };
            let bundle = NSBundle::mainBundle();
            // The center aborts outside an app bundle. Identifier alone can be
            // provided by other bundle types, so check the actual bundle path too.
            if bundle.bundleIdentifier().is_none()
                || !bundle.bundlePath().to_string().ends_with(".app")
            {
                return DeliveryOutcome::Unavailable;
            }
            let center = UNUserNotificationCenter::currentNotificationCenter();
            let (sender, receiver) = async_channel::bounded(1);
            let authorization = RcBlock::new(move |granted: Bool, error: *mut NSError| {
                let outcome = if !error.is_null() {
                    DeliveryOutcome::AuthorizationError
                } else if granted.as_bool() {
                    DeliveryOutcome::Accepted
                } else {
                    DeliveryOutcome::Denied
                };
                let _ = sender.try_send(outcome);
            });
            center.requestAuthorizationWithOptions_completionHandler(
                UNAuthorizationOptions::Alert,
                &authorization,
            );
            let authorized = receiver
                .recv()
                .await
                .unwrap_or(DeliveryOutcome::AuthorizationError);
            if !self.is_current() {
                return DeliveryOutcome::Invalidated;
            }
            if authorized != DeliveryOutcome::Accepted {
                return authorized;
            }
            let content = UNMutableNotificationContent::new();
            content.setTitle(&NSString::from_str(&title));
            content.setBody(&NSString::from_str(&body));
            let request = UNNotificationRequest::requestWithIdentifier_content_trigger(
                &NSString::from_str(&self.tag),
                &content,
                None,
            );
            let (sender, receiver) = async_channel::bounded(1);
            let completion = RcBlock::new(move |error: *mut NSError| {
                let _ = sender.try_send(if error.is_null() {
                    DeliveryOutcome::Accepted
                } else {
                    DeliveryOutcome::DeliveryError
                });
            });
            center.addNotificationRequest_withCompletionHandler(&request, Some(&completion));
            let outcome = receiver
                .recv()
                .await
                .unwrap_or(DeliveryOutcome::DeliveryError);
            if !self.is_current() {
                // Unique receipt identifiers cannot dismiss a newer generation's alert.
                use objc2_foundation::NSArray;
                let identifiers = NSArray::from_retained_slice(&[NSString::from_str(&self.tag)]);
                center.removePendingNotificationRequestsWithIdentifiers(&identifiers);
                center.removeDeliveredNotificationsWithIdentifiers(&identifiers);
                return DeliveryOutcome::Invalidated;
            }
            outcome
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = (title, body);
            DeliveryOutcome::Unavailable
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sound {
    Completion,
    Input,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Call {
    Sound(Sound),
    Badge(usize),
    Delivery(String),
    Receipt(String, DeliveryOutcome),
}

pub struct Platform {
    recording: bool,
    pub calls: Vec<Call>,
    generation: Rc<Cell<u64>>,
    sequence: u64,
    pending: HashSet<String>,
    outcomes: VecDeque<async_channel::Receiver<DeliveryOutcome>>,
    last_outcome: Option<DeliveryOutcome>,
    authorization: Authorization,
    authorization_epoch: u64,
}
impl Platform {
    /// Recording mode never invokes AppKit, including in headless UI tests.
    pub fn new(recording: bool) -> Self {
        Self {
            recording,
            calls: Vec::new(),
            generation: Rc::new(Cell::new(0)),
            sequence: 0,
            pending: HashSet::new(),
            outcomes: VecDeque::new(),
            last_outcome: None,
            authorization: Authorization::Unknown,
            authorization_epoch: 0,
        }
    }
    /// Read-only permission probe: never requests authorization or sends a banner.
    pub fn probe_authorization(&mut self) -> async_channel::Receiver<Authorization> {
        self.authorization_epoch += 1;
        let (sender, receiver) = async_channel::bounded(1);
        if self.recording {
            let _ = sender.try_send(self.authorization);
            return receiver;
        }
        #[cfg(target_os = "macos")]
        {
            use block2::RcBlock;
            use objc2_foundation::NSBundle;
            use objc2_user_notifications::{
                UNAuthorizationStatus, UNNotificationSettings, UNUserNotificationCenter,
            };
            let bundle = NSBundle::mainBundle();
            if bundle.bundleIdentifier().is_none()
                || !bundle.bundlePath().to_string().ends_with(".app")
            {
                let _ = sender.try_send(Authorization::Unavailable);
            } else {
                let completion =
                    RcBlock::new(move |settings: std::ptr::NonNull<UNNotificationSettings>| {
                        let status = unsafe { settings.as_ref() }.authorizationStatus();
                        let authorization = if status == UNAuthorizationStatus::NotDetermined {
                            Authorization::NotDetermined
                        } else if status == UNAuthorizationStatus::Denied {
                            Authorization::Denied
                        } else if status == UNAuthorizationStatus::Authorized
                            || status == UNAuthorizationStatus::Provisional
                            || status == UNAuthorizationStatus::Ephemeral
                        {
                            Authorization::Authorized
                        } else {
                            Authorization::Unknown
                        };
                        let _ = sender.try_send(authorization);
                    });
                UNUserNotificationCenter::currentNotificationCenter()
                    .getNotificationSettingsWithCompletionHandler(&completion);
            }
        }
        #[cfg(not(target_os = "macos"))]
        let _ = sender.try_send(Authorization::Unavailable);
        receiver
    }

    pub fn authorization_epoch(&self) -> u64 {
        self.authorization_epoch
    }

    pub fn delivery_epoch(&self) -> u64 {
        self.sequence
    }

    pub fn record_authorization(&mut self, authorization: Authorization) {
        self.authorization = authorization;
        // A current successful probe supersedes permission-related delivery failures.
        // Keep real delivery failures as historical feedback, never as current denial.
        if authorization == Authorization::Authorized
            && matches!(
                self.last_outcome,
                Some(
                    DeliveryOutcome::Denied
                        | DeliveryOutcome::Unavailable
                        | DeliveryOutcome::AuthorizationError
                )
            )
        {
            self.last_outcome = None;
        }
    }

    pub fn state_text(&self) -> &'static str {
        match self.authorization {
            Authorization::Denied => {
                return "Notification authorization denied · enable it in system settings";
            }
            Authorization::NotDetermined => {
                return "Notification authorization not determined · requested when a background event needs delivery";
            }
            Authorization::Unavailable => {
                return "Desktop notifications unavailable · a bundled app on a supported platform is required";
            }
            _ => {}
        }
        match self.last_outcome {
            Some(DeliveryOutcome::Accepted) => {
                "OS accepted the last notification request (banner delivery is not guaranteed)"
            }
            Some(DeliveryOutcome::Denied) => {
                "Notification authorization denied · enable it in system settings"
            }
            Some(DeliveryOutcome::Unavailable) => {
                "Desktop notifications unavailable · a bundled app is required"
            }
            Some(DeliveryOutcome::AuthorizationError) => {
                "Could not determine notification authorization"
            }
            Some(DeliveryOutcome::DeliveryError) => "Last notification delivery failed",
            _ if !self.recording && !Self::supported() => {
                "Desktop notifications unsupported on this platform"
            }
            _ if self.authorization == Authorization::Authorized => {
                "Notification authorization granted · no delivery receipt yet"
            }
            _ => "Notification authorization not yet checked",
        }
    }

    pub fn supported() -> bool {
        cfg!(target_os = "macos")
    }
    pub fn effect_mode(&self, mode: NotificationMode) -> NotificationMode {
        if self.recording || Self::supported() {
            mode
        } else {
            NotificationMode::Off
        }
    }
    /// Explicit test receipts; recording never invents acceptance.
    pub fn queue_delivery(&mut self) -> async_channel::Sender<DeliveryOutcome> {
        assert!(self.recording);
        let (sender, receiver) = async_channel::bounded(1);
        self.outcomes.push_back(receiver);
        sender
    }
    pub fn begin_delivery(&mut self, environment: &str, thread: &str) -> Delivery {
        self.sequence += 1;
        let generation = self.generation.get();
        let tag = serde_json::to_string(&(environment, thread, generation, self.sequence))
            .expect("notification identifier");
        self.pending.insert(tag.clone());
        let recording = if self.recording {
            self.calls.push(Call::Delivery(tag.clone()));
            Some(self.outcomes.pop_front().unwrap_or_else(|| {
                let (sender, receiver) = async_channel::bounded(1);
                sender
                    .try_send(DeliveryOutcome::Unavailable)
                    .expect("empty channel");
                receiver
            }))
        } else {
            None
        };
        Delivery {
            tag,
            generation,
            current: self.generation.clone(),
            recording,
        }
    }
    pub fn record_receipt(&mut self, tag: &str, outcome: DeliveryOutcome) {
        if outcome != DeliveryOutcome::Invalidated && self.pending.contains(tag) {
            self.last_outcome = Some(outcome);
            match outcome {
                DeliveryOutcome::Accepted | DeliveryOutcome::DeliveryError => {
                    self.authorization = Authorization::Authorized
                }
                DeliveryOutcome::Denied => self.authorization = Authorization::Denied,
                DeliveryOutcome::Unavailable => self.authorization = Authorization::Unavailable,
                DeliveryOutcome::AuthorizationError => self.authorization = Authorization::Unknown,
                _ => {}
            }
        }
        if self.recording {
            self.calls.push(Call::Receipt(tag.into(), outcome));
        }
        if outcome != DeliveryOutcome::Accepted {
            self.pending.remove(tag);
        }
    }
    pub fn invalidate(&mut self) -> impl Iterator<Item = String> + '_ {
        self.generation.set(self.generation.get() + 1);
        self.pending.drain()
    }
    /// Check current preferences at playback readiness, not when work was queued.
    /// NSSound loads synchronously, so no asynchronous decode can outlive this check.
    pub fn play(&mut self, event: Event, current_mode: NotificationMode) {
        if !current_mode.sound() {
            return;
        }
        let sound = if event == Event::TurnCompleted {
            Sound::Completion
        } else {
            Sound::Input
        };
        if self.recording {
            self.calls.push(Call::Sound(sound));
            return;
        }
        #[cfg(target_os = "macos")]
        {
            use objc2_app_kit::NSSound;
            use objc2_foundation::NSString;
            let name = NSString::from_str(match sound {
                Sound::Completion => "Glass",
                Sound::Input => "Ping",
            });
            if let Some(sound) = NSSound::soundNamed(&name) {
                sound.play();
            }
        }
    }
    pub fn badge(&mut self, count: usize) {
        if self.recording {
            self.calls.push(Call::Badge(count));
            return;
        }
        #[cfg(target_os = "macos")]
        {
            use objc2::MainThreadMarker;
            use objc2_app_kit::NSApplication;
            use objc2_foundation::NSString;
            let mtm =
                MainThreadMarker::new().expect("notification effects run on GPUI main thread");
            let label = (count > 0).then(|| NSString::from_str(&count.to_string()));
            NSApplication::sharedApplication(mtm)
                .dockTile()
                .setBadgeLabel(label.as_deref());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fixa_authorization_probe_supersedes_stale_denial_and_unavailability() {
        for outcome in [
            DeliveryOutcome::Denied,
            DeliveryOutcome::Unavailable,
            DeliveryOutcome::AuthorizationError,
        ] {
            let mut platform = Platform::new(true);
            let delivery = platform.begin_delivery("env", "thread");
            platform.record_receipt(&delivery.tag, outcome);
            platform.record_authorization(Authorization::Authorized);
            assert_eq!(
                platform.state_text(),
                "Notification authorization granted · no delivery receipt yet"
            );
            assert_eq!(platform.delivery_epoch(), 1);
        }
        let mut platform = Platform::new(true);
        let delivery = platform.begin_delivery("env", "thread");
        platform.record_receipt(&delivery.tag, DeliveryOutcome::DeliveryError);
        platform.record_authorization(Authorization::Authorized);
        assert_eq!(platform.state_text(), "Last notification delivery failed");
    }

    #[test]
    fn imported_modes_cannot_enable_unsupported_effects() {
        let platform = Platform::new(false);
        for mode in NotificationMode::ALL {
            assert_eq!(
                platform.effect_mode(mode),
                if Platform::supported() {
                    mode
                } else {
                    NotificationMode::Off
                }
            );
        }
    }

    #[test]
    fn readiness_checks_mode_and_distinguishes_tones_and_badge_calls() {
        let mut platform = Platform::new(true);
        platform.play(Event::TurnCompleted, NotificationMode::Off);
        platform.play(Event::TurnCompleted, NotificationMode::Sound);
        platform.play(Event::ThreadFailed, NotificationMode::NotificationsAndSound);
        platform.badge(2);
        platform.badge(0);
        assert_eq!(
            platform.calls,
            [
                Call::Sound(Sound::Completion),
                Call::Sound(Sound::Input),
                Call::Badge(2),
                Call::Badge(0)
            ]
        );
    }
}
