//! Windows-only fresh filesystem and native-start ordering for pause sessions.

use super::windows_pause_evidence::{
    CalibratedWindowsPauseEvidenceFactory, LocalWindowsPauseArtifactVerifier,
};
use super::windows_pause_session::{
    WindowsPauseSession, WindowsPauseSessionAdmission, WindowsPauseSessionError,
};
use record_core::CaptureCadence;
use record_recovery::{
    CaptureRoot, CaptureStart, ManifestOwner, RecordingSessionIntent, RecordingSessionJournalFile,
};

pub(super) fn start_private(
    project_dir: &std::path::Path,
    capture_id: &str,
    created_unix_ms: u64,
    active_duration_limit_ms: Option<u64>,
    admission: WindowsPauseSessionAdmission,
) -> Result<
    WindowsPauseSession<
        RecordingSessionJournalFile,
        CalibratedWindowsPauseEvidenceFactory<LocalWindowsPauseArtifactVerifier>,
        record_capture::windows_pause_pilot::WindowsPausePilotThread,
    >,
    WindowsPauseSessionError,
> {
    // Admission has already completed without filesystem/native work.
    let root =
        CaptureRoot::for_project(project_dir).map_err(|_| WindowsPauseSessionError::Setup)?;
    let capture_dir = root
        .create_capture_dir(capture_id)
        .map_err(|_| WindowsPauseSessionError::Setup)?;
    ManifestOwner::begin(
        &capture_dir,
        CaptureStart::new(capture_id, admission.checkpoint_interval_ms),
    )
    .map_err(|_| WindowsPauseSessionError::Setup)?;
    let intent = RecordingSessionIntent::new(
        capture_id,
        created_unix_ms,
        admission.checkpoint_interval_ms,
        admission.profile().fps() as f64,
        active_duration_limit_ms,
        false,
        admission.profile().exact_monitor_id(),
        admission.streams.streams().to_vec(),
    )
    .with_capture_cadence(
        CaptureCadence::from_server_fps(admission.profile().fps() as f64)
            .map_err(|_| WindowsPauseSessionError::Setup)?,
    );
    let journal = RecordingSessionJournalFile::create_new(&root, capture_id, intent)
        .map_err(|_| WindowsPauseSessionError::Setup)?;
    let lifecycle = record_capture::windows_pause_pilot::start_private(
        admission.profile.clone(),
        record_capture::CheckpointConfig {
            manifest_dir: capture_dir.display().to_string(),
            interval_ms: admission.checkpoint_interval_ms,
        },
    )
    .map_err(|_| WindowsPauseSessionError::Lifecycle)?;
    let factory = CalibratedWindowsPauseEvidenceFactory::new(
        LocalWindowsPauseArtifactVerifier::new(capture_dir),
    );
    WindowsPauseSession::from_started_lifecycle(journal, &admission, lifecycle, factory)
}
