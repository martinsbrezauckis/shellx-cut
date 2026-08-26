use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use super::super::dispatch;
use super::test_actor;
use crate::state::AppState;
use record_capture::{Capture, CaptureConfig, CaptureOutput};
use record_core::{EventTrack, RecordError, Settings};
use serde_json::json;

static CAPTURE_START_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

struct FakeBlockedPortal;

impl Capture for FakeBlockedPortal {
    fn capture(
        &self,
        _cfg: &CaptureConfig,
        stop: Arc<AtomicBool>,
    ) -> record_core::Result<CaptureOutput> {
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
            settings: Settings {
                width: 2,
                height: 2,
                fps: 30.0,
                audio_rate: 48_000,
            },
        })
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
    let _capture_lock = CAPTURE_START_LOCK.lock().unwrap();
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

#[tokio::test(flavor = "current_thread")]
async fn dispatch_stop_returns_the_fake_blocked_portal_terminal_failure_promptly() {
    let _capture_lock = CAPTURE_START_LOCK.lock().unwrap();
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
    crate::screen_record::start_capture_with_test_backend(
        capture_id.into(),
        None,
        30.0,
        false,
        false,
        false,
        None,
        None,
        project_dir.clone(),
        capture_dir.clone(),
        project_path.clone(),
        capture_dir.join("record.log"),
        || Ok(Box::new(FakeBlockedPortal)),
    )
    .unwrap();

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
