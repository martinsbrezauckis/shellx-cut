use std::fs;
use std::path::Path;

use tempfile::tempdir;

use crate::{
    read_manifest, recover_interrupted, stitch_complete, CaptureStart, CheckpointFacts,
    ManifestOwner, MediaFacts, OwnerState, RecoveryState,
};

fn tools(dir: &Path) -> (String, String) {
    #[cfg(not(windows))]
    let ffmpeg = dir.join("ffmpeg");
    #[cfg(windows)]
    let ffmpeg = dir.join("ffmpeg.cmd");
    #[cfg(not(windows))]
    let ffprobe = dir.join("ffprobe");
    #[cfg(windows)]
    let ffprobe = dir.join("ffprobe.cmd");
    #[cfg(not(windows))]
    fs::write(&ffmpeg, "#!/bin/sh\nlast=\nfor arg do last=$arg; done\n[ \"$last\" = - ] || printf checkpoint > \"$last\"\n").unwrap();
    #[cfg(windows)]
    fs::write(&ffmpeg, "@echo off\r\nset last=\r\n:next\r\nif \"%~1\"==\"\" goto done\r\nset \"last=%~1\"\r\nshift\r\ngoto next\r\n:done\r\nif \"%last%\"==\"-\" exit /b 0\r\n>\"%last%\" <nul set /p =checkpoint\r\nexit /b 0\r\n").unwrap();
    #[cfg(not(windows))]
    fs::write(&ffprobe, "#!/bin/sh\nprintf '{\"format\":{\"duration\":\"0.100\"},\"streams\":[{\"codec_type\":\"video\",\"nb_read_frames\":\"1\"}]}'\n").unwrap();
    #[cfg(windows)]
    fs::write(&ffprobe, "@echo off\r\necho {\"format\":{\"duration\":\"0.100\"},\"streams\":[{\"codec_type\":\"video\",\"nb_read_frames\":\"1\"}]}\r\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&ffmpeg, fs::Permissions::from_mode(0o755)).unwrap();
        fs::set_permissions(&ffprobe, fs::Permissions::from_mode(0o755)).unwrap();
    }
    (ffmpeg.display().to_string(), ffprobe.display().to_string())
}

fn segment(owner: &mut ManifestOwner, sequence: u64) {
    let staging = owner.begin_segment(sequence, sequence * 100).unwrap();
    fs::write(&staging, format!("segment-{sequence}")).unwrap();
    owner
        .publish(
            sequence,
            &staging,
            CheckpointFacts {
                start_ms: sequence * 100,
                end_ms: (sequence + 1) * 100,
                event_offset_ms: sequence * 100,
                audio_offset_ms: None,
            },
            MediaFacts {
                duration_ms: 100,
                decoded_video_frames: 1,
                has_audio: false,
            },
        )
        .unwrap();
}

#[test]
fn missing_final_checkpoint_seals_verified_prefix_and_retry_is_idempotent() {
    let root = tempdir().unwrap();
    let mut owner = ManifestOwner::begin(root.path(), CaptureStart::new("cap", 100)).unwrap();
    segment(&mut owner, 0);
    segment(&mut owner, 1);
    drop(owner);
    fs::remove_file(root.path().join("checkpoints/segment-000001.mp4")).unwrap();
    let (ffmpeg, ffprobe) = tools(root.path());
    let result = recover_interrupted(root.path(), &ffmpeg, &ffprobe, OwnerState::Dead)
        .unwrap()
        .unwrap();
    assert_eq!(result.receipt.state, RecoveryState::Recovered);
    assert_eq!(result.receipt.recovered_segments, 1);
    assert!(result.quarantined.is_none());
    assert!(root.path().join("recovered.mp4").is_file());
    assert!(
        recover_interrupted(root.path(), &ffmpeg, &ffprobe, OwnerState::Dead)
            .unwrap()
            .is_none()
    );
}

