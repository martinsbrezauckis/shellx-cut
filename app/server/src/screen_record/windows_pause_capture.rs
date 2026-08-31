//! Private Windows owner for the pause-pilot lifecycle.
//!
//! This is deliberately not selected by `screen_record.start`: normal Windows
//! recording owns passive input and may own audio, while the pilot can prove
//! an exact screen-video stream plus its sealed optional audio owners. The owner nevertheless uses the ordinary
//! in-process reservation, shared Stop signal, project projection, and recovery
//! receipt path so a future public admission does not need a second lifecycle.

use super::monitor_start_admission::Target;
use super::private_pause_capture_projection::PrivateFrameGridProjection;
use super::windows_pause_private_admission;
use super::windows_pause_session::{WindowsPauseSessionAdmission, WindowsPauseSessionError};
use cut_core::{error_codes, CutError};
use record_capture::{Capture, CaptureConfig, CaptureOutput};
use record_core::{error_codes as record_error_codes, RecordError};
use record_recovery::CaptureRoot;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

const STOP_ACK_TIMEOUT: Duration = Duration::from_secs(30);
const STOP_POLL_INTERVAL: Duration = Duration::from_millis(10);

/// Start one pre-admitted private screen-only lifecycle on the ordinary server
/// worker path. This helper has no public caller; it is retained solely as the
/// safe native composition point once all normally selected streams have an
/// equivalent pause/resume proof.
#[allow(clippy::too_many_arguments, dead_code)]
pub(super) fn start_private_screen_only(
    capture_id: String,
    duration_ms: Option<u64>,
    fps: f64,
    monitor: Option<u32>,
    monitor_id: Option<String>,
    project_dir: PathBuf,
    out_dir: PathBuf,
    project_path: PathBuf,
    log_path: PathBuf,
) -> Result<(), CutError> {
    start_private_selected(
        capture_id,
        duration_ms,
        fps,
        monitor,
        monitor_id,
        project_dir,
        out_dir,
        project_path,
        log_path,
        record_capture::SelectedCaptureStreams::screen_only(),
    )
}

/// The accepted private stream set is routed directly to the native profile.
/// The ordinary capture wrapper itself still receives no independent audio
/// worker flags: this owned pause lifecycle seals and projects those selected
/// WAVs, so a second outer recorder could not truthfully share its boundaries.
#[allow(clippy::too_many_arguments)]
fn start_private_selected(
    capture_id: String,
    duration_ms: Option<u64>,
    fps: f64,
    monitor: Option<u32>,
    monitor_id: Option<String>,
    project_dir: PathBuf,
    out_dir: PathBuf,
    project_path: PathBuf,
    log_path: PathBuf,
    streams: record_capture::SelectedCaptureStreams,
) -> Result<(), CutError> {
    let microphone = streams
        .streams()
        .contains(&record_recovery::RecordingStream::MicrophoneAudio);
    let system_audio = streams
        .streams()
        .contains(&record_recovery::RecordingStream::SystemAudio);
    if streams
        != record_capture::SelectedCaptureStreams::new(microphone, system_audio, false, false)
    {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            "private Windows pause capture has an unsupported stream selection",
            "the private pause pilot can own only exact screen video plus sealed microphone/system audio",
        ));
    }
    let admission = windows_pause_private_admission::admit(
        Target {
            legacy_index: monitor,
            exact_id: monitor_id.clone(),
        },
        fps,
        microphone,
        system_audio,
        false,
        false,
        false,
        false,
        false,
    )
    .map_err(|_| {
        CutError::new(
            error_codes::INVALID_ARGS,
            "private Windows pause capture requires exact selected streams",
            "the private pause pilot cannot seal every requested recording stream",
        )
    })?;
    let backend_project_dir = project_dir.clone();
    let backend_capture_id = capture_id.clone();
    super::start_capture_with_backend(
        capture_id,
        duration_ms,
        fps,
        None,
        false,
        record_capture::MicrophoneSource::SystemDefault,
        false,
        false,
        monitor,
        monitor_id,
        None,
        None,
        None,
        None,
        false,
        project_dir,
        out_dir,
        project_path,
        log_path,
        false,
        move || {
            Ok(Box::new(WindowsPausePilotCapture {
                project_dir: backend_project_dir,
                capture_id: backend_capture_id,
                admission,
            }))
        },
    )
}

