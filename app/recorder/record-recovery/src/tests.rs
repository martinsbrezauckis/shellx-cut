use std::fs;
use std::path::Path;
use std::process::Command;

use tempfile::tempdir;

use crate::{
    owner_state, recover_interrupted, verify_media, CaptureStart, CheckpointFacts, ManifestOwner,
    MediaFacts, OwnerState, RecoveryState,
};

fn tools(dir: &Path) -> (String, String) {
    let ffmpeg = dir.join("ffmpeg");
    let ffprobe = dir.join("ffprobe");
    fs::write(&ffmpeg, "#!/bin/sh\nlast=\nfor arg do last=$arg; done\n[ \"$last\" = - ] || printf checkpoint > \"$last\"\n").unwrap();
    fs::write(&ffprobe, "#!/bin/sh\nprintf '{\"format\":{\"duration\":\"0.100\"},\"streams\":[{\"codec_type\":\"video\",\"nb_read_frames\":\"1\"}]}'\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&ffmpeg, fs::Permissions::from_mode(0o755)).unwrap();
        fs::set_permissions(&ffprobe, fs::Permissions::from_mode(0o755)).unwrap();
    }
    (ffmpeg.display().to_string(), ffprobe.display().to_string())
}
fn media(duration_ms: u64) -> MediaFacts {
    MediaFacts {
        duration_ms,
        decoded_video_frames: 1,
        has_audio: false,
    }
}
fn segment(owner: &mut ManifestOwner, seq: u64) {
    let p = owner.begin_segment(seq, seq * 100).unwrap();
    fs::write(&p, format!("segment-{seq}")).unwrap();
    owner
        .publish(
            seq,
            &p,
            CheckpointFacts {
                start_ms: seq * 100,
                end_ms: (seq + 1) * 100,
                event_offset_ms: seq * 100,
                audio_offset_ms: None,
            },
            media(100),
        )
        .unwrap();
}

#[test]
fn publish_is_append_only_and_crash_after_open_has_unknown_tail() {
    let dir = tempdir().unwrap();
    let mut owner = ManifestOwner::begin(dir.path(), CaptureStart::new("cap", 100)).unwrap();
    segment(&mut owner, 0);
    let _open = owner.begin_segment(1, 100).unwrap();
    let (ffmpeg, ffprobe) = tools(dir.path());
    let receipt = recover_interrupted(dir.path(), &ffmpeg, &ffprobe, OwnerState::Dead)
        .unwrap()
        .unwrap()
        .receipt;
    assert_eq!(receipt.recovered_segments, 1);
    assert_eq!(receipt.lost_tail_ms, None);
    assert_eq!(receipt.lost_tail_lower_bound_ms, 0);
}

#[test]
fn no_usable_checkpoint_is_interrupted_not_recovered() {
    let dir = tempdir().unwrap();
    let _owner = ManifestOwner::begin(dir.path(), CaptureStart::new("cap", 100)).unwrap();
    let (ffmpeg, ffprobe) = tools(dir.path());
    let receipt = recover_interrupted(dir.path(), &ffmpeg, &ffprobe, OwnerState::Dead)
        .unwrap()
        .expect("dead unsealed capture receives a terminal receipt")
        .receipt;
    assert_eq!(receipt.state, RecoveryState::Interrupted);
    assert_eq!(receipt.recovered_segments, 0);
    assert_eq!(receipt.source, None);
    assert_eq!(
        receipt.lost_tail_ms, None,
        "no final open/media proof means unknown loss"
    );
}

#[test]
fn final_publish_never_replaces_an_existing_source() {
    let dir = tempdir().unwrap();
    let part = dir.path().join(".source.mp4.part.mp4");
    let output = dir.path().join("source.mp4");
    fs::write(&part, b"new verified output").unwrap();
    fs::write(&output, b"pre-existing output").unwrap();
    assert!(crate::atomic::publish_new_synced(&part, &output).is_err());
    assert_eq!(fs::read(&output).unwrap(), b"pre-existing output");
    assert!(
        part.is_file(),
        "failed publication keeps its recoverable candidate"
    );
}

