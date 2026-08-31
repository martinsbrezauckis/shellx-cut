//! Screen-owned native camera sidecar.

use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

#[cfg(any(windows, target_os = "macos"))]
use std::sync::atomic::Ordering;
#[cfg(any(windows, target_os = "macos"))]
use std::sync::mpsc;

use record_capture::CaptureClock;
use record_core::{error_codes, CameraArtifact, CameraTerminalState, RecordError};

pub(super) struct CameraCaptureSidecar {
    #[cfg(any(windows, target_os = "macos"))]
    worker: Option<std::thread::JoinHandle<record_core::Result<Option<CameraArtifact>>>>,
    #[cfg(any(windows, target_os = "macos"))]
    terminal_tx: Option<mpsc::SyncSender<CameraTerminalState>>,
}

impl CameraCaptureSidecar {
    pub(super) fn start(
        device_id: Option<String>,
        capture_id: &str,
        capture_dir: &Path,
        clock: Option<CaptureClock>,
        stop: Arc<AtomicBool>,
    ) -> record_core::Result<Option<Self>> {
        let Some(device_id) = device_id else {
            return Ok(None);
        };
        #[cfg(any(windows, target_os = "macos"))]
        {
            let clock = clock.ok_or_else(|| {
                camera_error(
                    "camera capture has no shared screen clock",
                    "the live recorder did not supply its CaptureClock",
                )
            })?;
            let capture_dir = capture_dir.to_path_buf();
            let capture_id = capture_id.to_owned();
            let (terminal_tx, terminal_rx) = mpsc::sync_channel(1);
            let worker = std::thread::Builder::new()
                .name(format!("cut-camera-{capture_id}"))
                .spawn(move || {
                    // Native owners are deliberately constructed and retained
                    // on this thread through their one terminal Stop. Windows
                    // Media Foundation owns a per-thread COM apartment; moving
                    // a live adapter across JoinHandle would require an
                    // unsound broad `Send` promise. Only inert request values
                    // cross into this worker and the sealed artifact crosses
                    // back after the native owner has fully released.
                    let mut owner =
                        record_capture::private_camera_owner::PrivateCameraOwner::reserve(
                            &capture_dir,
                            &capture_id,
                        )?;
                    match owner.use_camera(&device_id, &clock, &stop)? {
                        record_capture::private_camera_owner::PrivateCameraUse::Started => {}
                        record_capture::private_camera_owner::PrivateCameraUse::Refused(
                            readiness,
                        ) => {
                            stop.store(true, Ordering::Release);
                            return Err(camera_refusal(readiness));
                        }
                        record_capture::private_camera_owner::PrivateCameraUse::Unavailable => {
                            stop.store(true, Ordering::Release);
                            return Err(camera_error(
                                "camera capture is unavailable in this build",
                                "the selected camera has no admitted native owner",
                            ));
                        }
                    }
                    let terminal = await_terminal_command(&terminal_rx)?;
                    owner.stop(terminal)
                })
                .map_err(|error| {
                    camera_error(
                        "could not start the camera capture worker",
                        &error.to_string(),
                    )
                })?;
            return Ok(Some(Self {
                worker: Some(worker),
                terminal_tx: Some(terminal_tx),
            }));
        }
        #[cfg(not(any(windows, target_os = "macos")))]
        {
            let _ = (device_id, capture_id, capture_dir, clock, stop);
            Err(camera_error(
                "camera capture is unavailable on this platform",
                "record without Camera or use a Windows or macOS Cut installation",
            ))
        }
    }

