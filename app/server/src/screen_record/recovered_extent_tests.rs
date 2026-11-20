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