#[test]
fn unavailable_verifier_leaves_checkpoint_output_and_receipt_untouched() {
    let root = tempdir().unwrap();
    let mut owner = ManifestOwner::begin(root.path(), CaptureStart::new("cap", 100)).unwrap();
    segment(&mut owner, 0);
    drop(owner);
    let checkpoint = root.path().join("checkpoints/segment-000000.mp4");
    let before = fs::read(&checkpoint).unwrap();
    assert!(recover_interrupted(
        root.path(),
        "missing-ffmpeg",
        "missing-ffprobe",
        OwnerState::Dead
    )
    .is_err());
    assert_eq!(fs::read(&checkpoint).unwrap(), before);
    assert!(!root.path().join("recovered.mp4").exists());
    assert!(read_manifest(root.path()).unwrap().receipt.is_none());
    assert!(!root.path().join("quarantine").exists());
}

#[test]
fn output_published_before_receipt_is_verified_and_sealed_on_retry() {
    let root = tempdir().unwrap();
    let mut owner = ManifestOwner::begin(root.path(), CaptureStart::new("cap", 100)).unwrap();
    segment(&mut owner, 0);
    let checkpoints = owner.manifest().checkpoints.clone();
    drop(owner);
    let (ffmpeg, ffprobe) = tools(root.path());
    let output = stitch_complete(
        root.path(),
        &checkpoints,
        &ffmpeg,
        &ffprobe,
        "recovered.mp4",
    )
    .unwrap();
    let published = fs::read(&output).unwrap();
    let result = recover_interrupted(root.path(), &ffmpeg, &ffprobe, OwnerState::Dead)
        .unwrap()
        .unwrap();
    assert_eq!(result.receipt.state, RecoveryState::Recovered);
    assert_eq!(fs::read(&output).unwrap(), published);
    assert!(read_manifest(root.path()).unwrap().receipt.is_some());
}

#[test]
fn quarantine_before_receipt_is_durable_across_a_retry() {
    let root = tempdir().unwrap();
    let mut owner = ManifestOwner::begin(root.path(), CaptureStart::new("cap", 100)).unwrap();
    segment(&mut owner, 0);
    segment(&mut owner, 1);
    drop(owner);
    let corrupt = root.path().join("checkpoints/segment-000001.mp4");
    fs::write(&corrupt, b"corrupt-after-publication").unwrap();
    let quarantined = crate::integrity::quarantine(root.path(), 1).unwrap();
    assert!(quarantined.is_file());
    let (ffmpeg, ffprobe) = tools(root.path());
    let result = recover_interrupted(root.path(), &ffmpeg, &ffprobe, OwnerState::Dead)
        .unwrap()
        .unwrap();
    assert_eq!(result.receipt.state, RecoveryState::Quarantined);
    assert_eq!(result.quarantined.as_deref(), Some(quarantined.as_path()));
    assert!(read_manifest(root.path()).unwrap().receipt.is_some());
}

#[test]
#[cfg_attr(
    windows,
    ignore = "Windows replacement durability is tracked separately"
)]
fn corrupt_checkpoint_and_torn_tail_are_both_archived_before_idempotent_seal() {
    let root = tempdir().unwrap();
    let mut owner = ManifestOwner::begin(root.path(), CaptureStart::new("cap", 100)).unwrap();
    segment(&mut owner, 0);
    segment(&mut owner, 1);
    drop(owner);
    fs::write(
        root.path().join("checkpoints/segment-000001.mp4"),
        b"proved-corrupt-checkpoint",
    )
    .unwrap();
    let torn = b"{\"kind\":";
    use std::io::Write;
    fs::OpenOptions::new()
        .append(true)
        .open(root.path().join(crate::MANIFEST_FILE))
        .unwrap()
        .write_all(torn)
        .unwrap();
    let (ffmpeg, ffprobe) = tools(root.path());
    let result = recover_interrupted(root.path(), &ffmpeg, &ffprobe, OwnerState::Dead)
        .unwrap()
        .unwrap();
    assert_eq!(result.receipt.state, RecoveryState::Quarantined);
    assert_eq!(result.receipt.recovered_segments, 1);
    assert_eq!(result.receipt.source.as_deref(), Some("recovered.mp4"));
    assert_eq!(result.receipt.lost_tail_ms, Some(100));
    assert_eq!(result.receipt.lost_tail_lower_bound_ms, 100);
    assert_eq!(result.receipt.lost_tail_upper_bound_ms, Some(100));
    assert_eq!(
        result.quarantined,
        Some(crate::integrity::quarantine_path(root.path(), 1))
    );
    assert!(root
        .path()
        .join("quarantine/segment-000001.mp4.corrupt")
        .is_file());
    assert_eq!(
        fs::read(
            root.path()
                .join("quarantine/capture.manifest.torn-tail.jsonl")
        )
        .unwrap(),
        torn
    );
    let sealed = read_manifest(root.path()).unwrap();
    assert_eq!(sealed.receipt, Some(result.receipt));
    assert!(root.path().join("recovered.mp4").is_file());
    assert!(
        recover_interrupted(root.path(), &ffmpeg, &ffprobe, OwnerState::Dead)
            .unwrap()
            .is_none()
    );
}

