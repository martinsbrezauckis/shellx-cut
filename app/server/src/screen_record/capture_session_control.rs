//! Private lifecycle ownership for one in-process screen capture.
//!
//! The coordinator is deliberately observation-only in this slice: it records
//! preparation, backend start, and terminal stop without exposing pause/resume
//! controls or claiming that any selected stream can be sealed independently.

use record_capture::{CaptureClock, PauseStreamCoordinator, SelectedCaptureStreams, SessionPhase};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Read-only internal lifecycle facts for server tests and future wiring.
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(dead_code)] // The status seam is deliberately private until server wiring consumes it.
pub(crate) struct CaptureSessionStatus {
    pub(crate) phase: SessionPhase,
    pub(crate) selected_streams: SelectedCaptureStreams,
    pub(crate) stop_requested: bool,
}

#[derive(Debug)]
struct CaptureSessionState {
    stop: Arc<AtomicBool>,
    coordinator: Mutex<PauseStreamCoordinator>,
}

/// One accepted screen-record request and its private physical stop signal.
///
/// The state begins in `Preparing`. Only the shared backend clock may advance it
/// to `Recording`; every terminal path transitions it to `Stopped` before waking
/// native workers through `stop`.
#[derive(Debug, Clone)]
pub(crate) struct CaptureSessionControl {
    state: Arc<CaptureSessionState>,
}

/// Keeps a worker-owned control terminal if its capture path unwinds early.
pub(crate) struct CaptureSessionTerminalGuard(CaptureSessionControl);

impl Drop for CaptureSessionTerminalGuard {
    fn drop(&mut self) {
        self.0.terminalize();
    }
}

impl CaptureSessionControl {
    pub(crate) fn new(
        duration_ms: Option<u64>,
        audio: bool,
        system_audio: bool,
        passive_input_capture_active: bool,
    ) -> Self {
        // Cursor, click, and scroll observation make `InputEvents` a selected
        // stream independently of the key-content permission carried only by
        // `CaptureConfig::capture_keys`. This control must never use key
        // permission to erase passive input from its immutable registration.
        let streams =
            SelectedCaptureStreams::new(audio, system_audio, false, passive_input_capture_active);
        let coordinator =
            PauseStreamCoordinator::new(streams, duration_ms.map(Duration::from_millis));
        Self {
            state: Arc::new(CaptureSessionState {
                stop: Arc::new(AtomicBool::new(false)),
                coordinator: Mutex::new(coordinator),
            }),
        }
    }

    /// Start one observer for the capture clock that native backend setup owns.
    ///
    /// The observer only reacts to an actual clock origin. A stop while setup is
    /// pending wakes it through the same signal and leaves the control terminal.
    pub(crate) fn observe_backend_start(&self, clock: CaptureClock) -> std::io::Result<()> {
        let control = self.clone();
        std::thread::Builder::new()
            .name("cut-capture-clock-observer".into())
            .spawn(move || {
                let stop = control.stop_signal();
                if let Some(started_at) = clock.wait_started(stop.as_ref()) {
                    control.backend_started_at(started_at);
                }
            })
            .map(|_| ())
    }

    /// Terminalize the lifecycle before signaling physical backend shutdown.
    ///
    /// `PauseStreamCoordinator::stop_at` is idempotent and terminal. Recovering
    /// a poisoned lock preserves its actual coordinator value so neither a failed
    /// observer nor a prior panic can prevent a later stop from taking effect.
    pub(crate) fn terminalize(&self) {
        let mut coordinator = self.coordinator();
        let _ = coordinator.stop_at(Instant::now());
        self.state.stop.store(true, Ordering::Relaxed);
    }

    /// Ensure unexpected worker unwinding cannot release a nonterminal session.
    pub(crate) fn terminal_guard(&self) -> CaptureSessionTerminalGuard {
        CaptureSessionTerminalGuard(self.clone())
    }

