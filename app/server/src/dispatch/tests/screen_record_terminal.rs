use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use super::super::dispatch;
use super::test_actor;
use crate::state::AppState;
use record_capture::{Capture, CaptureConfig, CaptureOutput};
use record_core::{CaptureCadence, EventTrack, RecordError, Settings};
use serde_json::json;

struct FakeBlockedPortal {
    entered_native_backend: std::sync::mpsc::SyncSender<()>,
}

impl Capture for FakeBlockedPortal {
    fn capture(
        &self,
        _cfg: &CaptureConfig,
        stop: Arc<AtomicBool>,
    ) -> record_core::Result<CaptureOutput> {
        // The terminal-path test must distinguish a Stop after native capture
        // entry from the intentional Stop-before-launch handoff refusal. This
        // fake is the native backend for that test, so signal only once the
        // worker has claimed the handoff and invoked Capture::capture.
        self.entered_native_backend.send(()).map_err(|error| {
            RecordError::new(
                record_core::error_codes::CAPTURE,
                "fake portal could not report native capture entry",
                error.to_string(),
            )
        })?;
        while !stop.load(Ordering::Relaxed) {
            std::thread::sleep(Duration::from_millis(2));
        }
        Err(RecordError::new(
            record_core::error_codes::CAPTURE,
            "screen capture stopped before the first frame",
            "fake portal source selection was interrupted by screen_record.stop",
        ))
    }
}

struct FakeSuccessfulPortal;

impl Capture for FakeSuccessfulPortal {
    fn capture(
        &self,
        cfg: &CaptureConfig,
        _stop: Arc<AtomicBool>,
    ) -> record_core::Result<CaptureOutput> {
        let checkpoint_root = Path::new(
            &cfg.checkpoint
                .as_ref()
                .expect("server start always configures checkpoints")
                .manifest_dir,
        );
        let mut owner = record_recovery::ManifestOwner::open(checkpoint_root).map_err(|error| {
            RecordError::new(
                record_core::error_codes::CAPTURE,
                "fake portal could not open checkpoint manifest",
                error.to_string(),
            )
        })?;
        let staging = owner.begin_segment(0, 0).map_err(manifest_error)?;
        std::fs::write(&staging, b"fake finalized portal checkpoint").map_err(|error| {
            RecordError::new(
                record_core::error_codes::IO,
                "fake portal could not write checkpoint",
                error.to_string(),
            )
        })?;
        owner
            .publish(
                0,
                &staging,
                record_recovery::CheckpointFacts {
                    start_ms: 0,
                    end_ms: 10,
                    event_offset_ms: 0,
                    audio_offset_ms: None,
                },
                record_recovery::MediaFacts {
                    duration_ms: 10,
                    decoded_video_frames: 1,
                    has_audio: false,
                    width: None,
                    height: None,
                    codec_name: None,
                    avg_frame_rate: None,
                    r_frame_rate: None,
                },
            )
            .map_err(manifest_error)?;
        let source = Path::new(&cfg.out_dir).join("source.mp4");
        std::fs::write(&source, b"fake portal source").map_err(|error| {
            RecordError::new(
                record_core::error_codes::IO,
                "fake portal could not write source",
                error.to_string(),
            )
        })?;
        Ok(CaptureOutput {
            source_video: source.display().to_string(),
            events: EventTrack {
                duration_ms: 10,
                screen_w: 2,
                screen_h: 2,
                monitors: vec![],
                cursor: vec![],
                clicks: vec![],
                scrolls: vec![],
                keys: vec![],
                cursor_correlation: Default::default(),
            },
            camera_artifact: None,
            webcam_video: None,
            audio: None,
            microphone_outcome: record_capture::MicrophoneCaptureOutcome::NotRequested,
            settings: Settings {
                width: 2,
                height: 2,
                fps: 30.0,
                audio_rate: 48_000,
            },
            capture_quality: None,
            verified_media: None,
        })
    }
}

struct FakePrepublishedPauseProjection {
    matches_returned_project: bool,
}

