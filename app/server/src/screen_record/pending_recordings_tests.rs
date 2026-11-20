use super::*;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(1);

fn capture_id() -> String {
    format!(
        "cap_{}_{}_{}",
        std::process::id(),
        super::super::active_capture::unix_ms_now(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    )
}

/// A project folder with one capture folder holding a journal and, when
/// `sound_bytes > 0`, an unfinished microphone file of that size.
fn project_with_capture(id: &str, sound_bytes: usize) -> (tempfile::TempDir, PathBuf) {
    let project = tempfile::tempdir().unwrap();
    let capture = project.path().join("cache").join("screen_record").join(id);
    std::fs::create_dir_all(&capture).unwrap();
    record_recovery::ManifestOwner::begin(&capture, record_recovery::CaptureStart::new(id, 15_000))
        .unwrap();
    if sound_bytes > 0 {
        let mut bytes = b"RIFF\0\0\0\0WAVEfmt \x10\0\0\0\x01\0\x02\0\x80\xbb\0\0\0\xee\x02\0\x04\0\x10\0data\0\0\0\0".to_vec();
        bytes.resize(bytes.len() + sound_bytes, 7);
        std::fs::write(capture.join(".mic-wav-0123.part"), bytes).unwrap();
    }
    (project, capture)
}

fn listed(id: &str) -> Option<Value> {
    all_notes()
        .iter()
        .filter(|note| note.capture_id == id)
        .find_map(describe)
}

#[test]
fn a_recording_that_never_finished_is_offered_at_the_next_start() {
    let id = capture_id();
    let (project, _capture) = project_with_capture(&id, 200_000);
    note_started(&id, project.path());
    let item = listed(&id).expect("an unfinished recording is listed");
    assert_eq!(item["situation"], json!("interrupted"));
    assert_eq!(item["can_recover"], json!(true));
    assert!(item["message"]
        .as_str()
        .unwrap()
        .contains("ShellX Cut closed while recording"));
    assert!(item["suggested_action"]
        .as_str()
        .unwrap()
        .contains("screen_record.recover"));
    remove(&id);
    assert!(listed(&id).is_none());
}

#[test]
fn an_interruption_with_nothing_saved_says_so_and_offers_no_recovery() {
    let id = capture_id();
    let (project, capture) = project_with_capture(&id, 0);
    note_started(&id, project.path());
    let item = listed(&id).unwrap();
    assert_eq!(item["situation"], json!("nothing_saved"));
    assert_eq!(item["can_recover"], json!(false));
    assert_eq!(item["files_folder"], json!(capture.display().to_string()));
    remove(&id);
}

#[test]
fn a_normal_stop_clears_the_note_and_a_stop_at_close_keeps_it() {
    let id = capture_id();
    let (project, capture) = project_with_capture(&id, 0);
    note_started(&id, project.path());
    after_stop_saved(&id, false);
    assert!(listed(&id).is_none());

    note_started(&id, project.path());
    std::fs::write(capture.join("mic.wav"), b"saved").unwrap();
    after_stop_saved(&id, true);
    let item = listed(&id).unwrap();
    assert_eq!(item["situation"], json!("saved_on_close"));
    assert_eq!(item["added_to_timeline"], json!(false));
    assert!(item["message"]
        .as_str()
        .unwrap()
        .contains("not on the timeline yet"));
    mark_added_to_timeline(&id);
    let item = listed(&id).unwrap();
    assert_eq!(item["added_to_timeline"], json!(true));
    assert_eq!(item["can_recover"], json!(false));
    assert!(item["message"]
        .as_str()
        .unwrap()
        .contains("added to the timeline"));
    remove(&id);
}

#[test]
fn a_failure_with_nothing_recorded_leaves_no_notice() {
    let id = capture_id();
    let (project, _capture) = project_with_capture(&id, 0);
    note_started(&id, project.path());
    after_failure(&id, true);
    assert!(listed(&id).is_none());
}

#[tokio::test]
async fn dismissing_forgets_the_notice_but_keeps_the_files() {
    let id = capture_id();
    let (project, capture) = project_with_capture(&id, 200_000);
    note_started(&id, project.path());
    let reply = dismiss_handler(json!({ "capture_id": id })).await.unwrap();
    assert_eq!(reply.result.unwrap()["dismissed"], json!(true));
    assert!(listed(&id).is_none());
    assert!(capture.join(".mic-wav-0123.part").is_file());
    let again = dismiss_handler(json!({ "capture_id": id }))
        .await
        .unwrap_err();
    assert_eq!(again.code, error_codes::NOT_FOUND);
}

#[tokio::test]
async fn a_recording_still_running_here_is_not_called_interrupted() {
    let _lock = super::super::capture_registry::capture_test_lock()
        .lock()
        .await;
    let id = capture_id();
    let (project, _capture) = project_with_capture(&id, 200_000);
    note_started(&id, project.path());
    let reservation = super::super::capture_registry::reserve_project_transition_test_capture(
        project.path(),
        id.clone(),
    );
    assert!(listed(&id).is_none());
    drop(reservation);
    assert!(listed(&id).is_some());
    remove(&id);
}

#[test]
fn the_capture_id_of_a_recording_file_is_its_folder_name() {
    let source = Path::new("/p/x.cutproj/cache/screen_record/cap_1_2_3/source.mp4");
    assert_eq!(capture_id_of_source(source).as_deref(), Some("cap_1_2_3"));
    assert_eq!(
        capture_id_of_source(Path::new("/p/x.cutproj/cache/a b/source.mp4")),
        None
    );
}

/// Row 182: frames ScreenCaptureKit wrote after an early Stop are not a take.
#[test]
fn an_early_stop_leftover_is_never_offered() {
    let id = capture_id();
    let (_project, capture) = project_with_capture(&id, 400_000);
    assert!(has_recoverable_media(&capture));
    mark_nothing_recorded(&capture);
    assert!(!has_recoverable_media(&capture));
}

#[test]
fn finished_checkpoints_without_a_receipt_do_not_claim_recovered_or_lost_time() {
    let id = capture_id();
    let (project, capture) = project_with_capture(&id, 40 * 192_000);
    let mut owner = record_recovery::ManifestOwner::open(&capture).unwrap();
    for index in 0..2 {
        let part = owner.begin_segment(index, index * 15_000).unwrap();
        std::fs::write(&part, b"checkpoint").unwrap();
        owner
            .publish(
                index,
                &part,
                record_recovery::CheckpointFacts {
                    start_ms: index * 15_000,
                    end_ms: (index + 1) * 15_000,
                    event_offset_ms: 0,
                    audio_offset_ms: None,
                },
                record_recovery::MediaFacts {
                    duration_ms: 15_000,
                    decoded_video_frames: 450,
                    has_audio: false,
                    width: Some(640),
                    height: Some(360),
                    codec_name: Some("h264".into()),
                    avg_frame_rate: None,
                    r_frame_rate: None,
                },
            )
            .unwrap();
    }
    drop(owner);
    assert!(record_recovery::read_manifest(&capture)
        .unwrap()
        .receipt
        .is_none());
    let note = Note {
        schema: SCHEMA.into(),
        capture_id: id,
        project_dir: project.path().into(),
        started_unix_ms: 0,
        outcome: Outcome::Recording,
        added_to_timeline: false,
    };
    for outcome in [Outcome::Recording, Outcome::Failed, Outcome::SavedOnClose] {
        let note = Note {
            outcome,
            ..note.clone()
        };
        let item = describe(&note).unwrap();
        assert_eq!(item["can_recover"], true);
        assert_eq!(item["checkpoints"], 2);
        let message = item["message"].as_str().unwrap();
        assert!(!message.contains("0 s"), "{message}");
        assert!(!message.contains("were not saved"), "{message}");
        assert!(message.contains("has not been measured yet"), "{message}");
        assert_eq!(item["recovered_extent_state"], "not_measured");
        for field in [
            "recovered_ms",
            "recorded_ms_at_least",
            "recorded_longer_unknown",
            "lost_tail_lower_bound_ms",
        ] {
            assert!(item[field].is_null(), "{field}: {}", item[field]);
        }
    }
    let sound = capture.join(".mic-wav-0123.part");
    let held_sound = capture.join("held-sound");
    std::fs::rename(&sound, &held_sound).unwrap();
    let video_only = describe(&note).unwrap();
    assert_eq!(video_only["can_recover"], true);
    assert_eq!(video_only["recovered_extent_state"], "not_measured");
    assert!(video_only["recovered_ms"].is_null());
    assert!(!video_only["message"]
        .as_str()
        .unwrap()
        .contains("were not saved"));
    std::fs::rename(&held_sound, &sound).unwrap();
    record_recovery::ManifestOwner::open(&capture)
        .unwrap()
        .publish_receipt(record_recovery::RecoveryReceipt {
            state: record_recovery::RecoveryState::Recovered,
            recovered_segments: 2,
            lost_tail_ms: None,
            lost_tail_lower_bound_ms: 0,
            lost_tail_upper_bound_ms: None,
            audio_first_packet_offset_ms: None,
            source: Some("recovered.mp4".into()),
            note: "saved checkpoints prepared".into(),
        })
        .unwrap();
    let item = describe(&note).unwrap();
    assert_eq!(item["recovered_extent_state"], "measured");
    assert_eq!(item["recovered_ms"], 30_000);
    assert_eq!(item["recorded_ms_at_least"], 40_000);
    assert_eq!(item["recorded_longer_unknown"], false);
    assert!(item["message"]
        .as_str()
        .unwrap()
        .contains("About 30 s of about 40 s can be recovered; the last 10 s were not saved"));
}
