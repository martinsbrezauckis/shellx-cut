//! Private macOS composition for the pause-safe ScreenCaptureKit owner.

use cut_core::{error_codes, CutError};
use record_capture::{Capture, CaptureConfig, CaptureOutput, SelectedCaptureStreams};
use record_core::{error_codes as record_error_codes, RecordError};
use record_recovery::{CaptureRoot, RecordingStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

const STOP_ACK_TIMEOUT: Duration = Duration::from_secs(30);
const STOP_POLL_INTERVAL: Duration = Duration::from_millis(10);

/// Compose the native macOS pause owner with the ordinary reservation,
/// recovery manifest, durable session journal, projection, and Stop path.
#[allow(clippy::too_many_arguments, dead_code)]
pub(super) fn start_private_selected(
    capture_id: String,
    duration_ms: Option<u64>,
    fps: f64,
    monitor: Option<u32>,
    monitor_id: Option<String>,
    project_dir: PathBuf,
    out_dir: PathBuf,
    project_path: PathBuf,
    log_path: PathBuf,
    microphone_source: record_capture::MicrophoneSource,
    streams: SelectedCaptureStreams,
) -> Result<(), CutError> {
    start_selected(
        capture_id,
        duration_ms,
        fps,
        monitor,
        monitor_id,
        project_dir,
        out_dir,
        project_path,
        log_path,
        microphone_source,
        streams,
        None,
    )
}

/// Start the bounded public pause path. Its caller has already rejected every
/// stream or feature the native owner cannot durably seal.
#[allow(clippy::too_many_arguments)]
pub(super) fn start_public_selected(
    capture_id: String,
    duration_ms: Option<u64>,
    fps: f64,
    monitor: Option<u32>,
    monitor_id: Option<String>,
    project_dir: PathBuf,
    out_dir: PathBuf,
    project_path: PathBuf,
    log_path: PathBuf,
    microphone_source: record_capture::MicrophoneSource,
    microphone: bool,
    system_audio: bool,
) -> Result<(), CutError> {
    let controls = super::macos_pause_control::register(&capture_id)?;
    let streams = SelectedCaptureStreams::new(microphone, system_audio, false, false);
    start_selected(
        capture_id,
        duration_ms,
        fps,
        monitor,
        monitor_id,
        project_dir,
        out_dir,
        project_path,
        log_path,
        microphone_source,
        streams,
        Some(controls),
    )
}

#[allow(clippy::too_many_arguments)]
fn start_selected(
    capture_id: String,
    duration_ms: Option<u64>,
    fps: f64,
    monitor: Option<u32>,
    monitor_id: Option<String>,
    project_dir: PathBuf,
    out_dir: PathBuf,
    project_path: PathBuf,
    log_path: PathBuf,
    microphone_source: record_capture::MicrophoneSource,
    streams: SelectedCaptureStreams,
    controls: Option<super::macos_pause_control::PauseControlRegistration>,
) -> Result<(), CutError> {
    let exact_monitor_id = monitor_id.clone().ok_or_else(|| {
        CutError::new(
            error_codes::INVALID_ARGS,
            "private macOS pause capture requires an exact display",
            "select a current display from Recorder before starting capture",
        )
    })?;
    if !valid_streams(&streams) || !fps.is_finite() || fps.fract() != 0.0 {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            "private macOS pause capture has unsupported settings",
            "use an integer frame rate and screen with optional microphone or system audio",
        ));
    }
    let backend_project_dir = project_dir.clone();
    let backend_capture_id = capture_id.clone();
    super::start_capture_with_backend(
        capture_id,
        duration_ms,
        fps,
        None,
        streams
            .streams()
            .contains(&RecordingStream::MicrophoneAudio),
        microphone_source.clone(),
        streams.streams().contains(&RecordingStream::SystemAudio),
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
            Ok(Box::new(MacosPausePilotCapture {
                project_dir: backend_project_dir,
                capture_id: backend_capture_id,
                exact_monitor_id,
                fps,
                streams,
                microphone_source,
                controls,
            }))
        },
    )
}

struct MacosPausePilotCapture {
    project_dir: PathBuf,
    capture_id: String,
    exact_monitor_id: String,
    fps: f64,
    streams: SelectedCaptureStreams,
    microphone_source: record_capture::MicrophoneSource,
    controls: Option<super::macos_pause_control::PauseControlRegistration>,
}