impl Capture for FakePrepublishedPauseProjection {
    fn capture(
        &self,
        cfg: &CaptureConfig,
        _stop: Arc<AtomicBool>,
    ) -> record_core::Result<CaptureOutput> {
        let checkpoint_root = Path::new(
            &cfg.checkpoint
                .as_ref()
                .expect("server start always configures checkpoints")
                .manifest_dir,
        );
        let mut owner = record_recovery::ManifestOwner::open(checkpoint_root).map_err(|error| {
            RecordError::new(
                record_core::error_codes::CAPTURE,
                "fake pause projection could not open checkpoint manifest",
                error.to_string(),
            )
        })?;
        let staging = owner.begin_segment(0, 0).map_err(manifest_error)?;
        std::fs::write(&staging, b"fake finalized pause checkpoint").map_err(|error| {
            RecordError::new(
                record_core::error_codes::IO,
                "fake pause projection could not write checkpoint",
                error.to_string(),
            )
        })?;
        owner
            .publish(
                0,
                &staging,
                record_recovery::CheckpointFacts {
                    start_ms: 0,
                    end_ms: 10,
                    event_offset_ms: 0,
                    audio_offset_ms: None,
                },
                record_recovery::MediaFacts {
                    duration_ms: 10,
                    decoded_video_frames: 1,
                    has_audio: false,
                    width: None,
                    height: None,
                    codec_name: None,
                    avg_frame_rate: None,
                    r_frame_rate: None,
                },
            )
            .map_err(manifest_error)?;
        std::fs::write(
            Path::new(&cfg.out_dir).join("source.mp4"),
            b"fake pause source",
        )
        .map_err(|error| {
            RecordError::new(
                record_core::error_codes::IO,
                "fake pause projection could not write source",
                error.to_string(),
            )
        })?;
        let output = CaptureOutput {
            source_video: "source.mp4".into(),
            events: EventTrack {
                duration_ms: 10,
                screen_w: 2,
                screen_h: 2,
                monitors: vec![],
                cursor: vec![],
                clicks: vec![],
                scrolls: vec![],
                keys: vec![],
                cursor_correlation: Default::default(),
            },
            camera_artifact: None,
            webcam_video: None,
            audio: None,
            microphone_outcome: record_capture::MicrophoneCaptureOutcome::NotRequested,
            settings: Settings {
                width: 2,
                height: 2,
                fps: 30.0,
                audio_rate: 48_000,
            },
            capture_quality: None,
            verified_media: None,
        };
        let mut project = output
            .clone()
            .into_project_with_capture_cadence(CaptureCadence::from_server_fps(cfg.fps).unwrap());
        if !self.matches_returned_project {
            project.source_video = "other-source.mp4".into();
        }
        record_recovery::replace_synced(
            &Path::new(&cfg.out_dir).join("project.json"),
            &serde_json::to_vec_pretty(&project).unwrap(),
        )
        .map_err(|error| {
            RecordError::new(
                record_core::error_codes::IO,
                "fake pause projection could not publish project",
                error.to_string(),
            )
        })?;
        Ok(output)
    }

    fn prepublished_project(&self) -> bool {
        true
    }
}

fn manifest_error(error: record_recovery::ManifestError) -> RecordError {
    RecordError::new(
        record_core::error_codes::CAPTURE,
        "fake portal could not publish checkpoint",
        error.to_string(),
    )
}

fn create_capture(
    project_dir: &Path,
    capture_id: &str,
) -> (std::path::PathBuf, std::path::PathBuf) {
    let capture_dir = crate::screen_record::create_capture_dir(project_dir, capture_id).unwrap();
    crate::screen_record::recovery::begin(&capture_dir, capture_id).unwrap();
    crate::screen_record::publish_marker(
        project_dir,
        capture_id,
        &serde_json::to_vec(&json!({"duration_ms": null})).unwrap(),
    )
    .unwrap();
    let project =
        crate::screen_record::capture_file(project_dir, capture_id, "project.json").unwrap();
    (capture_dir, project)
}

