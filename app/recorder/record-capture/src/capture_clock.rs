//! One capture-wide elapsed clock shared by screen, input, mic, and system audio.
//!
//! Portal selection and encoder setup may take significant time. Audio must not
//! begin on a speculative pre-portal clock: backends open this gate only when the
//! user-approved capture session is ready, and every checkpoint/event timestamp
//! is measured from that same instant.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Default)]
pub struct CaptureClock {
    state: Arc<(Mutex<Option<Instant>>, Condvar)>,
}

impl CaptureClock {
    pub fn new() -> Self {
        Self::default()
    }

    /// Open the clock once after the backend has obtained its real capture
    /// session. Repeated calls keep the original origin.
    pub fn start(&self) -> Instant {
        let (lock, wake) = &*self.state;
        let mut started = lock
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let origin = *started.get_or_insert_with(Instant::now);
        wake.notify_all();
        origin
    }

    /// Wait for the backend-owned origin without holding up a failed/stopped
    /// capture. The wait is interruptible so a permission/portal error cannot
    /// strand an audio worker.
    pub fn wait_started(&self, stop: &AtomicBool) -> Option<Instant> {
        let (lock, wake) = &*self.state;
        let mut started = lock
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        loop {
            if let Some(origin) = *started {
                return Some(origin);
            }
            if stop.load(Ordering::Relaxed) {
                return None;
            }
            let (next, _) = wake
                .wait_timeout(started, Duration::from_millis(25))
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            started = next;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::CaptureClock;
    use std::sync::atomic::AtomicBool;
    use std::sync::mpsc;
    use std::sync::Arc;
    use std::time::Duration;

    #[test]
    fn sidecar_waits_for_backend_owned_origin() {
        let clock = CaptureClock::new();
        let stop = Arc::new(AtomicBool::new(false));
        let (sent, received) = mpsc::channel();
        let waiter_clock = clock.clone();
        let waiter_stop = stop.clone();
        let waiter = std::thread::spawn(move || {
            sent.send(waiter_clock.wait_started(&waiter_stop)).unwrap();
        });
        assert!(received.recv_timeout(Duration::from_millis(20)).is_err());
        let origin = clock.start();
        assert_eq!(
            received.recv_timeout(Duration::from_secs(1)).unwrap(),
            Some(origin)
        );
        waiter.join().unwrap();
    }
}
