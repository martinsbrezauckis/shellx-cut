use super::*;
use crate::screen_record::pause_session_owner::PauseSessionFactOutcome;
use crate::screen_record::pause_worker_protocol::PauseWorkerFact;

#[test]
fn pause_fact_and_evidence_arrive_together_but_owner_keeps_durable_order() {
    let (commands, command_rx, event_tx, event_rx) = channel();
    let mut owner = owner(commands);
    let mut translator = WindowsPauseEventTranslator::new(event_rx, Factory { reject: false });
    let origin = Instant::now();
    owner
        .start_after_backend_origin(SessionTimeOrigin::observed(origin, 1_000))
        .unwrap();
    owner.request_pause_at(at(origin, 100)).unwrap();
    assert!(matches!(
        command_rx.try_recv().unwrap(),
        Some(WindowsPausePilotCommand::Pause {
            generation: 1,
            epoch: 1
        })
    ));
    event_tx
        .send(WindowsPausePilotEvent::PauseSealed {
            generation: 1,
            epoch: 1,
            run: run(44, 0, 120),
            input: None,
            audio: Vec::new(),
            observed_at: at(origin, 120),
        })
        .unwrap();
    let WindowsPauseAdapterEvent::PauseSealed { fact, evidence, .. } =
        translator.try_next().unwrap().unwrap()
    else {
        panic!("translated pause seal required")
    };
    assert_eq!(
        owner.accept_worker_fact(fact, at(origin, 120)).unwrap(),
        PauseSessionFactOutcome::PauseFactsComplete { generation: 1 }
    );
    owner.seal_pause_at(evidence, at(origin, 121)).unwrap();
    assert_eq!(owner.journal().sealed_runs().len(), 1);
    assert_eq!(
        owner.journal().transitions().last().unwrap().state,
        RecordingSessionState::Paused
    );
}

fn sealed_audio(
    stream: RecordingStream,
) -> record_capture::windows_pause_pilot::WindowsSealedAudioRun {
    let artifact = match stream {
        RecordingStream::MicrophoneAudio => {
            "recording-microphone-generation-00000000000000000001.wav"
        }
        RecordingStream::SystemAudio => "recording-system-generation-00000000000000000001.wav",
        _ => panic!("test requires an admitted audio stream"),
    };
    record_capture::windows_pause_pilot::WindowsSealedAudioRun {
        stream,
        source_generation: 1,
        artifact: artifact.into(),
        bytes: 48,
        sha256: "a".repeat(64),
        media_duration_ms: 120,
        native_ready_unix_ms: 1_000,
        native_ready_raw_ms: 1,
        raw_start_ms: 0,
        raw_end_ms: 120,
    }
}

#[test]
fn translation_requires_every_selected_audio_owner_before_the_durable_owner_can_complete() {
    let (commands, command_rx, event_tx, event_rx) = channel();
    let streams = SelectedCaptureStreams::new(true, true, false, false);
    let mut owner = owner_with_streams(commands, streams.clone());
    let origin = Instant::now();
    owner
        .start_after_backend_origin(SessionTimeOrigin::observed(origin, 1_000))
        .unwrap();
    owner.request_pause_at(at(origin, 10)).unwrap();
    assert!(matches!(
        command_rx.try_recv().unwrap(),
        Some(WindowsPausePilotCommand::Pause { generation: 1, .. })
    ));
    let mut translator =
        WindowsPauseEventTranslator::with_streams(event_rx, Factory { reject: false }, streams);
    event_tx
        .send(WindowsPausePilotEvent::PauseSealed {
            generation: 1,
            epoch: 1,
            run: run(1, 0, 120),
            input: None,
            audio: vec![
                sealed_audio(RecordingStream::MicrophoneAudio),
                sealed_audio(RecordingStream::SystemAudio),
            ],
            // The epoch-1 pause command was issued at +10 ms. Every generated
            // selected-stream fact must witness the native seal after that
            // command, so keep the event timestamp aligned with the durable
            // owner acceptance below.
            observed_at: at(origin, 120),
        })
        .unwrap();
    let WindowsPauseAdapterEvent::PauseSealed {
        fact,
        additional_facts,
        ..
    } = translator.try_next().unwrap().unwrap()
    else {
        panic!("every selected audio owner must produce one pause fact")
    };
    assert!(matches!(
        fact,
        PauseWorkerFact::PauseSealed {
            stream: RecordingStream::ScreenVideo,
            ..
        }
    ));
    assert_eq!(
        owner.accept_worker_fact(fact, at(origin, 120)).unwrap(),
        PauseSessionFactOutcome::AwaitingWorkerFacts,
        "screen alone can never make the selected audio pause durable"
    );
    assert_eq!(
        additional_facts
            .iter()
            .map(|fact| match fact {
                PauseWorkerFact::PauseSealed { stream, .. } => *stream,
                _ => panic!("audio pause facts must be sealed"),
            })
            .collect::<Vec<_>>(),
        vec![
            RecordingStream::MicrophoneAudio,
            RecordingStream::SystemAudio
        ]
    );
    assert_eq!(
        owner
            .accept_worker_fact(additional_facts[0], at(origin, 120))
            .unwrap(),
        PauseSessionFactOutcome::AwaitingWorkerFacts
    );
    assert_eq!(
        owner
            .accept_worker_fact(additional_facts[1], at(origin, 120))
            .unwrap(),
        PauseSessionFactOutcome::PauseFactsComplete { generation: 1 }
    );
}

#[test]
fn translation_rejects_a_missing_or_duplicate_selected_audio_owner() {
    let (_commands, _command_rx, event_tx, event_rx) = channel();
    let mut translator = WindowsPauseEventTranslator::with_streams(
        event_rx,
        Factory { reject: false },
        SelectedCaptureStreams::new(true, false, false, false),
    );
    event_tx
        .send(WindowsPausePilotEvent::PauseSealed {
            generation: 1,
            epoch: 1,
            run: run(1, 0, 120),
            input: None,
            audio: vec![
                sealed_audio(RecordingStream::MicrophoneAudio),
                sealed_audio(RecordingStream::MicrophoneAudio),
            ],
            observed_at: Instant::now(),
        })
        .unwrap();
    assert!(matches!(
        translator.try_next(),
        Err(WindowsPauseAdapterError::EvidenceRejected)
    ));
}
