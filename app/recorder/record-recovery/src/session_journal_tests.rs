use record_core::{CaptureCadence, FrameRate, ProbedMediaCadence};

use crate::{
    plan_run_aware_stitch, CheckpointSequenceRange, DurableStateTransition, RecordingSessionIntent,
    RecordingSessionJournal, RecordingSessionJournalEntry, RecordingSessionState, RecordingStream,
    RunAwareStitchSpan, SealedRun, SessionTerminal, StreamFragment, StreamFragmentFacts,
    TerminalDisposition,
};

fn sha256(seed: char) -> String {
    std::iter::repeat_n(seed, 64).collect()
}

fn video(
    checkpoint_sequence: u64,
    stream_sequence: u64,
    start_offset_ms: u64,
    end_offset_ms: u64,
    media_duration_ms: u64,
    seed: char,
) -> StreamFragment {
    StreamFragment {
        stream: RecordingStream::ScreenVideo,
        checkpoint_sequence: Some(checkpoint_sequence),
        stream_sequence,
        artifact: format!("screen/video-{checkpoint_sequence}.mp4"),
        bytes: 1,
        sha256: sha256(seed),
        facts: StreamFragmentFacts {
            start_offset_ms,
            end_offset_ms,
            media_duration_ms,
            decoded_video_frames: Some(1),
            avg_frame_rate: None,
            r_frame_rate: None,
        },
    }
}

fn microphone(
    stream_sequence: u64,
    start_offset_ms: u64,
    end_offset_ms: u64,
    media_duration_ms: u64,
    seed: char,
) -> StreamFragment {
    StreamFragment {
        stream: RecordingStream::MicrophoneAudio,
        checkpoint_sequence: None,
        stream_sequence,
        artifact: format!("microphone/audio-{stream_sequence}.m4a"),
        bytes: 1,
        sha256: sha256(seed),
        facts: StreamFragmentFacts {
            start_offset_ms,
            end_offset_ms,
            media_duration_ms,
            decoded_video_frames: None,
            avg_frame_rate: None,
            r_frame_rate: None,
        },
    }
}

