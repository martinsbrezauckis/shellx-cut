//! Files that only sit in a capture folder are not proof that this engine made
//! or checked them. A capture folder can come from someone else (a received
//! project): its journal, prepared parts, part records, notes and final take
//! are all the sender's. Only a take this engine recorded here keeps the quick
//! Stop and recovery shortcuts; a copied folder is checked the slow way.

use std::path::{Path, PathBuf};
use std::process::Command;

use record_core::FrameRate;
use tempfile::tempdir;

use super::{cfr, notes, plan};
use crate::integrity::verified_prefix;
use crate::{
    prepare_cfr_segment, read_manifest, stitch_complete_with_media, verify_media, CaptureStart,
    Checkpoint, CheckpointFacts, ManifestOwner,
};

/// A take recorded here: two solid-colour checkpoints with a held gap.
fn own_take(root: &Path) -> Vec<Checkpoint> {
    let mut start = CaptureStart::new("prov", 15_000);
    start.output_cadence = Some(FrameRate::new(30, 1).unwrap());
    let mut owner = ManifestOwner::begin(root, start).unwrap();
    for (sequence, (color, start_ms, end_ms)) in [("red", 0, 2_000), ("blue", 2_400, 4_000)]
        .into_iter()
        .enumerate()
    {
        let staging = owner.begin_segment(sequence as u64, start_ms).unwrap();
        let seconds = format!("{:.3}", (end_ms - start_ms) as f64 / 1000.0);
        assert!(Command::new("ffmpeg")
            .args(["-v", "error", "-y", "-f", "lavfi", "-i"])
            .arg(format!("color=c={color}:s=64x36:r=30:d={seconds}"))
            .args(["-c:v", "libx264", "-bf", "0", "-pix_fmt", "yuv420p", "-f", "mp4"])
            .arg(&staging)
            .status()
            .unwrap()
            .success());
        let media = verify_media("ffmpeg", "ffprobe", &staging).unwrap();
        let facts = CheckpointFacts {
            start_ms,
            end_ms,
            event_offset_ms: start_ms,
            audio_offset_ms: None,
        };
        owner
            .publish(sequence as u64, &staging, facts, media)
            .unwrap();
    }
    read_manifest(root).unwrap().checkpoints
}

/// The same folder received from someone else: a copy at another place.
fn received_copy(from: &Path, to: &Path) {
    assert!(Command::new("cp")
        .arg("-R")
        .arg(from)
        .arg(to)
        .status()
        .unwrap()
        .success());
}

/// An ffprobe that notes each call and then fails: only a decode that runs
/// leaves the note.
fn noting_ffprobe(dir: &Path) -> (String, PathBuf) {
    let note = dir.join("ffprobe-called");
    let script = dir.join("ffprobe-noting");
    std::fs::write(
        &script,
        format!("#!/bin/sh\necho called >> '{}'\nexit 1\n", note.display()),
    )
    .unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    (script.display().to_string(), note)
}

fn have_ffmpeg() -> bool {
    Command::new("ffmpeg")
        .arg("-version")
        .output()
        .is_ok_and(|out| out.status.success())
}

#[test]
fn a_received_checkpoint_is_decoded_even_when_its_journal_hash_matches() {
    if !have_ffmpeg() {
        return;
    }
    let dir = tempdir().unwrap();
    let own = dir.path().join("own");
    std::fs::create_dir(&own).unwrap();
    let segments = own_take(&own);
    let (ffprobe, called) = noting_ffprobe(dir.path());
    // This engine's own take keeps the quick admission: no decode.
    let (usable, issue) = verified_prefix(&own, &segments, "ffmpeg", &ffprobe).unwrap();
    assert_eq!((usable.len(), issue), (2, None));
    assert!(!called.exists(), "an own checkpoint is not decoded again");
    // The same bytes and journal in a received folder are decoded.
    let received = dir.path().join("received");
    received_copy(&own, &received);
    let admitted = verified_prefix(&received, &segments, "ffmpeg", &ffprobe);
    assert!(called.exists(), "a received checkpoint is decoded");
    assert!(
        !matches!(admitted, Ok((ref usable, None)) if usable.len() == 2),
        "an undecodable received checkpoint is not admitted: {admitted:?}"
    );
}