impl Capture for MacosPausePilotCapture {
    fn capture(
        &self,
        cfg: &CaptureConfig,
        stop: Arc<AtomicBool>,
    ) -> record_core::Result<CaptureOutput> {
        if let Some(placement) = cfg.controller_placement.as_ref() {
            placement.not_excluded(
                "Full-display pause recording can include Cut when unobscured; no controller exclusion or auto-hide is applied.",
            );
        }
        let root = CaptureRoot::for_project(&self.project_dir)
            .map_err(|_| capture_error("open prepared private capture root"))?;
        let project_binding = super::windows_pause_input_sidecar::admit_project_binding(&root)
            .map_err(|_| capture_error("admit private project identity and revision"))?;
        let mut session = super::macos_pause_session_start::start_in_prepared_capture(
            &self.project_dir,
            &self.capture_id,
            cfg.duration_ms,
            self.exact_monitor_id.clone(),
            self.fps,
            self.streams.clone(),
            self.microphone_source.clone(),
            project_binding,
            cfg.active_preview.clone(),
        )
        .map_err(session_error)?;
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
        let mut pending_control: Option<super::macos_pause_control::PauseRequest> = None;
        loop {
            let now = Instant::now();
            if !stop_requested
                && (stop.load(Ordering::Relaxed)
                    || deadline.is_some_and(|deadline| now >= deadline))
            {
                if let Some(request) = pending_control.take() {
                    request.complete(Err(
                        "Stop began before the requested pause transition sealed".into(),
                    ));
                }
                if let Some(controls) = self.controls.as_ref() {
                    controls.reject_queued(
                        "Stop began before the requested pause transition could start",
                    );
                }
                session.request_stop_at(now).map_err(session_error)?;
                stop_requested = true;
                acknowledgement_deadline = now.checked_add(STOP_ACK_TIMEOUT);
            }
            if !stop_requested && pending_control.is_none() {
                if let Some(request) = self
                    .controls
                    .as_ref()
                    .and_then(|controls| controls.next_request())
                {
                    let issued = match request.action() {
                        super::macos_pause_control::PauseAction::Pause => {
                            session.request_pause_at(now)
                        }
                        super::macos_pause_control::PauseAction::Resume => {
                            session.request_resume_at(now)
                        }
                    };
                    if issued.is_err() {
                        request.complete(Err(
                            "the requested transition is not valid for the current durable state"
                                .into(),
                        ));
                    } else {
                        pending_control = Some(request);
                    }
                }
            }
            match session.pump_once().map_err(session_error)? {
                Some(super::windows_pause_session::WindowsPauseSessionEvent::Paused) => {
                    super::macos_pause_transition::complete_transition(
                        pending_control.take(),
                        super::macos_pause_control::PauseAction::Pause,
                        super::macos_pause_control::PauseReceipt::paused(
                            super::macos_pause_transition::logical_time(&session),
                        ),
                    )?;
                }
                Some(super::windows_pause_session::WindowsPauseSessionEvent::Resumed) => {
                    super::macos_pause_transition::complete_transition(
                        pending_control.take(),
                        super::macos_pause_control::PauseAction::Resume,
                        super::macos_pause_control::PauseReceipt::resumed(
                            super::macos_pause_transition::logical_time(&session),
                        ),
                    )?;
                }
                Some(super::windows_pause_session::WindowsPauseSessionEvent::ResumeRefused) => {
                    let Some(request) = pending_control.take() else {
                        return Err(capture_error(
                            "macOS pause lifecycle refused an unrequested resume",
                        ));
                    };
                    if request.action() != super::macos_pause_control::PauseAction::Resume {
                        return Err(capture_error(
                            "macOS pause lifecycle refused the wrong public transition",
                        ));
                    }
                    request.complete(Err(
                        "the native owner refused Resume; the durable state remains paused".into(),
                    ));
                }
                Some(super::windows_pause_session::WindowsPauseSessionEvent::Stopped)
                    if stop_requested =>
                {
                    break
                }
                Some(_) if !stop_requested => {
                    return Err(capture_error(
                        "private macOS pause lifecycle changed without a server command",
                    ));
                }
                Some(_) | None => {}
            }
            if acknowledgement_deadline.is_some_and(|deadline| Instant::now() >= deadline) {
                return Err(capture_error(
                    "private macOS pause lifecycle did not acknowledge Stop",
                ));
            }
            std::thread::sleep(STOP_POLL_INTERVAL);
        }
        let mut projection =
            super::private_pause_capture_projection::PrivateFrameGridProjection::new(
                root,
                self.capture_id.clone(),
                self.streams.clone(),
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

fn valid_streams(streams: &SelectedCaptureStreams) -> bool {
    let selected = streams.streams();
    let microphone = selected.contains(&RecordingStream::MicrophoneAudio);
    let system_audio = selected.contains(&RecordingStream::SystemAudio);
    *streams == SelectedCaptureStreams::new(microphone, system_audio, false, false)
}

fn session_error(_: super::windows_pause_session::WindowsPauseSessionError) -> RecordError {
    capture_error("run private macOS pause session")
}

fn capture_error(message: &str) -> RecordError {
    RecordError::new(
        record_error_codes::CAPTURE,
        message,
        "the durable capture journal retained the incomplete native evidence",
    )
}
