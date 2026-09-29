use super::{
    plan_placement, publication, read_timing, timing_path, SystemAudioPlacement, SystemAudioTiming,
};
use cut_core::error_codes;

fn timing(offset_ms: Option<u64>) -> SystemAudioTiming {
    SystemAudioTiming {
        schema: "shellx-cut/system-audio-timing/1".into(),
        first_packet_offset_ms: offset_ms,
    }
}

#[test]
fn delayed_packet_offsets_create_honest_shorter_placements() {
    for (offset, expected_at, expected_duration) in
        [(0, 100, 3_000), (37, 137, 2_963), (2_350, 2_450, 650)]
    {
        assert_eq!(
            plan_placement(100, Some(3_000), Some(5_000), Some(&timing(Some(offset)))),
            SystemAudioPlacement::Insert {
                at_ms: expected_at,
                source_duration_ms: expected_duration,
            }
        );
    }
}

#[test]
fn absent_sidecar_keeps_legacy_zero_offset() {
    assert_eq!(
        plan_placement(100, Some(3_000), Some(5_000), None),
        SystemAudioPlacement::Insert {
            at_ms: 100,
            source_duration_ms: 3_000,
        }
    );
}

#[test]
fn no_packet_sidecar_skips_empty_system_clip() {
    assert!(matches!(
        plan_placement(0, Some(3_000), Some(5_000), Some(&timing(None))),
        SystemAudioPlacement::Skip { .. }
    ));
}

#[test]
fn missing_sidecar_is_backward_compatible() {
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(read_timing(dir.path()).unwrap(), None);
    assert_eq!(
        timing_path(dir.path()),
        dir.path().join("system-audio.json")
    );
}

#[test]
fn pending_current_capture_never_falls_back_to_legacy_zero_offset() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("system.wav"), b"current-format-wav").unwrap();
    std::fs::write(publication::pending_timing_path(dir.path()), b"pending").unwrap();

    let error = read_timing(dir.path()).unwrap_err();
    assert_eq!(error.code, error_codes::INVALID_ARGS);
    assert!(error.message.contains("incomplete"));
}

#[test]
fn timing_sidecar_round_trips_the_packet_offset_contract() {
    let dir = tempfile::tempdir().unwrap();
    let expected = timing(Some(2_350));
    publication::begin_timing_publication(dir.path()).unwrap();
    assert!(publication::timing_publication_is_pending(dir.path()).unwrap());
    publication::write_timing(dir.path(), &expected).unwrap();
    assert!(publication::timing_publication_is_pending(dir.path()).unwrap());
    publication::clear_timing_publication(dir.path()).unwrap();
    assert_eq!(read_timing(dir.path()).unwrap(), Some(expected));
    assert!(!publication::pending_timing_path(dir.path()).exists());
    assert!(!dir.path().join("system-audio.json.pending.part").exists());
}

#[test]
fn oversized_timing_sidecar_is_rejected_before_json_parse() {
    let dir = tempfile::tempdir().unwrap();
    let path = timing_path(dir.path());
    std::fs::File::create(&path)
        .unwrap()
        .set_len(super::MAX_SYSTEM_AUDIO_TIMING_BYTES + 1)
        .unwrap();
    let error = read_timing(dir.path()).unwrap_err();
    assert_eq!(error.code, error_codes::INVALID_ARGS);
    assert!(error.message.contains("64 KiB limit"));
}

#[test]
fn timing_sidecar_at_byte_limit_still_loads() {
    let dir = tempfile::tempdir().unwrap();
    let mut bytes = serde_json::to_vec(&timing(Some(37))).unwrap();
    bytes.resize(super::MAX_SYSTEM_AUDIO_TIMING_BYTES as usize, b' ');
    std::fs::write(timing_path(dir.path()), bytes).unwrap();
    assert_eq!(read_timing(dir.path()).unwrap(), Some(timing(Some(37))));
}

#[cfg(unix)]
#[test]
fn timing_publication_never_follows_pending_or_legacy_part_links() {
    use std::os::unix::fs::symlink;

    let dir = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let pending_target = outside.path().join("pending-target");
    let part_target = outside.path().join("part-target");
    std::fs::write(&pending_target, b"pending outside").unwrap();
    std::fs::write(&part_target, b"part outside").unwrap();
    let pending = publication::pending_timing_path(dir.path());
    symlink(&pending_target, &pending).unwrap();
    assert!(publication::begin_timing_publication(dir.path()).is_err());
    assert_eq!(std::fs::read(&pending_target).unwrap(), b"pending outside");
    std::fs::remove_file(&pending).unwrap();

    publication::begin_timing_publication(dir.path()).unwrap();
    symlink(
        &part_target,
        dir.path().join("system-audio.json.pending.part"),
    )
    .unwrap();
    publication::write_timing(dir.path(), &timing(Some(37))).unwrap();
    assert_eq!(std::fs::read(&part_target).unwrap(), b"part outside");
    assert!(
        std::fs::symlink_metadata(dir.path().join("system-audio.json.pending.part"))
            .unwrap()
            .file_type()
            .is_symlink()
    );
}
