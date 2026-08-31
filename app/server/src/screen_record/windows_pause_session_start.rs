//! Windows-only native-start ordering for a prepared normal capture root.

use super::windows_pause_evidence::{
    CalibratedWindowsPauseEvidenceFactory, LocalWindowsPauseArtifactVerifier,
};
use super::windows_pause_session::{
    WindowsPauseSession, WindowsPauseSessionAdmission, WindowsPauseSessionError,
};
use record_core::CaptureCadence;
use record_recovery::{
    read_manifest, CaptureRoot, RecordingProjectBinding, RecordingSessionIntent,
    RecordingSessionJournalFile,
};

pub(super) fn start_in_prepared_capture(
    project_dir: &std::path::Path,
    capture_id: &str,
    active_duration_limit_ms: Option<u64>,
    admission: WindowsPauseSessionAdmission,
    project_binding: RecordingProjectBinding,
) -> Result<
    WindowsPauseSession<
        RecordingSessionJournalFile,
        CalibratedWindowsPauseEvidenceFactory<LocalWindowsPauseArtifactVerifier>,
        record_capture::windows_pause_pilot::WindowsPausePilotThread,
    >,
    WindowsPauseSessionError,
> {
    // Admission has already completed without filesystem/native work. The
    // ordinary screen-record start path owns capture-dir creation and the
    // checkpoint manifest; beginning either again would replace neither and
    // must fail instead of splitting recovery ownership.
    let root =
        CaptureRoot::for_project(project_dir).map_err(|_| WindowsPauseSessionError::Setup)?;
    let capture_dir = root
        .existing_capture_dir(capture_id)
        .map_err(|_| WindowsPauseSessionError::Setup)?;
    let capture_dir = capture_dir.ok_or(WindowsPauseSessionError::Setup)?;
    let manifest = read_manifest(&capture_dir).map_err(|_| WindowsPauseSessionError::Setup)?;
    if manifest.start.capture_id != capture_id
        || manifest.start.checkpoint_interval_ms != admission.checkpoint_interval_ms
        || manifest.receipt.is_some()
        || manifest.has_torn_tail()
    {
        return Err(WindowsPauseSessionError::Setup);
    }
    let intent = RecordingSessionIntent::new(
        capture_id,
        manifest.start.started_unix_ms,
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
    )
    .with_project_binding(project_binding.clone())
    .requiring_input_sidecars();
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
    let factory = CalibratedWindowsPauseEvidenceFactory::for_exact_monitor(
        LocalWindowsPauseArtifactVerifier::new(capture_dir),
        admission.profile().exact_monitor_id().to_string(),
    );
    WindowsPauseSession::from_started_lifecycle_with_input_sidecars(
        journal,
        &admission,
        lifecycle,
        factory,
        super::windows_pause_input_sidecar::WindowsPauseInputSidecarOwner::new(
            root,
            capture_id.into(),
            project_binding,
        ),
    )
}