/// A part record and files planted in a received folder, made to agree with
/// each other and with the journal, repeating the hold unit `units` times.
fn plant_part(root: &Path, segment: &Checkpoint, plan: &plan::SegmentPlan, units: u64) {
    let dir = root.join("stitch-cache");
    std::fs::create_dir_all(&dir).unwrap();
    let main = dir.join(format!("segment-{:06}.cfr.mp4", segment.sequence));
    let unit = dir.join(format!("segment-{:06}.unit.mp4", segment.sequence));
    std::fs::write(&main, b"sender's picture").unwrap();
    std::fs::write(&unit, b"sender's unit").unwrap();
    let mut media = segment.media.clone().unwrap();
    media.duration_ms = plan.target_ms();
    let record = serde_json::json!({
        "schema": "shellx-cut/stitch-cfr-part/2",
        "sequence": segment.sequence,
        "source_sha256": segment.sha256,
        "lead_ms": plan.lead_ms,
        "span_ms": plan.span_ms,
        "hold_ms": plan.hold_ms,
        "rate": "30/1",
        "output_sha256": crate::manifest::sha256(&main).unwrap(),
        "media": media,
        "unit_sha256": crate::manifest::sha256(&unit).unwrap(),
        "units": units,
    });
    std::fs::write(
        dir.join(format!("segment-{:06}.cfr.json", segment.sequence)),
        serde_json::to_vec(&record).unwrap(),
    )
    .unwrap();
}

#[test]
fn a_part_planted_in_a_received_folder_is_not_reused() {
    if !have_ffmpeg() {
        return;
    }
    let dir = tempdir().unwrap();
    let own = dir.path().join("own");
    std::fs::create_dir(&own).unwrap();
    let segments = own_take(&own);
    let received = dir.path().join("received");
    received_copy(&own, &received);
    let media = segments[0].media.clone().unwrap();
    let grid = super::grid(
        Some(FrameRate::new(30, 1).unwrap()),
        std::slice::from_ref(&media),
    );
    let plan = plan::plan_segment(0, &segments[0], &media, Some(2_400)).unwrap();
    plant_part(&received, &segments[0], &plan, u64::MAX / 2);
    assert_eq!(
        cfr::cached(&received, &segments[0], &plan, &grid),
        None,
        "a part with no proof that this engine made it is made again"
    );
}

#[test]
fn an_own_prepared_part_is_reused_and_its_copy_is_not() {
    if !have_ffmpeg() {
        return;
    }
    let dir = tempdir().unwrap();
    let own = dir.path().join("own");
    std::fs::create_dir(&own).unwrap();
    let segments = own_take(&own);
    prepare_cfr_segment(&own, &segments, 0, Some(2_400), "ffmpeg", "ffprobe").unwrap();
    let media = segments[0].media.clone().unwrap();
    let grid = super::grid(
        Some(FrameRate::new(30, 1).unwrap()),
        std::slice::from_ref(&media),
    );
    let plan = plan::plan_segment(0, &segments[0], &media, Some(2_400)).unwrap();
    assert!(cfr::cached(&own, &segments[0], &plan, &grid).is_some());
    let received = dir.path().join("received");
    received_copy(&own, &received);
    assert_eq!(cfr::cached(&received, &segments[0], &plan, &grid), None);
}

#[test]
fn a_journal_span_longer_than_any_recording_is_refused_before_the_part_list() {
    let segment = Checkpoint {
        sequence: 0,
        file: "checkpoints/segment-000000.mp4".into(),
        bytes: 1,
        sha256: String::new(),
        facts: CheckpointFacts {
            start_ms: 0,
            end_ms: u64::MAX / 4,
            event_offset_ms: 0,
            audio_offset_ms: None,
        },
        media: None,
        held_last_frame_ms: None,
    };
    let media = crate::MediaFacts {
        duration_ms: 1_000,
        decoded_video_frames: 30,
        has_audio: false,
        width: None,
        height: None,
        codec_name: None,
        avg_frame_rate: None,
        r_frame_rate: None,
    };
    assert!(plan::plan_segment(0, &segment, &media, None).is_err());
    // Many parts that are each short enough cannot add up to more either.
    let parts: Vec<Checkpoint> = (0..30)
        .map(|sequence| {
            let mut part = segment.clone();
            part.sequence = sequence;
            part.facts.start_ms = sequence * 6 * 24 * 3_600_000;
            part.facts.event_offset_ms = part.facts.start_ms;
            part.facts.end_ms = part.facts.start_ms + 1_000;
            part
        })
        .collect();
    let media = vec![media; parts.len()];
    assert!(plan::plan_segments(&parts, &media).is_err());
}