struct WindowsPausePilotCapture {
    project_dir: PathBuf,
    capture_id: String,
    admission: WindowsPauseSessionAdmission,
}

impl Capture for WindowsPausePilotCapture {
    fn capture(
        &self,
        cfg: &CaptureConfig,
        stop: Arc<AtomicBool>,
    ) -> record_core::Result<CaptureOutput> {
        let root = CaptureRoot::for_project(&self.project_dir)
            .map_err(|_| capture_error("open prepared private capture root"))?;
        let project_binding = super::windows_pause_input_sidecar::admit_project_binding(&root)
            .map_err(|_| capture_error("admit private project identity and revision"))?;
        let mut session = super::windows_pause_session_start::start_in_prepared_capture(
            &self.project_dir,
            &self.capture_id,
            cfg.duration_ms,
            self.admission.clone(),
            project_binding,
        )
        .map_err(session_error)?;

        // Only the native Started fact may open the shared ordinary capture
        // clock. The normal CaptureSessionControl observes this same clock and
        // Stop terminalizes it before setting the atomic signal below.
        let started = cfg
            .clock
            .as_ref()
            .map(record_capture::CaptureClock::start)
            .unwrap_or_else(Instant::now);
        let deadline = cfg
            .duration_ms
            .and_then(|duration_ms| started.checked_add(Duration::from_millis(duration_ms)));

        let mut stop_requested = false;
        let mut acknowledgement_deadline = None;
        loop {
            let now = Instant::now();
            if !stop_requested
                && (stop.load(Ordering::Relaxed)
                    || deadline.is_some_and(|deadline| now >= deadline))
            {
                // The session owner records terminal Stop before the native
                // command leaves the process. Late Pause/Resume facts cannot
                // reopen it, and the bounded wait below never claims success
                // without the corresponding native Stop evidence.
                session.request_stop_at(now).map_err(session_error)?;
                stop_requested = true;
                acknowledgement_deadline = now.checked_add(STOP_ACK_TIMEOUT);
            }

            match session.pump_once().map_err(session_error)? {
                Some(super::windows_pause_session::WindowsPauseSessionEvent::Stopped)
                    if stop_requested =>
                {
                    break
                }
                Some(_) if !stop_requested => {
                    return Err(RecordError::new(
                        record_error_codes::CAPTURE,
                        "private Windows pause pilot changed state unexpectedly",
                        "no private pause or resume command is admitted on this capture path",
                    ));
                }
                Some(_) => {}
                None => {}
            }

            if acknowledgement_deadline.is_some_and(|deadline| Instant::now() >= deadline) {
                return Err(RecordError::new(
                    record_error_codes::CAPTURE,
                    "private Windows pause pilot did not acknowledge Stop",
                    "the terminal native Stop evidence was not received within the bounded acknowledgement window",
                ));
            }
            std::thread::sleep(STOP_POLL_INTERVAL);
        }

        let mut projection = PrivateFrameGridProjection::new(
            root.clone(),
            self.capture_id.clone(),
            self.admission.streams.clone(),
        );
        session
            .execute_completed_projection(&mut projection)
            .map_err(session_error)?;
        projection.capture_output()
    }

    fn prepublished_project(&self) -> bool {
        true
    }
}

fn session_error(_error: WindowsPauseSessionError) -> RecordError {
    capture_error("run private Windows pause session")
}

fn capture_error(message: &str) -> RecordError {
    RecordError::new(
        record_error_codes::CAPTURE,
        message,
        "the private pause pilot retained its incomplete capture evidence",
    )
}

#[cfg(test)]
#[path = "windows_pause_capture_tests.rs"]
mod tests;