#[test]
fn only_one_open_segment_is_valid_or_resumable() {
    let dir = tempdir().unwrap();
    let mut owner = ManifestOwner::begin(dir.path(), CaptureStart::new("cap", 100)).unwrap();
    let _first = owner.begin_segment(0, 0).unwrap();
    assert!(owner.begin_segment(1, 100).is_err());
    drop(owner);
    let manifest = dir.path().join(crate::MANIFEST_FILE);
    use std::io::Write;
    std::fs::OpenOptions::new()
        .append(true)
        .open(manifest)
        .unwrap()
        .write_all(b"{\"kind\":\"open\",\"sequence\":1,\"staging\":\".checkpoint-000001.open.mp4\",\"start_ms\":100}\n")
        .unwrap();
    assert!(crate::read_manifest(dir.path()).is_err());
}

#[test]
fn wrong_pid_live_owner_and_corrupt_middle_are_fail_closed_and_idempotent() {
    let dir = tempdir().unwrap();
    let mut owner = ManifestOwner::begin(dir.path(), CaptureStart::new("cap", 100)).unwrap();
    segment(&mut owner, 0);
    segment(&mut owner, 1);
    segment(&mut owner, 2);
    let (ffmpeg, ffprobe) = tools(dir.path());
    assert!(
        recover_interrupted(dir.path(), &ffmpeg, &ffprobe, OwnerState::Alive)
            .unwrap()
            .is_none()
    );
    fs::write(
        dir.path().join("checkpoints/segment-000001.mp4"),
        b"truncated",
    )
    .unwrap();
    let result = recover_interrupted(dir.path(), &ffmpeg, &ffprobe, OwnerState::Dead)
        .unwrap()
        .unwrap();
    assert_eq!(result.receipt.recovered_segments, 1);
    assert!(result.quarantined.unwrap().is_file());
    assert!(
        recover_interrupted(dir.path(), &ffmpeg, &ffprobe, OwnerState::Dead)
            .unwrap()
            .is_none()
    );
}

#[test]
fn truncated_final_segment_reports_exact_known_tail() {
    let dir = tempdir().unwrap();
    let mut owner = ManifestOwner::begin(dir.path(), CaptureStart::new("cap", 100)).unwrap();
    segment(&mut owner, 0);
    segment(&mut owner, 1);
    fs::write(
        dir.path().join("checkpoints/segment-000001.mp4"),
        b"truncated-final",
    )
    .unwrap();
    let (ffmpeg, ffprobe) = tools(dir.path());
    let receipt = recover_interrupted(dir.path(), &ffmpeg, &ffprobe, OwnerState::Dead)
        .unwrap()
        .unwrap()
        .receipt;
    assert_eq!(receipt.recovered_segments, 1);
    assert_eq!(receipt.lost_tail_ms, Some(100));
    assert_eq!(receipt.lost_tail_upper_bound_ms, Some(100));
}

#[test]
fn pid_reuse_is_ambiguous_and_torn_tail_keeps_valid_prefix() {
    let mut start = CaptureStart::new("cap", 100);
    start.owner_identity = "other-process".into();
    assert_eq!(
        owner_state(&start),
        OwnerState::Ambiguous,
        "never recover a reused live PID"
    );
    let dir = tempdir().unwrap();
    let mut owner = ManifestOwner::begin(dir.path(), CaptureStart::new("cap", 100)).unwrap();
    segment(&mut owner, 0);
    drop(owner);
    let manifest = dir.path().join(crate::MANIFEST_FILE);
    use std::io::Write;
    std::fs::OpenOptions::new()
        .append(true)
        .open(&manifest)
        .unwrap()
        .write_all(b"{\"kind\":")
        .unwrap();
    let (ffmpeg, ffprobe) = tools(dir.path());
    let result = recover_interrupted(dir.path(), &ffmpeg, &ffprobe, OwnerState::Dead)
        .unwrap()
        .unwrap();
    assert_eq!(result.receipt.recovered_segments, 1);
    let tail = result.quarantined.unwrap();
    assert_eq!(fs::read(&tail).unwrap(), b"{\"kind\":");
    let repaired = crate::read_manifest(dir.path()).unwrap();
    assert!(!repaired.has_torn_tail());
    assert!(repaired.receipt.is_some());
    assert!(
        recover_interrupted(dir.path(), &ffmpeg, &ffprobe, OwnerState::Dead)
            .unwrap()
            .is_none()
    );
}

