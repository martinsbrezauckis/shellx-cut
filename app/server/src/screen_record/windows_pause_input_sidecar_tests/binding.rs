use super::*;
use record_recovery::{
    CaptureRoot, RecordingInputSidecarPin, RecordingSessionIntent, RecordingSessionJournal,
    RecordingSessionJournalFile, RecordingStream,
};
use std::fs;

#[test]
fn copied_capture_tree_is_rejected_by_the_current_project_origin() {
    let (temp, root) = root();
    let evidence = evidence(run());
    let ready = readiness();
    let project = binding(&root);
    let owner = WindowsPauseInputSidecarOwner::new(root.clone(), CAPTURE.into(), project.clone());
    let pin = owner.publish_or_reopen(&evidence, &ready).unwrap();
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
    .with_project_binding(project)
    .requiring_input_sidecars();
    let mut journal = RecordingSessionJournalFile::create_new(&root, CAPTURE, intent).unwrap();
    journal.append_transition(ready).unwrap();
    journal
        .append_entry(record_recovery::RecordingSessionJournalEntry::InputSidecar(
            pin,
        ))
        .unwrap();
    journal.seal_run(evidence.run().clone()).unwrap();
    drop(journal);

    let other = cut_core::ProjectStore::create(temp.path(), "copied-sidecar", None).unwrap();
    let copied = CaptureRoot::for_project(&other.dir).unwrap();
    let destination = copied.create_capture_dir(CAPTURE).unwrap();
    let source = root.existing_capture_dir(CAPTURE).unwrap().unwrap();
    for entry in fs::read_dir(source).unwrap() {
        let entry = entry.unwrap();
        fs::copy(entry.path(), destination.join(entry.file_name())).unwrap();
    }
    assert!(validation::verify_recovery_capture(&copied, CAPTURE).is_err());
}

#[test]
fn accepted_revision_must_remain_in_the_current_durable_log() {
    let (_temp, root) = root();
    let evidence = evidence(run());
    let ready = readiness();
    let mut project = binding(&root);
    project.accepted_revision = "op_999999".into();
    let owner = WindowsPauseInputSidecarOwner::new(root.clone(), CAPTURE.into(), project.clone());
    let pin = owner.publish_or_reopen(&evidence, &ready).unwrap();
    assert!(validation::verify_pinned_inputs(
        &root,
        CAPTURE,
        &journal(evidence.run().clone(), pin, project),
    )
    .is_err());
}

#[test]
fn later_durable_project_revisions_do_not_invalidate_the_accepted_prefix() {
    let (_temp, root) = root();
    let binding = binding(&root);
    let project_dir = root
        .cache_dir()
        .parent()
        .and_then(|cache_dir| cache_dir.parent())
        .unwrap();
    let mut store = cut_core::ProjectStore::open(project_dir).unwrap();
    store
        .rename("private-sidecar-renamed", cut_core::Actor::system(), None)
        .unwrap();

    verify_project_binding(&root, &binding).unwrap();
}

#[test]
fn resumed_durable_clock_and_independent_native_clock_are_pinned_separately() {
    let (_temp, root) = root();
    let mut resumed = run();
    resumed.sequence = 1;
    resumed.observed_start_ms = 200;
    resumed.observed_end_ms = 300;
    resumed.logical_start_ms = 100;
    resumed.logical_end_ms = 200;
    let evidence = evidence_with_native_ready(resumed, 1_201);
    let readiness = DurableStateTransition {
        sequence: 2,
        state: RecordingSessionState::Resumed,
        logical_offset_ms: 100,
        observed_unix_ms: 1_200,
    };
    let owner = WindowsPauseInputSidecarOwner::new(root.clone(), CAPTURE.into(), binding(&root));

    let pin = owner.publish_or_reopen(&evidence, &readiness).unwrap();

    assert_eq!(pin.ready_unix_ms, 1_200);
    assert_eq!(pin.native_ready_unix_ms, 1_201);
    validation::verify_pinned_run(&root, CAPTURE, evidence.run(), &readiness, &pin).unwrap();
}