#[test]
fn notes_planted_beside_a_received_take_do_not_replace_its_check() {
    if !have_ffmpeg() {
        return;
    }
    let dir = tempdir().unwrap();
    let own = dir.path().join("own");
    std::fs::create_dir(&own).unwrap();
    let segments = own_take(&own);
    let received = dir.path().join("received");
    received_copy(&own, &received);
    // A playable take that contradicts the journal (40 frames of another
    // picture instead of about 4 s), with notes that vouch for its hash.
    let take = received.join("recovered.mp4");
    assert!(Command::new("ffmpeg")
        .args(["-v", "error", "-y", "-f", "lavfi", "-i"])
        .arg("color=c=green:s=64x36:r=30:d=1.333")
        .args(["-c:v", "libx264", "-pix_fmt", "yuv420p"])
        .arg(&take)
        .status()
        .unwrap()
        .success());
    let notes = serde_json::json!({
        "schema": "shellx-cut/stitch-notes/1",
        "output": "recovered.mp4",
        "output_sha256": crate::manifest::sha256(&take).unwrap(),
        "notes": ["Nothing to see here."],
    });
    std::fs::write(
        received.join("stitch-notes.json"),
        serde_json::to_vec(&notes).unwrap(),
    )
    .unwrap();
    assert_eq!(notes::recorded_for(&received, &take), None);
    let stitched =
        stitch_complete_with_media(&received, &segments, "ffmpeg", "ffprobe", "recovered.mp4");
    assert!(
        stitched.is_err(),
        "a received take that fails the checkpoint contract is not accepted: {:?}",
        stitched.map(|made| made.notes)
    );
}
#[test]
fn notes_this_engine_saved_with_its_take_are_kept() {
    let dir = tempdir().unwrap();
    let take = dir.path().join("source.mp4");
    std::fs::write(&take, b"a take saved with a problem").unwrap();
    let note = vec!["The saved video has 10 frames, but its parts have 12.".to_string()];
    let hash = notes::record(dir.path(), &take, "source.mp4", &note).unwrap();
    notes::remember_take(&take, &hash);
    assert_eq!(notes::recorded_for(dir.path(), &take), Some(note));
    let received = dir.path().join("copy");
    std::fs::create_dir(&received).unwrap();
    for name in ["source.mp4", "stitch-notes.json"] {
        std::fs::copy(dir.path().join(name), received.join(name)).unwrap();
    }
    assert_eq!(
        notes::recorded_for(&received, &received.join("source.mp4")),
        None
    );
}

/// Unit-level Stop timing of an own take whose parts were prepared during the
/// recording: admission of the checkpoints and the final stitch. Printed for
/// the comparison before and after the origin records; it also asserts that
/// Stop made no part again and decoded no checkpoint.
#[test]
fn own_take_stop_timing_with_prepared_parts() {
    if !have_ffmpeg() {
        return;
    }
    let dir = tempdir().unwrap();
    let own = dir.path().join("own");
    std::fs::create_dir(&own).unwrap();
    let mut start = CaptureStart::new("timing", 15_000);
    start.output_cadence = Some(FrameRate::new(30, 1).unwrap());
    let mut owner = ManifestOwner::begin(&own, start).unwrap();
    let starts: Vec<u64> = (0..6).map(|index| index * 3_000).collect();
    for (sequence, &start_ms) in starts.iter().enumerate() {
        let staging = owner.begin_segment(sequence as u64, start_ms).unwrap();
        assert!(Command::new("ffmpeg")
            .args(["-v", "error", "-y", "-f", "lavfi", "-i"])
            .arg("testsrc2=s=1280x720:r=30:d=2.5")
            .args(["-c:v", "libx264", "-bf", "0", "-pix_fmt", "yuv420p", "-f", "mp4"])
            .arg(&staging)
            .status()
            .unwrap()
            .success());
        let media = verify_media("ffmpeg", "ffprobe", &staging).unwrap();
        let facts = CheckpointFacts {
            start_ms,
            end_ms: start_ms + 2_500,
            event_offset_ms: start_ms,
            audio_offset_ms: None,
        };
        owner
            .publish(sequence as u64, &staging, facts, media)
            .unwrap();
    }
    let segments = read_manifest(&own).unwrap().checkpoints;
    for index in 0..segments.len() - 1 {
        prepare_cfr_segment(
            &own,
            &segments[..=index],
            index,
            Some(starts[index + 1]),
            "ffmpeg",
            "ffprobe",
        )
        .unwrap();
    }
    let started = std::time::Instant::now();
    let (usable, issue) = verified_prefix(&own, &segments, "ffmpeg", "ffprobe").unwrap();
    let admitted = started.elapsed();
    assert_eq!((usable.len(), issue), (6, None));
    let stitched =
        stitch_complete_with_media(&own, &segments, "ffmpeg", "ffprobe", "source.mp4").unwrap();
    let stop = started.elapsed();
    assert!(stitched.notes.is_empty(), "{:?}", stitched.notes);
    eprintln!(
        "OWN-TAKE-STOP admission {} ms, admission + stitch {} ms",
        admitted.as_millis(),
        stop.as_millis()
    );
}