    pub(super) fn finish(
        self,
        terminal: CameraTerminalState,
    ) -> record_core::Result<Option<CameraArtifact>> {
        #[cfg(any(windows, target_os = "macos"))]
        {
            let mut sidecar = self;
            // Send the only terminal authority before joining. If native Start
            // has already failed, the receiver is gone; joining below returns
            // that truthful failure rather than masking it with a channel
            // error.
            if let Some(terminal_tx) = sidecar.terminal_tx.take() {
                let _ = terminal_tx.send(terminal);
            }
            let worker = sidecar
                .worker
                .take()
                .expect("camera worker exists until finish");
            return worker.join().map_err(|_| {
                camera_error(
                    "camera capture worker terminated unexpectedly",
                    "the camera owner panicked before terminal hand-off",
                )
            })?;
        }
        #[cfg(not(any(windows, target_os = "macos")))]
        {
            let _ = (self, terminal);
            Err(camera_error(
                "camera capture is unavailable on this platform",
                "no native camera owner exists",
            ))
        }
    }

    pub(super) fn finish_for_screen(
        sidecar: Option<Self>,
        screen_completed: bool,
    ) -> record_core::Result<Option<CameraArtifact>> {
        let terminal = if screen_completed {
            CameraTerminalState::Complete
        } else {
            CameraTerminalState::Cancelled
        };
        match sidecar {
            Some(sidecar) => sidecar.finish(terminal),
            None => Ok(None),
        }
    }
}

impl Drop for CameraCaptureSidecar {
    fn drop(&mut self) {
        #[cfg(any(windows, target_os = "macos"))]
        {
            // Normal capture always consumes the sidecar through `finish`.
            // An early unwind still sends the same Cancelled authority and
            // waits for native Close before detaching its exact worker.
            if let Some(terminal_tx) = self.terminal_tx.take() {
                let _ = terminal_tx.send(CameraTerminalState::Cancelled);
            }
            if let Some(worker) = self.worker.take() {
                let _ = worker.join();
            }
        }
    }
}

#[cfg(any(windows, target_os = "macos"))]
fn await_terminal_command(
    terminal_rx: &mpsc::Receiver<CameraTerminalState>,
) -> record_core::Result<CameraTerminalState> {
    terminal_rx.recv().map_err(|_| {
        camera_error(
            "camera capture lost its terminal authority",
            "the screen recording owner dropped before sending CameraTerminalState",
        )
    })
}

#[cfg(any(windows, target_os = "macos"))]
fn camera_refusal(readiness: record_capture::CameraReadiness) -> RecordError {
    let detail = match readiness {
        record_capture::CameraReadiness::Missing { detail }
        | record_capture::CameraReadiness::Enumerated { detail, .. }
        | record_capture::CameraReadiness::PermissionRequired { detail, .. }
        | record_capture::CameraReadiness::PermissionDenied { detail, .. }
        | record_capture::CameraReadiness::Busy { detail }
        | record_capture::CameraReadiness::NoFrame { detail, .. }
        | record_capture::CameraReadiness::Ready { detail, .. } => detail,
    };
    camera_error("the selected camera could not start", &detail)
}

fn camera_error(message: &str, cause: &str) -> RecordError {
    RecordError::new(error_codes::CAPTURE, message, cause)
        .with_action("check the Camera control in Recorder, then retry")
}

#[cfg(all(test, any(windows, target_os = "macos")))]
mod tests {
    use super::*;

    #[test]
    fn terminal_handoff_preserves_the_screen_terminal_state() {
        let (terminal_tx, terminal_rx) = mpsc::sync_channel(1);
        terminal_tx.send(CameraTerminalState::Cancelled).unwrap();

        assert_eq!(
            await_terminal_command(&terminal_rx).unwrap(),
            CameraTerminalState::Cancelled
        );
    }

    #[test]
    fn dropped_screen_owner_is_a_typed_terminal_handoff_failure() {
        let (terminal_tx, terminal_rx) = mpsc::sync_channel(1);
        drop(terminal_tx);

        let error = await_terminal_command(&terminal_rx).unwrap_err();
        assert_eq!(error.code, error_codes::CAPTURE);
        assert_eq!(error.message, "camera capture lost its terminal authority");
    }
}
