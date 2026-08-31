//! macOS native-start ordering for one prepared capture root.

use super::windows_pause_evidence::{
    CalibratedWindowsPauseEvidenceFactory, LocalWindowsPauseArtifactVerifier,
};
use super::windows_pause_session::{WindowsPauseSession, WindowsPauseSessionError};
use record_core::CaptureCadence;
use record_recovery::{
    read_manifest, CaptureRoot, RecordingProjectBinding, RecordingSessionIntent,
    RecordingSessionJournalFile, RecordingStream,
};

pub(super) fn start_in_prepared_capture(
    project_dir: &std::path::Path,
    capture_id: &str,
    active_duration_limit_ms: Option<u64>,
    exact_monitor_id: String,
    fps: f64,
    streams: record_capture::SelectedCaptureStreams,
    microphone_source: record_capture::MicrophoneSource,
    project_binding: RecordingProjectBinding,
) -> Result<
    WindowsPauseSession<
        RecordingSessionJournalFile,
        CalibratedWindowsPauseEvidenceFactory<LocalWindowsPauseArtifactVerifier>,
        record_capture::private_macos_pause_owner::MacosPausePilotThread,
    >,
    WindowsPauseSessionError,
> {
    let root =
        CaptureRoot::for_project(project_dir).map_err(|_| WindowsPauseSessionError::Setup)?;
    let capture_dir = root
        .existing_capture_dir(capture_id)
        .map_err(|_| WindowsPauseSessionError::Setup)?
        .ok_or(WindowsPauseSessionError::Setup)?;
    let manifest = read_manifest(&capture_dir).map_err(|_| WindowsPauseSessionError::Setup)?;
    if manifest.start.capture_id != capture_id
        || manifest.start.checkpoint_interval_ms != super::recovery::CHECKPOINT_INTERVAL_MS
        || manifest.receipt.is_some()
        || manifest.has_torn_tail()
        || !valid_streams(&streams)
    {
        return Err(WindowsPauseSessionError::Setup);
    }
    let intent = RecordingSessionIntent::new(
        capture_id,
        manifest.start.started_unix_ms,
        super::recovery::CHECKPOINT_INTERVAL_MS,
        fps,
        active_duration_limit_ms,
        false,
        &exact_monitor_id,
        streams.streams().to_vec(),
    )
    .with_capture_cadence(
        CaptureCadence::from_server_fps(fps).map_err(|_| WindowsPauseSessionError::Setup)?,
    )
    .with_project_binding(project_binding.clone())
    .requiring_input_sidecars();
    let journal = RecordingSessionJournalFile::create_new(&root, capture_id, intent)
        .map_err(|_| WindowsPauseSessionError::Setup)?;
    let lifecycle = record_capture::private_macos_pause_owner::start_private(
        exact_monitor_id.clone(),
        fps,
        streams
            .streams()
            .contains(&RecordingStream::MicrophoneAudio),
        streams.streams().contains(&RecordingStream::SystemAudio),
        microphone_source,
        record_capture::CheckpointConfig {
            manifest_dir: capture_dir.display().to_string(),
            interval_ms: super::recovery::CHECKPOINT_INTERVAL_MS,
        },
    )
    .map_err(|_| WindowsPauseSessionError::Lifecycle)?;
    let factory = CalibratedWindowsPauseEvidenceFactory::for_exact_monitor(
        LocalWindowsPauseArtifactVerifier::new(capture_dir),
        exact_monitor_id,
    );
    WindowsPauseSession::from_started_macos_lifecycle_with_input_sidecars(
        journal,
        streams,
        lifecycle,
        factory,
        super::windows_pause_input_sidecar::WindowsPauseInputSidecarOwner::new(
            root,
            capture_id.into(),
            project_binding,
        ),
    )
}

fn valid_streams(streams: &record_capture::SelectedCaptureStreams) -> bool {
    let selected = streams.streams();
    let microphone = selected.contains(&RecordingStream::MicrophoneAudio);
    let system_audio = selected.contains(&RecordingStream::SystemAudio);
    *streams == record_capture::SelectedCaptureStreams::new(microphone, system_audio, false, false)
}
