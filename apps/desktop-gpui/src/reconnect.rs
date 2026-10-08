//! Transport recovery never replays mutations whose server outcome is uncertain.
use std::time::Duration;

#[derive(Debug, Clone, Copy)]
pub struct Failure {
    pub message: &'static str,
    pub retryable: bool,
}

impl Failure {
    pub fn transient(message: &'static str) -> Self {
        Self {
            message,
            retryable: true,
        }
    }
    pub fn blocked(message: &'static str) -> Self {
        Self {
            message,
            retryable: false,
        }
    }
}

#[derive(Default)]
pub struct Backoff {
    attempt: u32,
}
impl Backoff {
    pub fn attempt(&self) -> u32 {
        self.attempt
    }
    pub fn reset(&mut self) {
        self.attempt = 0;
    }
    pub fn delay(&mut self) -> Duration {
        let delay =
            Duration::from_millis(250 * 2u64.pow(self.attempt.min(7))).min(Duration::from_secs(30));
        self.attempt = self.attempt.saturating_add(1);
        delay
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn retry_delay_is_bounded_and_success_resets_it() {
        let mut backoff = Backoff::default();
        assert_eq!(backoff.delay(), Duration::from_millis(250));
        assert_eq!(backoff.delay(), Duration::from_millis(500));
        for _ in 0..100 {
            assert!(backoff.delay() <= Duration::from_secs(30));
        }
        backoff.reset();
        assert_eq!(backoff.delay(), Duration::from_millis(250));
    }
}