fn intent() -> RecordingSessionIntent {
    RecordingSessionIntent::new(
        "session-1",
        100,
        100,
        30.0,
        Some(60_000),
        true,
        "opaque-target-1",
        vec![RecordingStream::ScreenVideo],
    )
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

fn sealed_run(
    sequence: u64,
    observed_start_ms: u64,
    observed_end_ms: u64,
    logical_start_ms: u64,
    logical_end_ms: u64,
    first_checkpoint: u64,
    last_checkpoint: u64,
    fragments: Vec<StreamFragment>,
) -> SealedRun {
    SealedRun {
        sequence,
        observed_start_ms,
        observed_end_ms,
        logical_start_ms,
        logical_end_ms,
        checkpoints: CheckpointSequenceRange {
            first: first_checkpoint,
            last: last_checkpoint,
        },
        fragments,
    }
}

fn complete_two_run_journal() -> RecordingSessionJournal {
    let mut journal = RecordingSessionJournal::new(intent()).unwrap();
    journal
        .append_transition(transition(0, RecordingSessionState::Started, 0, 100))
        .unwrap();
    journal
        .seal_run(sealed_run(
            0,
            0,
            400,
            0,
            400,
            0,
            1,
            vec![
                video(0, 0, 0, 100, 100, 'a'),
                video(1, 1, 250, 400, 100, 'b'),
            ],
        ))
        .unwrap();
    journal
        .append_transition(transition(1, RecordingSessionState::Paused, 400, 500))
        .unwrap();
    journal
        .append_transition(transition(2, RecordingSessionState::Resumed, 400, 1_300))
        .unwrap();
    journal
        .seal_run(sealed_run(
            1,
            1_400,
            1_500,
            400,
            500,
            2,
            2,
            vec![video(2, 0, 0, 100, 100, 'c')],
        ))
        .unwrap();
    journal
        .append_transition(transition(3, RecordingSessionState::Stopping, 500, 1_600))
        .unwrap();
    journal
        .seal_terminal(SessionTerminal {
            disposition: TerminalDisposition::Completed,
            logical_end_ms: 500,
            observed_unix_ms: 1_700,
        })
        .unwrap();
    journal
}

#[test]
fn run_aware_plan_pads_encoder_gaps_but_compacts_pause_between_sealed_runs() {
    let journal = complete_two_run_journal();
    let plan = plan_run_aware_stitch(&journal).unwrap();

    assert_eq!(plan.duration_ms, 500, "the 1s pause is not source time");
    assert_eq!(
        plan.spans,
        vec![
            RunAwareStitchSpan::Source {
                run_sequence: 0,
                checkpoint_sequence: 0,
                artifact: "screen/video-0.mp4".into(),
                sha256: sha256('a'),
                logical_offset_ms: 0,
                source_duration_ms: 100,
            },
            RunAwareStitchSpan::EncoderGapPadding {
                run_sequence: 0,
                logical_start_ms: 100,
                logical_end_ms: 250,
            },
            RunAwareStitchSpan::Source {
                run_sequence: 0,
                checkpoint_sequence: 1,
                artifact: "screen/video-1.mp4".into(),
                sha256: sha256('b'),
                logical_offset_ms: 250,
                source_duration_ms: 100,
            },
            RunAwareStitchSpan::EncoderGapPadding {
                run_sequence: 0,
                logical_start_ms: 350,
                logical_end_ms: 400,
            },
            RunAwareStitchSpan::Source {
                run_sequence: 1,
                checkpoint_sequence: 2,
                artifact: "screen/video-2.mp4".into(),
                sha256: sha256('c'),
                logical_offset_ms: 400,
                source_duration_ms: 100,
            },
        ]
    );
    assert!(plan.spans.iter().all(|span| {
        !matches!(
            span,
            RunAwareStitchSpan::EncoderGapPadding {
                logical_start_ms: 400,
                ..
            }
        )
    }));
}

#[test]
fn serializable_entries_replay_with_immutable_intent_and_terminal_disposition() {
    let mut entries = complete_two_run_journal().entries();
    let RecordingSessionJournalEntry::Intent(intent) = &mut entries[0] else {
        panic!("fixture entry ordering changed");
    };
    intent.fps = 47.5;
    let journal = RecordingSessionJournal::replay(entries).unwrap();
    let entries = journal.entries();
    assert!(matches!(
        entries.first(),
        Some(RecordingSessionJournalEntry::Intent(_))
    ));
    assert!(matches!(
        entries.last(),
        Some(RecordingSessionJournalEntry::Terminal(SessionTerminal {
            disposition: TerminalDisposition::Completed,
            ..
        }))
    ));
    let jsonl = entries
        .iter()
        .map(serde_json::to_string)
        .collect::<Result<Vec<_>, _>>()
        .unwrap()
        .join("\n");
    let replayed_entries = jsonl
        .lines()
        .map(serde_json::from_str)
        .collect::<Result<Vec<RecordingSessionJournalEntry>, _>>()
        .unwrap();
    let replayed = RecordingSessionJournal::replay(replayed_entries).unwrap();
    assert_eq!(replayed, journal);
    assert_eq!(replayed.intent().fps, 47.5);
}

#[test]
fn replays_integer_json_fps_as_its_exact_f64_equivalent() {
    let entry = serde_json::from_str::<RecordingSessionJournalEntry>(
        r#"{
            "kind":"intent",
            "schema":"shellx-cut/recording-session-journal/1",
            "session_id":"integer-json-fps",
            "created_unix_ms":100,
            "checkpoint_interval_ms":100,
            "fps":30,
            "record_keys":false,
            "target_descriptor":"opaque-target-integer-json",
            "requested_streams":["screen_video"]
        }"#,
    )
    .unwrap();

    let journal = RecordingSessionJournal::replay([entry]).unwrap();
    assert_eq!(journal.intent().fps, 30.0);
}

