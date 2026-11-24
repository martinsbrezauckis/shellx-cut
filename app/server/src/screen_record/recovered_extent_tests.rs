use super::*;

#[test]
fn a_kill_after_one_checkpoint_says_what_was_recorded_and_what_was_lost() {
    // Killed at about 20 s: one 15 s checkpoint, the next one opened at
    // 15.4 s, sound written until 19.8 s (an installed run, killed at 20 s).
    let extent = measure_from(Some(15_000), 15_000, Some(15_444), Some(19_800));
    assert_eq!(extent.recovered_ms, 15_000);
    assert_eq!(extent.recorded_ms_at_least, 19_800);
    assert_eq!(extent.lost_tail_lower_bound_ms, 4_800);
    assert!(!extent.longer_unknown, "the sound covers the open part");
    assert_eq!(
        extent.sentence(),
        "Recovered 15 s of about 20 s. The last 5 s were not saved before the recording stopped. \
         The sound of those seconds stays in the recording's folder."
    );
}

#[test]
fn without_sound_an_open_part_is_not_reported_as_nothing_lost() {
    let extent = measure_from(Some(45_000), 45_000, Some(45_500), None);
    assert!(extent.longer_unknown);
    assert_eq!(
        extent.lost_tail_lower_bound_ms, 500,
        "only what the files prove"
    );
    assert_eq!(
        extent.sentence(),
        "Recovered 45 s. The recording went on after that, but nothing after 45 s was saved \
         before it stopped, and how long it ran is not known."
    );
}

#[test]
fn without_sound_a_late_open_part_still_gives_the_proved_loss() {
    // A rotation that took long (export load): the next part opened 4 s after
    // the kept end.
    let extent = measure_from(Some(15_000), 15_000, Some(19_200), None);
    assert_eq!(extent.lost_tail_lower_bound_ms, 4_200);
    assert_eq!(
        extent.sentence(),
        "Recovered 15 s of about 19 s. The last 4 s were not saved before the recording stopped. \
         It may have run longer."
    );
}

#[test]
fn a_take_whose_sound_ends_with_the_picture_lost_nothing() {
    let extent = measure_from(Some(30_000), 30_000, None, Some(30_400));
    assert_eq!(extent.lost_tail_lower_bound_ms, 400);
    assert_eq!(extent.sentence(), "Recovered 30 s, all that was recorded.");
}

#[test]
fn sound_only_counts_the_sound_as_recovered() {
    // Killed before the first video part was finished.
    let extent = measure_from(None, 0, Some(0), Some(2_000));
    assert_eq!(extent.recovered_ms, 2_000);
    assert_eq!(extent.lost_tail_lower_bound_ms, 0);
    assert!(!extent.sound_of_lost_kept);
    assert_eq!(extent.sentence(), "Recovered 2 s, all that was recorded.");
}

#[test]
fn a_dropped_checkpoint_counts_as_lost() {
    // Two checkpoints were finished but only the first could be recovered.
    let extent = measure_from(Some(15_000), 30_000, Some(30_400), None);
    assert_eq!(extent.recorded_ms_at_least, 30_400);
    assert_eq!(extent.lost_tail_lower_bound_ms, 15_400);
}

#[test]
fn durations_are_worded_in_whole_seconds_and_minutes() {
    assert_eq!(duration_words(0), "0 s");
    assert_eq!(duration_words(14_600), "15 s");
    assert_eq!(duration_words(60_000), "1 min");
    assert_eq!(duration_words(125_400), "2 min 5 s");
}

#[test]
fn sound_length_is_read_from_the_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("mic.wav");
    let spec = hound::WavSpec {
        channels: 2,
        sample_rate: 48_000,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut writer = hound::WavWriter::create(&path, spec).unwrap();
    for _ in 0..(48_000 * 3 / 2) * 2 {
        writer.write_sample(0i16).unwrap();
    }
    writer.finalize().unwrap();
    assert_eq!(sound_ms(&path), Some(1_500));
    assert_eq!(sound_ms(&dir.path().join("absent.wav")), None);
}

#[test]
fn nothing_measurable_says_nothing_rather_than_zero_seconds() {
    let extent = measure_from(Some(0), 0, None, None);
    assert_eq!(extent.sentence(), "");
}

#[test]
fn the_notice_says_before_recovery_what_can_be_kept_of_what_was_recorded() {
    let lost = measure_from(Some(15_000), 15_000, Some(15_444), Some(19_800));
    assert_eq!(
        lost.notice_sentence(),
        "About 15 s of about 20 s can be recovered; the last 5 s were not saved before the recording stopped."
    );
    let unknown = measure_from(Some(45_000), 45_000, Some(45_500), None);
    assert_eq!(
        unknown.notice_sentence(),
        "About 45 s can be recovered. The recording went on after that, but nothing after 45 s was saved before it stopped, and how long it ran is not known."
    );
    let all = measure_from(Some(30_000), 30_000, None, None);
    assert_eq!(
        all.notice_sentence(),
        "About 30 s can be recovered, all that was recorded."
    );
    assert_eq!(measure_from(None, 0, None, None).notice_sentence(), "");
}

