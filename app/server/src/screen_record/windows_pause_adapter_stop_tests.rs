use super::*;

#[test]
fn translator_rejects_a_terminal_from_the_wrong_stop_epoch() {
    let (commands, command_rx, event_tx, event_rx) = channel();
    let mut owner = owner(commands);
    let origin = Instant::now();
    owner
        .start_after_backend_origin(SessionTimeOrigin::observed(origin, 1_000))
        .unwrap();
    let request = owner.request_stop_at(at(origin, 100)).unwrap();
    assert!(matches!(
        command_rx.try_recv().unwrap(),
        Some(WindowsPausePilotCommand::Stop { epoch: 1 })
    ));
    let mut translator = WindowsPauseEventTranslator::new(event_rx, Factory { reject: false });
    translator.expect_stop(&request);
    send_stop(&event_tx, 9, run(44, 0, 120), at(origin, 120));
    assert!(matches!(
        translator.try_next(),
        Err(WindowsPauseAdapterError::UnexpectedStopEpoch)
    ));
    assert!(owner.journal().terminal().is_none());
}
