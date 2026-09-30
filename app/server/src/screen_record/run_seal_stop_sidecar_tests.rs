//! Clock and media facts from retained Mac run r1790754737235-676871cf5dbc.
use super::*;
use record_recovery::{
    RecordingInputSidecarPin, RecordingProjectBinding, RecordingProjectIdentity,
};

fn required_sidecars() -> RunSealCoordinator<MemoryJournalSink> {
    let streams = SelectedCaptureStreams::screen_only();
    let intent = intent_with_record_keys(&streams, false)
        .with_project_binding(RecordingProjectBinding {
            identity: RecordingProjectIdentity {
                schema: "shellx-cut/project-identity/1".into(),
                origin_path_sha256: format!("sha256:{}", "a".repeat(64)),
                project_name: "stop-sidecar-regression".into(),
            },
            accepted_revision: "op_000001".into(),
        })
        .requiring_input_sidecars();
    RunSealCoordinator::new(MemoryJournalSink::new(intent, None), streams).unwrap()
}

fn pin(
    run: u64,
    ready_sequence: u64,
    ready_ms: u64,
    raw_start: u64,
    raw_end: u64,
) -> RecordingInputSidecarPin {
    RecordingInputSidecarPin {
        run_sequence: run,
        file_name: format!("recording-input-run-{run:06}.json"),
        sha256: "a".repeat(64),
        ready_transition_sequence: ready_sequence,
        ready_unix_ms: 1_000 + ready_ms,
        native_ready_unix_ms: 1_000 + ready_ms,
        native_ready_raw_ms: raw_start,
        raw_start_ms: raw_start,
        raw_end_ms: raw_end,
    }
}

fn retained_run(generation: u64, sequence: u64) -> SealedRunEvidence {
    let streams = SelectedCaptureStreams::screen_only();
    let (observed_start, observed_end, logical_start, logical_end, duration, frames) =
        if sequence == 0 {
            (0, 681, 0, 470, 225, 6)
        } else {
            (1040, 1703, 470, 875, 192, 5)
        };
    let draft = evidence(
        generation,
        sequence,
        observed_start,
        observed_end,
        logical_start,
        logical_end,
        &streams,
    );
    let mut run = draft.run().clone();
    run.fragments[0].facts.media_duration_ms = duration;
    run.fragments[0].facts.decoded_video_frames = Some(frames);
    SealedRunEvidence::new(
        generation,
        run,
        draft.legacy_projection_run(),
        streams.streams().to_vec(),
    )
}

#[test]
fn resumed_native_stop_pins_its_required_sidecar_before_the_run_and_terminal() {
    let origin = Instant::now();
    let mut coordinator = required_sidecars();
    start(&mut coordinator, origin);
    let pause = coordinator.request_pause_at(at(origin, 400)).unwrap();
    let first = retained_run(pause.boundary().generation, 0);
    coordinator
        .pin_recording_input_sidecar(&first, pin(0, 0, 0, 16, 486))
        .unwrap();
    coordinator.seal_pause_at(first, at(origin, 681)).unwrap();
    let resume = coordinator.request_resume_at(at(origin, 900)).unwrap();
    coordinator
        .seal_resume_at(
            ResumeReadinessEvidence::new(resume.generation, vec![RecordingStream::ScreenVideo]),
            at(origin, 1040),
        )
        .unwrap();
    let stop = coordinator.request_stop_at(at(origin, 1400)).unwrap();
    assert!(coordinator.phase().is_terminal());
    let final_run = retained_run(stop.generation().unwrap(), 1);
    let final_pin = pin(1, 2, 1040, 1081, 1486);
    assert!(coordinator
        .seal_stop_at(
            Some(retained_run(stop.generation().unwrap(), 1)),
            TerminalDisposition::Completed,
            at(origin, 1703),
        )
        .is_err());
    assert_eq!(coordinator.journal_sink().journal().sealed_runs().len(), 1);
    assert!(coordinator.journal_sink().journal().terminal().is_none());
    // Terminal timestamp issuance does not admit another generation or start.
    assert!(coordinator
        .pin_recording_input_sidecar(&retained_run(99, 1), final_pin.clone())
        .is_err());
    let mut wrong_start = final_run.run().clone();
    wrong_start.observed_start_ms += 1;
    let wrong_start = SealedRunEvidence::new(
        stop.generation().unwrap(),
        wrong_start,
        final_run.legacy_projection_run(),
        vec![RecordingStream::ScreenVideo],
    );
    assert!(coordinator
        .pin_recording_input_sidecar(&wrong_start, final_pin.clone())
        .is_err());
    coordinator
        .pin_recording_input_sidecar(&final_run, final_pin.clone())
        .unwrap();
    assert!(coordinator
        .pin_recording_input_sidecar(&final_run, final_pin.clone())
        .is_err());
    coordinator
        .seal_stop_at(
            Some(final_run),
            TerminalDisposition::Completed,
            at(origin, 1703),
        )
        .unwrap();
    let journal = coordinator.journal_sink().journal();
    assert_eq!(journal.sealed_runs().len(), 2);
    assert_eq!(journal.terminal().unwrap().logical_end_ms, 875);
    assert!(RecordingSessionJournal::replay(journal.entries()).is_ok());
    assert!(coordinator
        .pin_recording_input_sidecar(&retained_run(2, 1), final_pin)
        .is_err());
}

#[test]
fn stop_without_an_active_run_never_admits_a_sidecar() {
    let origin = Instant::now();
    let mut coordinator = required_sidecars();
    start(&mut coordinator, origin);
    let stop = coordinator
        .request_stop_at(origin + Duration::from_micros(500))
        .unwrap();
    assert!(!stop.requires_sealed_run());
    assert!(coordinator
        .pin_recording_input_sidecar(&retained_run(0, 0), pin(0, 0, 0, 16, 486))
        .is_err());
    coordinator
        .seal_stop_at(None, TerminalDisposition::Completed, at(origin, 1))
        .unwrap();
}
