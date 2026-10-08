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