#[test]
fn rejects_nonfinite_and_out_of_range_requested_fps() {
    for fps in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, 0.5, 240.5] {
        let mut invalid_intent = intent();
        invalid_intent.fps = fps;
        assert!(
            RecordingSessionJournal::new(invalid_intent).is_err(),
            "{fps:?}"
        );
    }
}

#[test]
fn rejects_drifting_or_final_media_cadence_in_immutable_intent() {
    let mut drifting = intent();
    let mut cadence = CaptureCadence::from_server_fps(30.0).unwrap();
    cadence.requested = FrameRate::new(25, 1).unwrap();
    drifting.capture_cadence = Some(cadence);
    assert!(RecordingSessionJournal::new(drifting).is_err());

    let mut final_media = intent();
    final_media.capture_cadence = Some(
        CaptureCadence::from_server_fps(30.0)
            .unwrap()
            .with_probed_media(ProbedMediaCadence {
                avg_frame_rate: Some(FrameRate::new(30, 1).unwrap()),
                r_frame_rate: Some(FrameRate::new(30, 1).unwrap()),
                decoded_video_frames: Some(3),
                duration_ms: Some(100),
            }),
    );
    assert!(RecordingSessionJournal::new(final_media).is_err());
}

#[test]
fn non_screen_streams_use_independent_sequences_and_share_the_sealed_run_interval() {
    let multi_stream_intent = RecordingSessionIntent::new(
        "session-2",
        100,
        100,
        60.0,
        None,
        false,
        "opaque-target-2",
        vec![
            RecordingStream::ScreenVideo,
            RecordingStream::MicrophoneAudio,
        ],
    );
    let mut valid = RecordingSessionJournal::new(multi_stream_intent.clone()).unwrap();
    valid
        .append_transition(transition(0, RecordingSessionState::Started, 0, 100))
        .unwrap();
    valid
        .seal_run(sealed_run(
            0,
            0,
            100,
            0,
            100,
            0,
            0,
            vec![
                video(0, 0, 0, 100, 100, 'a'),
                microphone(0, 0, 100, 100, 'b'),
            ],
        ))
        .unwrap();

    let mut forced_checkpoint = microphone(0, 0, 100, 100, 'b');
    forced_checkpoint.checkpoint_sequence = Some(0);
    let mut invalid_checkpoint = RecordingSessionJournal::new(multi_stream_intent.clone()).unwrap();
    invalid_checkpoint
        .append_transition(transition(0, RecordingSessionState::Started, 0, 100))
        .unwrap();
    assert!(invalid_checkpoint
        .seal_run(sealed_run(
            0,
            0,
            100,
            0,
            100,
            0,
            0,
            vec![video(0, 0, 0, 100, 100, 'a'), forced_checkpoint],
        ))
        .is_err());

    let mut outside_interval = microphone(0, 0, 100, 100, 'b');
    outside_interval.facts.end_offset_ms = 101;
    let mut invalid_interval = RecordingSessionJournal::new(multi_stream_intent).unwrap();
    invalid_interval
        .append_transition(transition(0, RecordingSessionState::Started, 0, 100))
        .unwrap();
    assert!(invalid_interval
        .seal_run(sealed_run(
            0,
            0,
            100,
            0,
            100,
            0,
            0,
            vec![video(0, 0, 0, 100, 100, 'a'), outside_interval],
        ))
        .is_err());
}