#[test]
fn real_ffmpeg_finalized_checkpoint_is_playable_and_stitches() {
    let dir = tempdir().unwrap();
    let mut owner = ManifestOwner::begin(dir.path(), CaptureStart::new("cap", 250)).unwrap();
    for seq in 0..2 {
        let staging = owner.begin_segment(seq, seq * 250).unwrap();
        let status = Command::new("ffmpeg")
            .args([
                "-v",
                "error",
                "-y",
                "-f",
                "lavfi",
                "-i",
                "color=c=black:s=32x32:r=10",
                "-t",
                "0.25",
                "-c:v",
                "libx264",
                "-pix_fmt",
                "yuv420p",
            ])
            .arg(&staging)
            .status()
            .expect("ffmpeg is required for the Linux checkpoint proof");
        assert!(status.success());
        let media = verify_media("ffmpeg", "ffprobe", &staging).unwrap();
        owner
            .publish(
                seq,
                &staging,
                CheckpointFacts {
                    start_ms: seq * 250,
                    end_ms: (seq + 1) * 250,
                    event_offset_ms: seq * 250,
                    audio_offset_ms: None,
                },
                media,
            )
            .unwrap();
    }
    let stitched = crate::stitch_complete(
        dir.path(),
        &owner.manifest().checkpoints,
        "ffmpeg",
        "ffprobe",
        "source.mp4",
    )
    .unwrap();
    assert!(
        stitched.is_file(),
        "ffprobe + decoded frame accepted the stitched source"
    );
}

#[test]
fn real_ffmpeg_stitch_materializes_measured_setup_delay_gap() {
    let dir = tempdir().unwrap();
    let mut owner = ManifestOwner::begin(dir.path(), CaptureStart::new("cap", 250)).unwrap();
    let setup_delay_ms = 250;
    // Models a backend that measured the second encoder's actual start only after
    // finalize/probe/setup completed, rather than reusing the prior segment end.
    for (seq, start_ms, end_ms) in [(0, 0, 250), (1, 250 + setup_delay_ms, 750)] {
        let staging = owner.begin_segment(seq, start_ms).unwrap();
        assert!(Command::new("ffmpeg")
            .args([
                "-v",
                "error",
                "-y",
                "-f",
                "lavfi",
                "-i",
                "color=c=black:s=32x32:r=10",
                "-t",
                "0.25",
                "-c:v",
                "libx264",
                "-pix_fmt",
                "yuv420p",
            ])
            .arg(&staging)
            .status()
            .unwrap()
            .success());
        let media = verify_media("ffmpeg", "ffprobe", &staging).unwrap();
        owner
            .publish(
                seq,
                &staging,
                CheckpointFacts {
                    start_ms,
                    end_ms,
                    event_offset_ms: start_ms,
                    audio_offset_ms: Some(0),
                },
                media,
            )
            .unwrap();
    }
    let source = crate::stitch_complete(
        dir.path(),
        &owner.manifest().checkpoints,
        "ffmpeg",
        "ffprobe",
        "source.mp4",
    )
    .unwrap();
    let facts = verify_media("ffmpeg", "ffprobe", &source).unwrap();
    let expected_ms = owner
        .manifest()
        .checkpoints
        .iter()
        .filter_map(|checkpoint| checkpoint.media.as_ref())
        .map(|media| media.duration_ms)
        .sum::<u64>()
        .saturating_add(setup_delay_ms);
    assert!(
        facts.duration_ms.abs_diff(expected_ms) <= 120,
        "the measured 250ms restart gap must reach the decoded timeline: {facts:?}"
    );
    assert!(!facts.has_audio, "checkpoint media stays video-only");
    assert!(
        facts.decoded_video_frames >= 7,
        "final proof decodes the padded frames"
    );

    let mut mismatched_event_clock = owner.manifest().checkpoints.clone();
    mismatched_event_clock[1].facts.event_offset_ms = mismatched_event_clock[1]
        .facts
        .event_offset_ms
        .saturating_add(1);
    assert!(
        crate::stitch_complete(
            dir.path(),
            &mismatched_event_clock,
            "ffmpeg",
            "ffprobe",
            "event-clock-mismatch.mp4",
        )
        .is_err(),
        "a segment whose event clock disagrees with its video start must fail closed"
    );
}