/// A capture folder whose parts are `spans` on the checkpoint clock, taken
/// with Pause & resume (its pause-session journal is present).
fn paused_capture(spans: &[(u64, u64)], open_at: Option<u64>) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let mut owner = record_recovery::ManifestOwner::begin(
        dir.path(),
        record_recovery::CaptureStart::new("cap_paused_take", 1_000),
    )
    .unwrap();
    for (sequence, &(start_ms, end_ms)) in spans.iter().enumerate() {
        let sequence = sequence as u64;
        let staging = owner.begin_segment(sequence, start_ms).unwrap();
        std::fs::write(&staging, b"part").unwrap();
        owner
            .publish(
                sequence,
                &staging,
                record_recovery::CheckpointFacts {
                    start_ms,
                    end_ms,
                    event_offset_ms: start_ms,
                    audio_offset_ms: None,
                },
                record_recovery::MediaFacts {
                    duration_ms: end_ms - start_ms,
                    decoded_video_frames: 1,
                    has_audio: false,
                    width: Some(64),
                    height: Some(64),
                    codec_name: Some("h264".into()),
                    avg_frame_rate: None,
                    r_frame_rate: None,
                },
            )
            .unwrap();
    }
    match open_at {
        // Recovery kept every finished part of the interrupted take.
        Some(start_ms) => {
            owner.begin_segment(spans.len() as u64, start_ms).unwrap();
            owner
                .publish_receipt(record_recovery::RecoveryReceipt {
                    state: record_recovery::RecoveryState::Recovered,
                    recovered_segments: spans.len() as u64,
                    lost_tail_ms: None,
                    lost_tail_lower_bound_ms: 0,
                    lost_tail_upper_bound_ms: None,
                    audio_first_packet_offset_ms: None,
                    source: Some("recovered.mp4".into()),
                    note: "test".into(),
                })
                .unwrap();
        }
        None => owner
            .publish_receipt(record_recovery::RecoveryReceipt {
                state: record_recovery::RecoveryState::Complete,
                recovered_segments: spans.len() as u64,
                lost_tail_ms: Some(0),
                lost_tail_lower_bound_ms: 0,
                lost_tail_upper_bound_ms: Some(0),
                audio_first_packet_offset_ms: None,
                source: Some("source.mp4".into()),
                note: "test".into(),
            })
            .unwrap(),
    }
    drop(owner);
    std::fs::write(
        dir.path()
            .join(record_recovery::RECORDING_SESSION_JOURNAL_FILE),
        b"{}\n",
    )
    .unwrap();
    dir
}

#[test]
fn a_finished_take_with_a_pause_is_measured_in_recorded_time() {
    // Installed Mac take: 30 s limit, Pause at 10 s, Resume at 22 s; the
    // length limit ended it at 30 s of recorded time, 42 s after Start.
    let capture = paused_capture(&[(680, 10_100), (22_100, 42_680)], None);
    let extent = measure(capture.path(), true, &[]);
    assert_eq!(
        extent.recovered_ms, 30_000,
        "the 12 s pause is not recorded time"
    );
    assert_eq!(extent.recorded_ms_at_least, 30_000);
    assert_eq!(
        extent.notice_sentence(),
        "About 30 s can be recovered, all that was recorded."
    );
}

#[test]
fn a_pause_before_an_unfinished_part_is_not_reported_as_lost() {
    // The first run (9.4 s) was saved; after a 12 s pause the next part had
    // only begun when the app was closed. The pause is not lost picture.
    let capture = paused_capture(&[(0, 9_400)], Some(21_400));
    let extent = measure(capture.path(), true, &[]);
    assert_eq!(extent.recorded_ms_at_least, 9_400);
    assert_eq!(extent.lost_tail_lower_bound_ms, 0);
    assert!(
        extent.longer_unknown,
        "the unfinished part ran for an unknown time"
    );
}

#[test]
fn a_take_without_pause_keeps_its_capture_clock() {
    let dir = tempfile::tempdir().unwrap();
    let mut owner = record_recovery::ManifestOwner::begin(
        dir.path(),
        record_recovery::CaptureStart::new("cap_plain_take", 1_000),
    )
    .unwrap();
    let staging = owner.begin_segment(0, 400).unwrap();
    std::fs::write(&staging, b"part").unwrap();
    owner
        .publish(
            0,
            &staging,
            record_recovery::CheckpointFacts {
                start_ms: 400,
                end_ms: 20_000,
                event_offset_ms: 400,
                audio_offset_ms: None,
            },
            record_recovery::MediaFacts {
                duration_ms: 19_600,
                decoded_video_frames: 1,
                has_audio: false,
                width: Some(64),
                height: Some(64),
                codec_name: Some("h264".into()),
                avg_frame_rate: None,
                r_frame_rate: None,
            },
        )
        .unwrap();
    drop(owner);
    let extent = measure(dir.path(), true, &[]);
    assert_eq!(
        extent.recorded_ms_at_least, 20_000,
        "an ordinary take counts from the start of its capture clock, as before"
    );
}