#[test]
#[cfg_attr(
    windows,
    ignore = "Windows replacement durability is tracked separately"
)]
fn torn_tail_retry_preserves_a_checkpoint_quarantined_before_the_receipt() {
    use std::io::Write;

    let root = tempdir().unwrap();
    let mut owner = ManifestOwner::begin(root.path(), CaptureStart::new("cap", 100)).unwrap();
    segment(&mut owner, 0);
    segment(&mut owner, 1);
    drop(owner);
    fs::write(
        root.path().join("checkpoints/segment-000001.mp4"),
        b"proved-corrupt-checkpoint",
    )
    .unwrap();
    let corrupt = crate::integrity::quarantine(root.path(), 1).unwrap();
    fs::OpenOptions::new()
        .append(true)
        .open(root.path().join(crate::MANIFEST_FILE))
        .unwrap()
        .write_all(b"{\"kind\":")
        .unwrap();
    let (ffmpeg, ffprobe) = tools(root.path());
    let result = recover_interrupted(root.path(), &ffmpeg, &ffprobe, OwnerState::Dead)
        .unwrap()
        .unwrap();
    assert_eq!(result.receipt.state, RecoveryState::Quarantined);
    assert_eq!(result.quarantined.as_deref(), Some(corrupt.as_path()));
    assert!(root
        .path()
        .join("quarantine/capture.manifest.torn-tail.jsonl")
        .is_file());
    assert!(
        recover_interrupted(root.path(), &ffmpeg, &ffprobe, OwnerState::Dead)
            .unwrap()
            .is_none()
    );
}

#[cfg(unix)]
#[test]
fn stitch_uses_a_private_workspace_not_preplanted_capture_root_temps() {
    use std::os::unix::fs::symlink;

    let root = tempdir().unwrap();
    let outside = tempdir().unwrap();
    let mut owner = ManifestOwner::begin(root.path(), CaptureStart::new("cap", 100)).unwrap();
    segment(&mut owner, 0);
    let checkpoints = owner.manifest().checkpoints.clone();
    drop(owner);
    let names = [
        "stitch.concat.txt",
        ".stitch-000000.mp4",
        ".recovered.mp4.part.mp4",
    ];
    for name in names {
        let target = outside.path().join(name);
        fs::write(&target, format!("outside-{name}")).unwrap();
        symlink(&target, root.path().join(name)).unwrap();
    }
    let (ffmpeg, ffprobe) = tools(root.path());
    stitch_complete(
        root.path(),
        &checkpoints,
        &ffmpeg,
        &ffprobe,
        "recovered.mp4",
    )
    .unwrap();
    for name in names {
        assert_eq!(
            fs::read_to_string(outside.path().join(name)).unwrap(),
            format!("outside-{name}")
        );
        assert!(fs::symlink_metadata(root.path().join(name))
            .unwrap()
            .file_type()
            .is_symlink());
    }
}
