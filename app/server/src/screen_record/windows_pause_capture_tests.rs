use super::*;
use record_capture::MicrophoneCaptureOutcome;
use record_core::{CursorCorrelation, EventTrack, RecordingProject, Settings};

#[test]
fn private_projection_reports_only_the_staged_microphone_product_leaf() {
    let temp = tempfile::tempdir().unwrap();
    let root = CaptureRoot::for_project(temp.path()).unwrap();
    root.create_capture_dir("audio-projection").unwrap();
    let events = EventTrack {
        duration_ms: 100,
        screen_w: 640,
        screen_h: 360,
        monitors: Vec::new(),
        cursor: Vec::new(),
        clicks: Vec::new(),
        scrolls: Vec::new(),
        keys: Vec::new(),
        cursor_correlation: CursorCorrelation::default(),
    };
    let mut project = RecordingProject::new(
        "source.mp4",
        Settings {
            width: 640,
            height: 360,
            fps: 30.0,
            audio_rate: 48_000,
        },
        events,
    );
    project.audio = Some("mic.wav".into());
    std::fs::write(
        root.capture_file("audio-projection", "source.mp4").unwrap(),
        b"video",
    )
    .unwrap();
    std::fs::write(
        root.capture_file("audio-projection", "mic.wav").unwrap(),
        b"audio",
    )
    .unwrap();
    std::fs::write(
        root.capture_file("audio-projection", "project.json")
            .unwrap(),
        serde_json::to_vec(&project).unwrap(),
    )
    .unwrap();

    let output = PrivateFrameGridProjection::test_without_completed_audio_receipt(
        root,
        "audio-projection".into(),
        record_capture::SelectedCaptureStreams::new(true, false, false, false),
    )
    .capture_output()
    .unwrap();
    assert_eq!(output.audio.as_deref(), Some("mic.wav"));
    assert_eq!(output.microphone_outcome, MicrophoneCaptureOutcome::Saved);
}

#[test]
fn selected_system_audio_requires_the_product_wav_and_timing_sidecar() {
    let temp = tempfile::tempdir().unwrap();
    let root = CaptureRoot::for_project(temp.path()).unwrap();
    root.create_capture_dir("system-audio-projection").unwrap();
    let events = EventTrack {
        duration_ms: 100,
        screen_w: 640,
        screen_h: 360,
        monitors: Vec::new(),
        cursor: Vec::new(),
        clicks: Vec::new(),
        scrolls: Vec::new(),
        keys: Vec::new(),
        cursor_correlation: CursorCorrelation::default(),
    };
    let project = RecordingProject::new(
        "source.mp4",
        Settings {
            width: 640,
            height: 360,
            fps: 30.0,
            audio_rate: 48_000,
        },
        events,
    );
    std::fs::write(
        root.capture_file("system-audio-projection", "source.mp4")
            .unwrap(),
        b"video",
    )
    .unwrap();
    std::fs::write(
        root.capture_file("system-audio-projection", "system.wav")
            .unwrap(),
        b"audio",
    )
    .unwrap();
    std::fs::write(
        root.capture_file(
            "system-audio-projection",
            crate::screen_record::system_audio::SYSTEM_AUDIO_TIMING_FILE,
        )
        .unwrap(),
        serde_json::to_vec(&crate::screen_record::system_audio::SystemAudioTiming {
            schema: "shellx-cut/system-audio-timing/1".into(),
            first_packet_offset_ms: Some(0),
        })
        .unwrap(),
    )
    .unwrap();
    std::fs::write(
        root.capture_file("system-audio-projection", "project.json")
            .unwrap(),
        serde_json::to_vec(&project).unwrap(),
    )
    .unwrap();

    let output = PrivateFrameGridProjection::test_without_completed_audio_receipt(
        root,
        "system-audio-projection".into(),
        record_capture::SelectedCaptureStreams::new(false, true, false, false),
    )
    .capture_output()
    .unwrap();
    assert!(output.audio.is_none());
    assert_eq!(
        output.microphone_outcome,
        MicrophoneCaptureOutcome::NotRequested
    );
}