    pub(crate) fn stop_signal(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.state.stop)
    }

    #[allow(dead_code)] // Read-only internal status for tests and later server wiring.
    pub(crate) fn status(&self) -> CaptureSessionStatus {
        let coordinator = self.coordinator();
        CaptureSessionStatus {
            phase: coordinator.phase(),
            selected_streams: coordinator.selected_streams().clone(),
            stop_requested: self.state.stop.load(Ordering::Relaxed),
        }
    }

    fn backend_started_at(&self, started_at: Instant) {
        let mut coordinator = self.coordinator();
        if self.state.stop.load(Ordering::Relaxed) || coordinator.phase().is_terminal() {
            return;
        }
        let _ = coordinator.start_at(started_at);
    }

    fn coordinator(&self) -> std::sync::MutexGuard<'_, PauseStreamCoordinator> {
        self.state
            .coordinator
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

#[cfg(test)]
mod tests {
    use super::CaptureSessionControl;
    use record_capture::{CaptureClock, RecordingStream, SessionPhase};
    use std::time::{Duration, Instant};

    fn wait_for_phase(control: &CaptureSessionControl, expected: SessionPhase) {
        let deadline = Instant::now() + Duration::from_secs(1);
        while control.status().phase != expected {
            assert!(
                Instant::now() < deadline,
                "lifecycle did not reach {expected:?}"
            );
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    #[test]
    fn capture_session_control_waits_for_the_shared_backend_clock() {
        let control = CaptureSessionControl::new(Some(1_000), true, true, true);
        let clock = CaptureClock::new();
        control.observe_backend_start(clock.clone()).unwrap();

        let preparing = control.status();
        assert_eq!(preparing.phase, SessionPhase::Preparing);
        assert_eq!(
            preparing.selected_streams.streams(),
            &[
                RecordingStream::ScreenVideo,
                RecordingStream::MicrophoneAudio,
                RecordingStream::SystemAudio,
                RecordingStream::InputEvents,
            ]
        );

        clock.start();
        wait_for_phase(&control, SessionPhase::Recording);
    }

    #[test]
    fn capture_session_control_selects_the_explicit_passive_input_stream() {
        let control = CaptureSessionControl::new(None, false, false, true);

        assert_eq!(
            control.status().selected_streams.streams(),
            &[RecordingStream::ScreenVideo, RecordingStream::InputEvents]
        );
    }

    #[test]
    fn capture_session_control_does_not_claim_input_when_passive_capture_is_disabled() {
        let control = CaptureSessionControl::new(None, false, false, false);

        assert_eq!(
            control.status().selected_streams.streams(),
            &[RecordingStream::ScreenVideo]
        );
    }

    #[test]
    fn capture_session_control_stop_is_terminal_and_idempotent_before_late_start() {
        let control = CaptureSessionControl::new(None, false, false, false);
        let clock = CaptureClock::new();
        control.observe_backend_start(clock.clone()).unwrap();

        control.terminalize();
        control.terminalize();
        clock.start();
        std::thread::sleep(Duration::from_millis(30));

        let status = control.status();
        assert_eq!(status.phase, SessionPhase::Stopped);
        assert!(status.stop_requested);
        assert_eq!(
            status.selected_streams.streams(),
            &[RecordingStream::ScreenVideo]
        );
    }

    #[test]
    fn capture_session_control_terminalizes_after_recording_for_every_completion_path() {
        let control = CaptureSessionControl::new(None, false, false, false);
        let clock = CaptureClock::new();
        control.observe_backend_start(clock.clone()).unwrap();
        clock.start();
        wait_for_phase(&control, SessionPhase::Recording);

        // The worker invokes this same terminal operation after either a normal
        // backend return or a backend error, before finalization continues.
        control.terminalize();
        assert_eq!(control.status().phase, SessionPhase::Stopped);
        assert!(control.status().stop_requested);
    }

    #[test]
    fn capture_session_control_stop_survives_a_poisoned_lifecycle_lock() {
        let control = CaptureSessionControl::new(None, false, false, false);
        let state = control.state.clone();
        let _ = std::panic::catch_unwind(move || {
            let _coordinator = state.coordinator.lock().unwrap();
            panic!("deliberately poison the lifecycle mutex");
        });

        control.terminalize();
        let status = control.status();
        assert_eq!(status.phase, SessionPhase::Stopped);
        assert!(status.stop_requested);
    }
}