async fn wait_for(path: &Path) {
    tokio::time::timeout(Duration::from_secs(2), async {
        while !path.is_file() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("capture worker did not publish its terminal projection");
}

async fn wait_for_capture_release(capture_id: &str) {
    tokio::time::timeout(Duration::from_secs(2), async {
        while crate::screen_record::stop_capture(capture_id) {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("capture worker did not release its reservation");
}

#[tokio::test(flavor = "current_thread")]
async fn fake_successful_portal_still_publishes_checkpoint_and_project() {
    let _capture_lock = crate::screen_record::capture_test_lock().lock().await;
    let temp = tempfile::tempdir().unwrap();
    let project_dir = temp.path().join("successful_portal.cutproj");
    std::fs::create_dir_all(&project_dir).unwrap();
    let capture_id = "cap-fake-portal-success";
    let (capture_dir, project_path) = create_capture(&project_dir, capture_id);

    crate::screen_record::start_capture_with_test_backend(
        capture_id.into(),
        None,
        30.0,
        false,
        false,
        false,
        None,
        None,
        None,
        None,
        project_dir,
        capture_dir.clone(),
        project_path.clone(),
        capture_dir.join("record.log"),
        || Ok(Box::new(FakeSuccessfulPortal)),
    )
    .unwrap();
    wait_for(&project_path).await;
    // Project publication deliberately precedes the durable Complete receipt.
    // Wait for worker cleanup before inspecting the manifest, or this assertion
    // races the final receipt append under a busy test runner.
    wait_for_capture_release(capture_id).await;

    let manifest = record_recovery::read_manifest(&capture_dir).unwrap();
    assert_eq!(manifest.checkpoints.len(), 1);
    assert_eq!(
        manifest.receipt.unwrap().state,
        record_recovery::RecoveryState::Complete
    );
    assert!(project_path.is_file());
    assert!(!capture_dir.join("capture.terminal.json").exists());
}

/// A client can lose the first Stop response after the worker has closed its
/// process-local reservation. The retained capture root is then the authority:
/// a repeated same-id Stop must re-read its finalized artifacts rather than
/// treating the absent in-memory control as an unknown recording.
#[tokio::test(flavor = "current_thread")]
async fn stop_reads_completed_artifacts_after_worker_releases_same_capture_id() {
    let _capture_lock = crate::screen_record::capture_test_lock().lock().await;
    let temp = tempfile::tempdir().unwrap();
    let state = AppState::new();
    let project_dir = temp.path().join("stop_after_worker_release.cutproj");
    let created = dispatch(
        &state,
        "project.create",
        json!({"name": "stop_after_worker_release", "dir": project_dir}),
        test_actor(),
    )
    .await;
    assert!(created.ok, "{:?}", created.error);

    let capture_id = "cap-stop-after-worker-release";
    let (capture_dir, project_path) = create_capture(&project_dir, capture_id);
    crate::screen_record::start_capture_with_test_backend(
        capture_id.into(),
        None,
        30.0,
        false,
        false,
        false,
        None,
        None,
        None,
        None,
        project_dir,
        capture_dir.clone(),
        project_path.clone(),
        capture_dir.join("record.log"),
        || Ok(Box::new(FakeSuccessfulPortal)),
    )
    .unwrap();
    wait_for(&project_path).await;
    wait_for_capture_release(capture_id).await;
    assert!(
        !crate::screen_record::stop_capture(capture_id),
        "the retry deliberately has no in-memory native owner to signal"
    );

    // This successful response stands in for a server-side completion whose
    // response could not be delivered to the client. The next request must not
    // depend on the already-released in-memory control.
    let first = dispatch(
        &state,
        "screen_record.stop",
        json!({"capture_id": capture_id, "autoedit": false}),
        test_actor(),
    )
    .await;
    assert!(first.ok, "{:?}", first.error);

    let retried = dispatch(
        &state,
        "screen_record.stop",
        json!({"capture_id": capture_id, "autoedit": false}),
        test_actor(),
    )
    .await;
    assert!(retried.ok, "{:?}", retried.error);
    let result = retried
        .result
        .as_ref()
        .expect("Stop returns artifact receipt");
    assert_eq!(result["capture_id"], capture_id);
    assert_eq!(
        result["source"],
        capture_dir
            .join("source.mp4")
            .canonicalize()
            .unwrap()
            .display()
            .to_string()
    );
    assert!(
        capture_dir.join("events.json").is_file(),
        "same-id recovery writes the normal stop event artifact"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn private_preprojection_uses_normal_recovery_without_replacing_its_project() {
    let _capture_lock = crate::screen_record::capture_test_lock().lock().await;
    let temp = tempfile::tempdir().unwrap();
    let project_dir = temp.path().join("private_pause_projection.cutproj");
    std::fs::create_dir_all(&project_dir).unwrap();
    let capture_id = "cap-private-pause-projection";
    let (capture_dir, project_path) = create_capture(&project_dir, capture_id);

    crate::screen_record::start_capture_with_test_backend(
        capture_id.into(),
        None,
        30.0,
        false,
        false,
        false,
        None,
        None,
        None,
        None,
        project_dir,
        capture_dir.clone(),
        project_path.clone(),
        capture_dir.join("record.log"),
        || {
            Ok(Box::new(FakePrepublishedPauseProjection {
                matches_returned_project: true,
            }))
        },
    )
    .unwrap();
    wait_for(&project_path).await;
    wait_for_capture_release(capture_id).await;

    let project: record_core::RecordingProject =
        serde_json::from_slice(&std::fs::read(&project_path).unwrap()).unwrap();
    assert_eq!(project.source_video, "source.mp4");
    let manifest = record_recovery::read_manifest(&capture_dir).unwrap();
    assert_eq!(
        manifest.receipt.unwrap().state,
        record_recovery::RecoveryState::Complete
    );
    assert!(
        !capture_dir.join("capture.terminal.json").exists(),
        "a matching private projection must not be replaced or turned into a failure"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn mismatched_private_preprojection_fails_closed_without_complete_receipt() {
    let _capture_lock = crate::screen_record::capture_test_lock().lock().await;
    let temp = tempfile::tempdir().unwrap();
    let project_dir = temp.path().join("mismatched_pause_projection.cutproj");
    std::fs::create_dir_all(&project_dir).unwrap();
    let capture_id = "cap-mismatched-pause-projection";
    let (capture_dir, project_path) = create_capture(&project_dir, capture_id);

    crate::screen_record::start_capture_with_test_backend(
        capture_id.into(),
        None,
        30.0,
        false,
        false,
        false,
        None,
        None,
        None,
        None,
        project_dir,
        capture_dir.clone(),
        project_path,
        capture_dir.join("record.log"),
        || {
            Ok(Box::new(FakePrepublishedPauseProjection {
                matches_returned_project: false,
            }))
        },
    )
    .unwrap();
    wait_for(&capture_dir.join("capture.terminal.json")).await;
    wait_for_capture_release(capture_id).await;

    let manifest = record_recovery::read_manifest(&capture_dir).unwrap();
    assert!(
        manifest.receipt.is_none(),
        "a mismatched private projection must never receive a Complete receipt"
    );
    assert!(std::fs::read_to_string(capture_dir.join("record.log"))
        .unwrap()
        .contains("verify prepublished project.json"));
}

#[tokio::test(flavor = "current_thread")]
async fn dispatch_stop_returns_the_fake_blocked_portal_terminal_failure_promptly() {
    let _capture_lock = crate::screen_record::capture_test_lock().lock().await;
    let temp = tempfile::tempdir().unwrap();
    let state = AppState::new();
    let project_dir = temp.path().join("blocked_portal.cutproj");
    let created = dispatch(
        &state,
        "project.create",
        json!({"name": "blocked_portal", "dir": project_dir}),
        test_actor(),
    )
    .await;
    assert!(created.ok, "{:?}", created.error);

    let capture_id = "cap-fake-portal-blocked";
    let (capture_dir, project_path) = create_capture(&project_dir, capture_id);
    let (entered_native_backend, native_backend_entered) = std::sync::mpsc::sync_channel(1);
    crate::screen_record::start_capture_with_test_backend(
        capture_id.into(),
        None,
        30.0,
        false,
        false,
        false,
        None,
        None,
        None,
        None,
        project_dir.clone(),
        capture_dir.clone(),
        project_path.clone(),
        capture_dir.join("record.log"),
        move || {
            Ok(Box::new(FakeBlockedPortal {
                entered_native_backend,
            }))
        },
    )
    .unwrap();

    // `screen_record.stop` is deliberately allowed to win before native launch
    // and return its distinct pre-native-start failure. This test covers the
    // other terminal branch, so wait until the fake backend has actually
    // entered before issuing Stop. No frame is marked ready, preserving the
    // requirement that successful native capture is admitted only after a
    // delivered first frame.
    native_backend_entered
        .recv_timeout(Duration::from_secs(2))
        .expect("capture worker must enter the fake native backend before stop");

    let started = Instant::now();
    let stopped = tokio::time::timeout(
        Duration::from_secs(2),
        dispatch(
            &state,
            "screen_record.stop",
            json!({"capture_id": capture_id, "autoedit": false}),
            test_actor(),
        ),
    )
    .await
    .expect("stop must observe the typed terminal failure without the 45s project poll");

    assert!(!stopped.ok);
    let error = stopped.error.expect("stop must return the terminal error");
    assert_eq!(error.code, record_core::error_codes::CAPTURE);
    assert_eq!(
        error.message,
        "screen capture stopped before the first frame"
    );
    assert!(error.cause.contains("fake portal source selection"));
    assert!(started.elapsed() < Duration::from_secs(2));
    wait_for(&capture_dir.join("capture.terminal.json")).await;
    assert!(std::fs::read_to_string(capture_dir.join("record.log"))
        .unwrap()
        .contains("capture failed [capture]"));
    assert!(
        !project_path.exists(),
        "failed pre-first-frame capture must not publish a project"
    );
    let manifest = record_recovery::read_manifest(&capture_dir).unwrap();
    assert!(manifest.checkpoints.is_empty());
    assert!(
        manifest.receipt.is_none(),
        "failure must never publish complete"
    );
    wait_for_capture_release(capture_id).await;
}