#[test]
fn native_ready_and_raw_clock_mutations_are_rejected_after_rehashing() {
    let (_temp, root) = root();
    let evidence = evidence(run());
    let ready = readiness();
    let project = binding(&root);
    let owner = WindowsPauseInputSidecarOwner::new(root.clone(), CAPTURE.into(), project.clone());
    let pin = owner.publish_or_reopen(&evidence, &ready).unwrap();
    let path = root.capture_file(CAPTURE, &pin.file_name).unwrap();
    let mut sidecar: InputSidecar = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    sidecar.body.clocks.ready_unix_ms += 1;
    sidecar.fingerprint_sha256 = digest(&canonical(&sidecar.body).unwrap());
    let bytes = canonical(&sidecar).unwrap();
    fs::write(&path, &bytes).unwrap();
    let pin = RecordingInputSidecarPin {
        sha256: digest(&bytes),
        ready_unix_ms: ready.observed_unix_ms + 1,
        ..pin
    };
    assert!(validation::verify_pinned_run(&root, CAPTURE, evidence.run(), &ready, &pin).is_err());

    let mut sidecar: InputSidecar = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    sidecar.body.clocks.ready_unix_ms = ready.observed_unix_ms;
    sidecar.body.clocks.native_ready_unix_ms += 1;
    sidecar.fingerprint_sha256 = digest(&canonical(&sidecar.body).unwrap());
    let bytes = canonical(&sidecar).unwrap();
    fs::write(&path, &bytes).unwrap();
    let pin = RecordingInputSidecarPin {
        sha256: digest(&bytes),
        ready_unix_ms: ready.observed_unix_ms,
        ..pin
    };
    assert!(validation::verify_pinned_run(&root, CAPTURE, evidence.run(), &ready, &pin).is_err());

    let mut sidecar: InputSidecar = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    sidecar.body.clocks.native_ready_unix_ms = ready.observed_unix_ms;
    sidecar.body.clocks.native_ready_raw_ms += 1;
    sidecar.fingerprint_sha256 = digest(&canonical(&sidecar.body).unwrap());
    let bytes = canonical(&sidecar).unwrap();
    fs::write(&path, &bytes).unwrap();
    let pin = RecordingInputSidecarPin {
        sha256: digest(&bytes),
        native_ready_raw_ms: pin.native_ready_raw_ms + 1,
        ..pin
    };
    assert!(validation::verify_pinned_run(&root, CAPTURE, evidence.run(), &ready, &pin).is_err());

    let mut sidecar: InputSidecar = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    sidecar.body.clocks.native_ready_raw_ms = sidecar.body.clocks.raw_start_ms;
    sidecar.body.clocks.raw_end_ms += 1;
    sidecar.fingerprint_sha256 = digest(&canonical(&sidecar.body).unwrap());
    let bytes = canonical(&sidecar).unwrap();
    fs::write(&path, &bytes).unwrap();
    let pin = RecordingInputSidecarPin {
        sha256: digest(&bytes),
        native_ready_raw_ms: sidecar.body.clocks.native_ready_raw_ms,
        raw_end_ms: pin.raw_end_ms + 1,
        ..pin
    };
    assert!(validation::verify_pinned_run(&root, CAPTURE, evidence.run(), &ready, &pin).is_err());
}

#[test]
fn journal_rejects_a_pin_for_another_readiness_transition() {
    let (_temp, root) = root();
    let evidence = evidence(run());
    let ready = readiness();
    let project = binding(&root);
    let owner = WindowsPauseInputSidecarOwner::new(root.clone(), CAPTURE.into(), project.clone());
    let mut pin = owner.publish_or_reopen(&evidence, &ready).unwrap();
    pin.ready_transition_sequence = 1;
    let mut journal = RecordingSessionJournal::new(
        RecordingSessionIntent::new(
            CAPTURE,
            1_000,
            100,
            30.0,
            None,
            false,
            "shellx-monitor-v1:windows:test",
            vec![RecordingStream::ScreenVideo],
        )
        .with_project_binding(project)
        .requiring_input_sidecars(),
    )
    .unwrap();
    journal.append_transition(ready).unwrap();
    assert!(journal.append_input_sidecar(pin).is_err());
}

#[cfg(unix)]
#[test]
fn linked_sidecar_and_project_log_are_rejected_without_following_the_target() {
    use std::os::unix::fs::symlink;

    let (temp, root) = root();
    let evidence = evidence(run());
    let ready = readiness();
    let owner = WindowsPauseInputSidecarOwner::new(root.clone(), CAPTURE.into(), binding(&root));
    let pin = owner.publish_or_reopen(&evidence, &ready).unwrap();
    let sentinel = temp.path().join("sidecar-sentinel.json");
    fs::write(&sentinel, b"outside remains untouched").unwrap();
    let path = root.capture_file(CAPTURE, &pin.file_name).unwrap();
    fs::remove_file(&path).unwrap();
    symlink(&sentinel, &path).unwrap();
    assert!(validation::verify_pinned_run(&root, CAPTURE, evidence.run(), &ready, &pin).is_err());
    assert_eq!(fs::read(&sentinel).unwrap(), b"outside remains untouched");

    let project_log = root
        .cache_dir()
        .parent()
        .and_then(|cache_dir| cache_dir.parent())
        .unwrap()
        .join("ops.jsonl");
    fs::remove_file(&project_log).unwrap();
    symlink(&sentinel, &project_log).unwrap();
    assert!(admit_project_binding(&root).is_err());
    assert_eq!(fs::read(sentinel).unwrap(), b"outside remains untouched");
}