#[test]
fn rejects_unstitchable_intents_zero_frame_video_and_nonportable_artifact_paths() {
    assert!(RecordingSessionJournal::new(RecordingSessionIntent::new(
        "audio-only",
        100,
        100,
        30.0,
        None,
        false,
        "opaque-target-audio-only",
        vec![RecordingStream::MicrophoneAudio],
    ))
    .is_err());

    let mut zero_frames = video(0, 0, 0, 100, 100, 'a');
    zero_frames.facts.decoded_video_frames = Some(0);
    let mut invalid_frames = RecordingSessionJournal::new(intent()).unwrap();
    invalid_frames
        .append_transition(transition(0, RecordingSessionState::Started, 0, 100))
        .unwrap();
    assert!(invalid_frames
        .seal_run(sealed_run(0, 0, 100, 0, 100, 0, 0, vec![zero_frames],))
        .is_err());

    for artifact in [
        "/absolute.mp4",
        "../escape.mp4",
        "screen//empty-segment.mp4",
        "screen\\windows-separator.mp4",
        "C:drive-prefix.mp4",
        "./dot.mp4",
    ] {
        let mut fragment = video(0, 0, 0, 100, 100, 'a');
        fragment.artifact = artifact.into();
        let mut invalid_path = RecordingSessionJournal::new(intent()).unwrap();
        invalid_path
            .append_transition(transition(0, RecordingSessionState::Started, 0, 100))
            .unwrap();
        assert!(invalid_path
            .seal_run(sealed_run(0, 0, 100, 0, 100, 0, 0, vec![fragment],))
            .is_err());
    }
}

#[test]
fn replay_rejects_video_probe_rates_on_non_video_fragments() {
    let multi_stream_intent = RecordingSessionIntent::new(
        "session-rate-on-audio",
        100,
        100,
        30.0,
        None,
        false,
        "opaque-target-rate-on-audio",
        vec![
            RecordingStream::ScreenVideo,
            RecordingStream::MicrophoneAudio,
        ],
    );
    for use_avg_rate in [true, false] {
        let mut audio = microphone(0, 0, 100, 100, 'b');
        if use_avg_rate {
            audio.facts.avg_frame_rate = Some(FrameRate::new(30, 1).unwrap());
        } else {
            audio.facts.r_frame_rate = Some(FrameRate::new(30, 1).unwrap());
        }
        let entries = vec![
            RecordingSessionJournalEntry::Intent(Box::new(multi_stream_intent.clone())),
            RecordingSessionJournalEntry::Transition(transition(
                0,
                RecordingSessionState::Started,
                0,
                100,
            )),
            RecordingSessionJournalEntry::Run(sealed_run(
                0,
                0,
                100,
                0,
                100,
                0,
                0,
                vec![video(0, 0, 0, 100, 100, 'a'), audio],
            )),
        ];
        assert!(RecordingSessionJournal::replay(entries).is_err());
    }
}

#[test]
fn rejects_malformed_out_of_order_overlapping_and_non_monotonic_facts() {
    let mut malformed = RecordingSessionJournal::new(intent()).unwrap();
    malformed
        .append_transition(transition(0, RecordingSessionState::Started, 0, 100))
        .unwrap();
    let mut bad_hash = video(0, 0, 0, 100, 100, 'a');
    bad_hash.sha256 = "bad".into();
    assert!(malformed
        .seal_run(sealed_run(0, 0, 100, 0, 100, 0, 0, vec![bad_hash]))
        .is_err());

    let mut overlap = RecordingSessionJournal::new(intent()).unwrap();
    overlap
        .append_transition(transition(0, RecordingSessionState::Started, 0, 100))
        .unwrap();
    assert!(overlap
        .seal_run(sealed_run(
            0,
            0,
            300,
            0,
            300,
            0,
            1,
            vec![
                video(0, 0, 0, 200, 100, 'a'),
                video(1, 1, 150, 300, 100, 'b'),
            ],
        ))
        .is_err());

    let entries = complete_two_run_journal().entries();
    let mut out_of_order = entries.clone();
    out_of_order.swap(3, 4);
    assert!(RecordingSessionJournal::replay(out_of_order).is_err());

    let mut non_monotonic = entries;
    let RecordingSessionJournalEntry::Transition(resume) = &mut non_monotonic[4] else {
        panic!("fixture entry ordering changed")
    };
    resume.observed_unix_ms = 499;
    assert!(RecordingSessionJournal::replay(non_monotonic).is_err());
}
