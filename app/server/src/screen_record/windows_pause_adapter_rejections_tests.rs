use super::*;

#[test]
fn adapter_rejects_unverified_evidence_instead_of_synthesizing_it() {
    let (_commands, _command_rx, event_tx, event_rx) = channel();
    let mut translator = WindowsPauseEventTranslator::new(event_rx, Factory { reject: true });
    event_tx
        .send(WindowsPausePilotEvent::PauseSealed {
            generation: 1,
            epoch: 1,
            run: run(1, 0, 120),
            input: None,
            audio: Vec::new(),
            observed_at: Instant::now(),
        })
        .unwrap();
    assert!(matches!(
        translator.try_next(),
        Err(WindowsPauseAdapterError::EvidenceRejected)
    ));
}

#[test]
fn adapter_rejects_native_settings_that_do_not_match_the_accepted_range() {
    let (_commands, _command_rx, event_tx, event_rx) = channel();
    let mut translator = WindowsPauseEventTranslator::new(event_rx, Factory { reject: false });
    let mut native = run(44, 0, 120);
    native.accepted.range.width = 1;
    event_tx
        .send(WindowsPausePilotEvent::PauseSealed {
            generation: 1,
            epoch: 1,
            run: native,
            input: None,
            audio: Vec::new(),
            observed_at: Instant::now(),
        })
        .unwrap();
    assert!(matches!(
        translator.try_next(),
        Err(WindowsPauseAdapterError::EvidenceRejected)
    ));
}

#[test]
fn adapter_refuses_selected_input_until_its_sidecar_has_a_separate_verifier() {
    let (_commands, _command_rx, event_tx, event_rx) = channel();
    let mut translator = WindowsPauseEventTranslator::new(event_rx, Factory { reject: false });
    event_tx
        .send(WindowsPausePilotEvent::PauseSealed {
            generation: 1,
            epoch: 1,
            run: run(1, 0, 120),
            input: Some(record_capture::windows_pause_pilot::WindowsSealedInputRun {
                cursor: Vec::new(),
                clicks: Vec::new(),
                scrolls: Vec::new(),
                keys: Vec::new(),
            }),
            audio: Vec::new(),
            observed_at: Instant::now(),
        })
        .unwrap();
    assert!(matches!(
        translator.try_next(),
        Err(WindowsPauseAdapterError::EvidenceRejected)
    ));
}
