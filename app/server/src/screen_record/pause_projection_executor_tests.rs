use std::path::Path;

use record_core::{
    CursorCoordinateSource, CursorCoordinateState, CursorCorrelation, EventTrack, Monitor, Settings,
};
use record_recovery::{
    CheckpointSequenceRange, DurableStateTransition, RecordingSessionIntent,
    RecordingSessionJournalFile, RecordingSessionState, RecordingStream, RunAwareStitchPlan,
    RunAwareStitchSpan, SealedRun, SessionTerminal, StreamFragment, StreamFragmentFacts,
    TerminalDisposition,
};
use sha2::{Digest, Sha256};

use super::super::pause_projection::SealedLegacyProjectionRun;
use super::types::{
    PauseProjectionExecution, ProjectionMediaFacts, StagedPauseProjectionSource,
    VerifiedPauseProjectionSource,
};
use super::{
    execute_pause_projection, PauseProjectionArtifactVerifier, PauseProjectionSourceStager,
};

const CAPTURE_ID: &str = "pause-owned";

struct Fixture {
    _temp: tempfile::TempDir,
    root: record_recovery::CaptureRoot,
    capture: std::path::PathBuf,
    journal: record_recovery::RecordingSessionJournal,
    event_runs: Vec<SealedLegacyProjectionRun>,
}

struct FakeVerifier;

impl PauseProjectionArtifactVerifier for FakeVerifier {
    fn verify(&self, path: &Path) -> Result<ProjectionMediaFacts, cut_core::CutError> {
        let name = path.file_name().and_then(|name| name.to_str());
        Ok(ProjectionMediaFacts {
            duration_ms: match name {
                Some("run-0.mp4") => 50,
                Some("run-1.mp4") => 100,
                Some("source.mp4") => 200,
                _ => 0,
            },
            decoded_video_frames: match name {
                Some("run-0.mp4") => 3,
                Some("run-1.mp4") => 6,
                Some("source.mp4") => 9,
                _ => 0,
            },
            has_audio: false,
            avg_frame_rate: None,
            r_frame_rate: None,
        })
    }
}

#[derive(Default)]
struct FakeStager {
    calls: usize,
    plans: Vec<RunAwareStitchPlan>,
}

impl PauseProjectionSourceStager for FakeStager {
    fn stage(
        &mut self,
        _sources: &[VerifiedPauseProjectionSource],
        plan: &RunAwareStitchPlan,
        output: &Path,
    ) -> Result<StagedPauseProjectionSource, cut_core::CutError> {
        self.calls += 1;
        self.plans.push(plan.clone());
        std::fs::write(output, format!("compact-source-{}", plan.duration_ms)).unwrap();
        Ok(StagedPauseProjectionSource {
            duration_ms: plan.duration_ms,
        })
    }
}

fn fixture() -> Fixture {
    let temp = tempfile::tempdir().unwrap();
    let root = record_recovery::CaptureRoot::for_project(temp.path()).unwrap();
    let capture = root.create_capture_dir(CAPTURE_ID).unwrap();
    let screen = capture.join("screen");
    std::fs::create_dir(&screen).unwrap();
    let first = b"sealed-run-zero";
    let second = b"sealed-run-one";
    std::fs::write(screen.join("run-0.mp4"), first).unwrap();
    std::fs::write(screen.join("run-1.mp4"), second).unwrap();

    let intent = RecordingSessionIntent::new(
        "pause-session",
        1,
        100,
        30.0,
        None,
        false,
        "opaque-target",
        vec![RecordingStream::ScreenVideo],
    );
    let mut journal = RecordingSessionJournalFile::create_new(&root, CAPTURE_ID, intent).unwrap();
    journal
        .append_transition(transition(0, RecordingSessionState::Started, 0, 1))
        .unwrap();
    journal
        .seal_run(run(0, 0, 100, 0, 100, 50, 3, first))
        .unwrap();
    journal
        .append_transition(transition(1, RecordingSessionState::Paused, 100, 120))
        .unwrap();
    journal
        .append_transition(transition(2, RecordingSessionState::Resumed, 100, 1_100))
        .unwrap();
    journal
        .seal_run(run(1, 100, 200, 1, 100, 100, 6, second))
        .unwrap();
    journal
        .append_transition(transition(3, RecordingSessionState::Stopping, 200, 1_220))
        .unwrap();
    journal
        .seal_terminal(SessionTerminal {
            disposition: TerminalDisposition::Completed,
            logical_end_ms: 200,
            observed_unix_ms: 1_221,
        })
        .unwrap();
    Fixture {
        _temp: temp,
        root,
        capture,
        journal: journal.journal().clone(),
        event_runs: vec![
            SealedLegacyProjectionRun::new(0, 0, 100, settings(), events(100)),
            SealedLegacyProjectionRun::new(1, 100, 200, settings(), events(100)),
        ],
    }
}

fn run(
    sequence: u64,
    logical_start_ms: u64,
    logical_end_ms: u64,
    checkpoint: u64,
    span_ms: u64,
    media_duration_ms: u64,
    frames: u64,
    bytes: &[u8],
) -> SealedRun {
    SealedRun {
        sequence,
        observed_start_ms: logical_start_ms + 10,
        observed_end_ms: logical_end_ms + 10,
        logical_start_ms,
        logical_end_ms,
        checkpoints: CheckpointSequenceRange {
            first: checkpoint,
            last: checkpoint,
        },
        fragments: vec![StreamFragment {
            stream: RecordingStream::ScreenVideo,
            checkpoint_sequence: Some(checkpoint),
            stream_sequence: 0,
            artifact: format!("screen/run-{checkpoint}.mp4"),
            bytes: bytes.len() as u64,
            sha256: digest(bytes),
            facts: StreamFragmentFacts {
                start_offset_ms: 0,
                end_offset_ms: span_ms,
                media_duration_ms,
                decoded_video_frames: Some(frames),
                avg_frame_rate: None,
                r_frame_rate: None,
            },
        }],
    }
}

