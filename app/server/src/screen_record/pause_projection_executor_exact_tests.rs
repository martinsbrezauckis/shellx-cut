use std::path::Path;

use record_core::{
    CaptureCadence, CursorCoordinateSource, CursorCoordinateState, CursorCorrelation, EventTrack,
    FrameRate, Monitor, Settings,
};
use record_recovery::{
    CheckpointSequenceRange, DurableStateTransition, RecordingSessionIntent,
    RecordingSessionJournalFile, RecordingSessionState, RecordingStream, SealedRun,
    SessionTerminal, StreamFragment, StreamFragmentFacts, TerminalDisposition,
};
use sha2::{Digest, Sha256};

use super::super::pause_projection::SealedLegacyProjectionRun;
use super::stager::FrameGridPauseProjectionSourceStager;
use super::types::{
    PauseProjectionArtifactVerifier, ProjectionMediaFacts, VerifiedPauseProjectionSource,
};
use super::{execute_frame_grid_pause_projection, PauseProjectionExecution};

const CAPTURE_ID: &str = "pause-exact";

struct Fixture {
    _temp: tempfile::TempDir,
    root: record_recovery::CaptureRoot,
    capture: std::path::PathBuf,
    journal: record_recovery::RecordingSessionJournal,
    event_runs: Vec<SealedLegacyProjectionRun>,
}

struct StrictFakeVerifier;

impl PauseProjectionArtifactVerifier for StrictFakeVerifier {
    fn verify(&self, path: &Path) -> Result<ProjectionMediaFacts, cut_core::CutError> {
        let source = path.file_name().and_then(|name| name.to_str()) == Some("source.mp4");
        Ok(ProjectionMediaFacts {
            duration_ms: if source { 200 } else { 100 },
            decoded_video_frames: if source { 6 } else { 3 },
            has_audio: false,
            avg_frame_rate: Some(rate()),
            r_frame_rate: Some(rate()),
        })
    }
}

#[derive(Default)]
struct StrictFakeStager {
    calls: usize,
}

impl FrameGridPauseProjectionSourceStager for StrictFakeStager {
    fn stage_frame_grid(
        &mut self,
        _capture_dir: &Path,
        sources: &[VerifiedPauseProjectionSource],
        grid: &record_recovery::FrameGridStitchPlan,
        output: &Path,
    ) -> Result<(), cut_core::CutError> {
        assert_eq!(sources.len(), 2);
        assert_eq!(grid.output_frame_rate, rate());
        assert_eq!(grid.output_frame_count, 6);
        self.calls += 1;
        std::fs::write(output, b"strict compact source").unwrap();
        Ok(())
    }
}

fn fixture() -> Fixture {
    let temp = tempfile::tempdir().unwrap();
    let root = record_recovery::CaptureRoot::for_project(temp.path()).unwrap();
    let capture = root.create_capture_dir(CAPTURE_ID).unwrap();
    let screen = capture.join("screen");
    std::fs::create_dir(&screen).unwrap();
    let first = b"sealed-first";
    let second = b"sealed-second";
    std::fs::write(screen.join("run-0.mp4"), first).unwrap();
    std::fs::write(screen.join("run-1.mp4"), second).unwrap();
    let intent = RecordingSessionIntent::new(
        "exact-pause-session",
        1,
        100,
        30.0,
        None,
        false,
        "opaque-target",
        vec![RecordingStream::ScreenVideo],
    )
    .with_capture_cadence(CaptureCadence::from_server_fps(30.0).unwrap());
    let mut journal = RecordingSessionJournalFile::create_new(&root, CAPTURE_ID, intent).unwrap();
    journal
        .append_transition(transition(0, RecordingSessionState::Started, 0, 1))
        .unwrap();
    journal.seal_run(run(0, 0, 0, first)).unwrap();
    journal
        .append_transition(transition(1, RecordingSessionState::Paused, 100, 200))
        .unwrap();
    journal
        .append_transition(transition(2, RecordingSessionState::Resumed, 100, 1_200))
        .unwrap();
    journal.seal_run(run(1, 100, 1, second)).unwrap();
    journal
        .append_transition(transition(3, RecordingSessionState::Stopping, 200, 1_300))
        .unwrap();
    journal
        .seal_terminal(SessionTerminal {
            disposition: TerminalDisposition::Completed,
            logical_end_ms: 200,
            observed_unix_ms: 1_301,
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

fn run(sequence: u64, logical_start_ms: u64, checkpoint: u64, bytes: &[u8]) -> SealedRun {
    SealedRun {
        sequence,
        observed_start_ms: logical_start_ms + 10,
        observed_end_ms: logical_start_ms + 110,
        logical_start_ms,
        logical_end_ms: logical_start_ms + 100,
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
            sha256: format!("{:x}", Sha256::digest(bytes)),
            facts: StreamFragmentFacts {
                start_offset_ms: 0,
                end_offset_ms: 100,
                media_duration_ms: 100,
                decoded_video_frames: Some(3),
                avg_frame_rate: Some(rate()),
                r_frame_rate: Some(rate()),
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

fn rate() -> FrameRate {
    FrameRate::new(30, 1).unwrap()
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
            detail: Some("exact projection fixture".into()),
        },
    }
}

#[test]
fn publishes_only_after_strict_grid_output_proof_and_retries_idempotently() {
    let fixture = fixture();
    let mut stager = StrictFakeStager::default();
    let first = execute_frame_grid_pause_projection(
        &fixture.root,
        CAPTURE_ID,
        &fixture.journal,
        &fixture.event_runs,
        &StrictFakeVerifier,
        &mut stager,
    )
    .unwrap();
    let second = execute_frame_grid_pause_projection(
        &fixture.root,
        CAPTURE_ID,
        &fixture.journal,
        &fixture.event_runs,
        &StrictFakeVerifier,
        &mut stager,
    )
    .unwrap();
    assert_eq!(first.execution, PauseProjectionExecution::Published);
    assert_eq!(second.execution, PauseProjectionExecution::AlreadyComplete);
    assert_eq!(first.source_duration_ms, 200);
    assert_eq!(stager.calls, 1);
    assert_eq!(
        std::fs::read(fixture.capture.join("source.mp4")).unwrap(),
        b"strict compact source"
    );
}
