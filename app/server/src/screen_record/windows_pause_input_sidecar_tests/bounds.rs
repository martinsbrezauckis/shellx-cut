use super::*;

fn persist(root: &CaptureRoot, journal: &RecordingSessionJournal) {
    let mut file =
        RecordingSessionJournalFile::create_new(root, CAPTURE, journal.intent().clone()).unwrap();
    for entry in journal.entries().into_iter().skip(1) {
        file.append_entry(entry).unwrap();
    }
}

#[test]
fn oversized_sidecar_fails_before_hashing_in_all_shared_verifiers() {
    let (_temp, root) = root();
    let evidence = evidence(run());
    let ready = readiness();
    let project = binding(&root);
    let owner = WindowsPauseInputSidecarOwner::new(root.clone(), CAPTURE.into(), project.clone());
    let pin = owner.publish_or_reopen(&evidence, &ready).unwrap();
    let journal = journal(evidence.run().clone(), pin.clone(), project);
    persist(&root, &journal);
    let path = root.capture_file(CAPTURE, &pin.file_name).unwrap();
    fs::OpenOptions::new()
        .write(true)
        .open(&path)
        .unwrap()
        .set_len(validation::MAX_INPUT_SIDECAR_BYTES + 1)
        .unwrap();
    let expected = "recording-input sidecar exceeds its byte limit";
    assert_eq!(
        validation::verify_pinned_run(&root, CAPTURE, evidence.run(), &ready, &pin).unwrap_err(),
        expected
    );
    assert_eq!(
        validation::verify_pinned_inputs(&root, CAPTURE, &journal).unwrap_err(),
        expected
    );
    assert_eq!(
        validation::verified_audio_sources(&root, CAPTURE, &journal).unwrap_err(),
        expected
    );
    assert_eq!(
        validation::verify_recovery_capture(&root, CAPTURE).unwrap_err(),
        expected
    );
    let scan =
        crate::screen_record::recovery::scan(root.cache_dir(), "missing-ffmpeg", "missing-ffprobe");
    assert_eq!(
        scan.failed_closed,
        [format!("{CAPTURE}: pause_session_ownership_unsafe")]
    );
    assert!(scan.recovered.is_empty() && scan.deferred.is_empty());
    let mut optional_intent = journal.intent().clone();
    optional_intent.input_sidecars_required = false;
    let optional = RecordingSessionJournal::new(optional_intent).unwrap();
    assert!(validation::verify_pinned_inputs(&root, CAPTURE, &optional)
        .unwrap()
        .is_empty());
    assert!(
        validation::verified_audio_sources(&root, CAPTURE, &optional)
            .unwrap()
            .is_empty()
    );
    assert!(path.is_file());
}

#[test]
fn reopen_comparison_rejects_bytes_beyond_exact_expected_length() {
    let (_temp, root) = root();
    let evidence = evidence(run());
    let owner = WindowsPauseInputSidecarOwner::new(root.clone(), CAPTURE.into(), binding(&root));
    let pin = owner.publish_or_reopen(&evidence, &readiness()).unwrap();
    let path = root.capture_file(CAPTURE, &pin.file_name).unwrap();
    let expected = fs::read(&path).unwrap();
    fs::OpenOptions::new()
        .write(true)
        .open(&path)
        .unwrap()
        .set_len(expected.len() as u64 + 1)
        .unwrap();
    assert_eq!(
        validation::existing_matches(&root, CAPTURE, &pin, &expected).unwrap_err(),
        "recording-input sidecar exceeds its byte limit"
    );
}

#[test]
fn long_checkpoint_run_retains_canonical_sidecar_recovery() {
    let (_temp, root) = root();
    let mut sealed = run();
    let template = sealed.fragments[0].clone();
    sealed.fragments = (0..5_000)
        .map(|sequence| {
            let mut fragment = template.clone();
            fragment.checkpoint_sequence = Some(sequence);
            fragment.stream_sequence = sequence;
            fragment.artifact = format!("checkpoints/segment-{sequence:06}.mp4");
            fragment.facts.start_offset_ms = sequence * 100;
            fragment.facts.end_offset_ms = (sequence + 1) * 100;
            fragment
        })
        .collect();
    sealed.checkpoints.last = 4_999;
    sealed.observed_end_ms = 500_000;
    sealed.logical_end_ms = 500_000;
    let mut sidecar = InputSidecar::new(
        CAPTURE,
        &binding(&root),
        &run(),
        evidence(run()).recording_input().unwrap(),
        &readiness(),
    )
    .unwrap();
    sidecar.body.run = sealed.clone();
    sidecar.body.clocks.raw_end_ms = 500_050;
    sidecar.body.clocks.session_observed_end_ms = 500_000;
    sidecar.body.clocks.logical_end_ms = 500_000;
    sidecar.fingerprint_sha256 = digest(&canonical(&sidecar.body).unwrap());
    let bytes = canonical(&sidecar).unwrap();
    assert!(bytes.len() > 1024 * 1024);
    assert!(bytes.len() as u64 <= validation::MAX_INPUT_SIDECAR_BYTES);
    let owner = WindowsPauseInputSidecarOwner::new(root.clone(), CAPTURE.into(), binding(&root));
    let mut pin = owner
        .publish_or_reopen(&evidence(run()), &readiness())
        .unwrap();
    pin.sha256 = digest(&bytes);
    pin.raw_end_ms = 500_050;
    fs::write(root.capture_file(CAPTURE, &pin.file_name).unwrap(), bytes).unwrap();
    let intent = RecordingSessionIntent::new(
        CAPTURE,
        1_000,
        100,
        30.0,
        None,
        false,
        "shellx-monitor-v1:windows:test",
        vec![RecordingStream::ScreenVideo],
    )
    .with_project_binding(binding(&root))
    .requiring_input_sidecars();
    let mut journal = RecordingSessionJournal::new(intent).unwrap();
    journal.append_transition(readiness()).unwrap();
    journal.append_input_sidecar(pin).unwrap();
    journal.seal_run(sealed).unwrap();
    persist(&root, &journal);
    validation::verify_recovery_capture(&root, CAPTURE).unwrap();
}