fn transition(
    sequence: u64,
    state: RecordingSessionState,
    logical_offset_ms: u64,
    observed_unix_ms: u64,
) -> DurableStateTransition {
    DurableStateTransition {
        sequence,
        state,
        logical_offset_ms,
        observed_unix_ms,
    }
}

fn settings() -> Settings {
    Settings {
        width: 640,
        height: 360,
        fps: 30.0,
        audio_rate: 48_000,
    }
}

fn events(duration_ms: u64) -> EventTrack {
    EventTrack {
        duration_ms,
        screen_w: 640,
        screen_h: 360,
        monitors: vec![Monitor {
            id: 1,
            x: 0,
            y: 0,
            w: 640,
            h: 360,
            primary: true,
        }],
        cursor: vec![],
        clicks: vec![],
        scrolls: vec![],
        keys: vec![],
        cursor_correlation: CursorCorrelation {
            source: CursorCoordinateSource::RdevinAbsolute,
            state: CursorCoordinateState::Exact,
            exact_clicks: 0,
            approximate_clicks: 0,
            unavailable_clicks: 0,
            max_metadata_age_ms: Some(16),
            detail: Some("fixture".into()),
        },
    }
}

fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn execute(fixture: &Fixture, stager: &mut FakeStager) -> super::PauseProjectionResult {
    execute_pause_projection(
        &fixture.root,
        CAPTURE_ID,
        &fixture.journal,
        &fixture.event_runs,
        &FakeVerifier,
        stager,
    )
    .unwrap()
}

#[test]
fn refuses_hash_drift_and_unsafe_artifact_paths() {
    let drift_fixture = fixture();
    std::fs::write(drift_fixture.capture.join("screen/run-0.mp4"), b"drifted").unwrap();
    assert!(execute_pause_projection(
        &drift_fixture.root,
        CAPTURE_ID,
        &drift_fixture.journal,
        &drift_fixture.event_runs,
        &FakeVerifier,
        &mut FakeStager::default()
    )
    .is_err());
    assert!(!drift_fixture.capture.join("source.mp4").exists());

    #[cfg(unix)]
    {
        use std::os::unix::fs::symlink;
        let linked_fixture = fixture();
        let outside = linked_fixture.capture.join("outside.mp4");
        std::fs::write(&outside, b"sealed-run-zero").unwrap();
        std::fs::remove_file(linked_fixture.capture.join("screen/run-0.mp4")).unwrap();
        symlink(&outside, linked_fixture.capture.join("screen/run-0.mp4")).unwrap();
        assert!(execute_pause_projection(
            &linked_fixture.root,
            CAPTURE_ID,
            &linked_fixture.journal,
            &linked_fixture.event_runs,
            &FakeVerifier,
            &mut FakeStager::default()
        )
        .is_err());
    }
}

#[test]
fn publishes_compact_duration_with_encoder_padding_but_no_pause_padding() {
    let fixture = fixture();
    let mut stager = FakeStager::default();
    let result = execute(&fixture, &mut stager);
    assert_eq!(result.execution, PauseProjectionExecution::Published);
    assert_eq!(result.source_duration_ms, 200);
    assert!(stager.plans[0]
        .spans
        .contains(&RunAwareStitchSpan::EncoderGapPadding {
            run_sequence: 0,
            logical_start_ms: 50,
            logical_end_ms: 100,
        }));
    assert!(stager.plans[0].spans.iter().all(|span| !matches!(
        span,
        RunAwareStitchSpan::EncoderGapPadding {
            logical_start_ms: 100,
            ..
        }
    )));
    let events: EventTrack =
        serde_json::from_slice(&std::fs::read(fixture.capture.join("events.json")).unwrap())
            .unwrap();
    let project: record_core::RecordingProject =
        serde_json::from_slice(&std::fs::read(fixture.capture.join("project.json")).unwrap())
            .unwrap();
    assert_eq!(events.duration_ms, 200);
    assert_eq!(project.events.duration_ms, 200);
    assert_eq!(project.source_video, "source.mp4");
}

#[test]
fn completed_projection_is_idempotent_without_restaging() {
    let fixture = fixture();
    let mut stager = FakeStager::default();
    let first = execute(&fixture, &mut stager);
    let source = std::fs::read(fixture.capture.join("source.mp4")).unwrap();
    let second = execute(&fixture, &mut stager);
    assert_eq!(first.execution, PauseProjectionExecution::Published);
    assert_eq!(second.execution, PauseProjectionExecution::AlreadyComplete);
    assert_eq!(stager.calls, 1);
    assert_eq!(
        std::fs::read(fixture.capture.join("source.mp4")).unwrap(),
        source
    );
}

#[test]
fn retry_completes_after_a_partial_no_replace_failure() {
    let fixture = fixture();
    let mut stager = FakeStager::default();
    std::fs::create_dir(fixture.capture.join("project.json")).unwrap();
    assert!(execute_pause_projection(
        &fixture.root,
        CAPTURE_ID,
        &fixture.journal,
        &fixture.event_runs,
        &FakeVerifier,
        &mut stager
    )
    .is_err());
    assert!(fixture.capture.join("source.mp4").is_file());
    assert!(fixture.capture.join("events.json").is_file());
    std::fs::remove_dir(fixture.capture.join("project.json")).unwrap();
    let retried = execute(&fixture, &mut stager);
    assert_eq!(retried.execution, PauseProjectionExecution::Published);
    assert_eq!(stager.calls, 2);
    assert!(fixture.capture.join("project.json").is_file());
}
