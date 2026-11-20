//! Still-screen takes: a native encoder delivers a few frames per second and
//! restarts leave gaps. The stitched take must be CFR on the admitted grid,
//! last exactly as long as the recording, hold the earlier picture through a
//! gap, reuse parts prepared during the recording, and save the take with a
//! note (never discard it) when one part cannot be made.

use std::path::{Path, PathBuf};
use std::process::Command;

use record_core::FrameRate;
use tempfile::tempdir;

use crate::{
    prepare_cfr_segment, stitch_complete_with_media, verify_media, CaptureStart, CheckpointFacts,
    ManifestOwner,
};

/// One sparse VFR segment of a solid colour: frames at `times_ms` (first at 0),
/// a 600 timescale like ScreenCaptureKit files, published at `start..end`.
fn publish_sparse(
    owner: &mut ManifestOwner,
    sequence: u64,
    color: &str,
    times_ms: &[u64],
    start_ms: u64,
    end_ms: u64,
) {
    let staging = owner.begin_segment(sequence, start_ms).unwrap();
    let expr: Vec<String> = times_ms
        .iter()
        .enumerate()
        .map(|(index, time)| format!("eq(N\\,{index})*{:.3}", *time as f64 / 1000.0))
        .collect();
    assert!(Command::new("ffmpeg")
        .args([
            "-v",
            "error",
            "-y",
            "-f",
            "lavfi",
            "-i",
            &format!("color=c={color}:s=64x36:r=60")
        ])
        .args(["-frames:v", &times_ms.len().to_string()])
        .args(["-vf", &format!("setpts=({})/TB", expr.join("+"))])
        .args([
            "-fps_mode",
            "vfr",
            "-video_track_timescale",
            "600",
            "-c:v",
            "libx264",
            "-bf",
            "0",
            "-pix_fmt",
            "yuv420p"
        ])
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
    owner.publish(sequence, &staging, facts, media).unwrap();
}

/// The 1u still-screen run: a late first frame, 14 to 15 s segments with a
/// handful of frames each, restart gaps, and a short last segment.
fn still_screen_capture(root: &Path) {
    let mut start = CaptureStart::new("still", 15_000);
    start.output_cadence = Some(FrameRate::new(30, 1).unwrap());
    let mut owner = ManifestOwner::begin(root, start).unwrap();
    publish_sparse(
        &mut owner,
        0,
        "red",
        &[0, 400, 3_000, 9_000, 14_200],
        120,
        15_013,
    );
    publish_sparse(&mut owner, 1, "lime", &[0, 6_000, 13_900], 16_900, 30_920);
    publish_sparse(&mut owner, 2, "blue", &[0, 100, 148], 31_759, 32_100);
}

/// RGB of the stitched frame shown at `at_ms`.
fn color_at(path: &Path, at_ms: u64) -> [u8; 3] {
    let out = Command::new("ffmpeg")
        .args([
            "-v",
            "error",
            "-ss",
            &format!("{:.3}", at_ms as f64 / 1000.0),
            "-i",
        ])
        .arg(path)
        .args([
            "-frames:v",
            "1",
            "-vf",
            "crop=1:1:32:18,format=rgb24",
            "-f",
            "rawvideo",
            "-",
        ])
        .output()
        .unwrap();
    assert_eq!(out.stdout.len(), 3, "one RGB pixel at {at_ms} ms");
    [out.stdout[0], out.stdout[1], out.stdout[2]]
}

fn dominant(rgb: [u8; 3]) -> usize {
    (0..3).max_by_key(|&channel| rgb[channel]).unwrap()
}

/// An ffmpeg that logs every part it is asked to make, and refuses the part
/// of `refuse` (a checkpoint file name) when given.
fn logging_ffmpeg(dir: &Path, refuse: Option<&str>) -> (String, PathBuf) {
    let log = dir.join("ffmpeg-calls.log");
    let script = dir.join("ffmpeg-wrapper");
    let refuse = refuse.map_or(String::new(), |name| {
        format!("case \"$*\" in *{name}*tpad*) exit 1;; esac\n")
    });
    std::fs::write(
        &script,
        format!(
            "#!/bin/sh\ncase \"$*\" in *tpad*) echo \"$*\" >> '{}';; esac\n{refuse}exec ffmpeg \"$@\"\n",
            log.display()
        ),
    )
    .unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    (script.display().to_string(), log)
}

fn part_calls(log: &Path) -> usize {
    std::fs::read_to_string(log).map_or(0, |text| text.lines().count())
}

#[test]
fn real_ffmpeg_still_screen_take_is_cfr_on_the_capture_clock_and_holds_the_earlier_picture() {
    let dir = tempdir().unwrap();
    still_screen_capture(dir.path());
    let segments = crate::read_manifest(dir.path()).unwrap().checkpoints;
    let stitched =
        stitch_complete_with_media(dir.path(), &segments, "ffmpeg", "ffprobe", "source.mp4")
            .unwrap();
    assert!(stitched.notes.is_empty(), "{:?}", stitched.notes);
    let media = verify_media("ffmpeg", "ffprobe", &stitched.path).unwrap();
    let thirty = FrameRate::new(30, 1).ok();
    assert_eq!((media.avg_frame_rate, media.r_frame_rate), (thirty, thirty));
    assert!(media.duration_ms.abs_diff(32_100) <= 34, "{media:?}");
    assert!(media.decoded_video_frames.abs_diff(963) <= 1, "{media:?}");
    // Before the first frame: that first picture. In the restart gap after
    // segment 0 (15.0 to 16.9 s): segment 0's last picture, red, not the next
    // one. Inside segment 1 after its first frame: lime.
    assert_eq!(dominant(color_at(&stitched.path, 50)), 0);
    assert_eq!(dominant(color_at(&stitched.path, 16_000)), 0);
    assert_eq!(dominant(color_at(&stitched.path, 17_500)), 1);
    assert_eq!(dominant(color_at(&stitched.path, 31_000)), 1);
    assert_eq!(dominant(color_at(&stitched.path, 32_000)), 2);
}

#[test]
fn real_ffmpeg_a_part_prepared_during_the_recording_is_reused_at_stop() {
    let dir = tempdir().unwrap();
    still_screen_capture(dir.path());
    let segments = crate::read_manifest(dir.path()).unwrap().checkpoints;
    let (ffmpeg, log) = logging_ffmpeg(dir.path(), None);
    prepare_cfr_segment(
        dir.path(),
        &segments[..1],
        0,
        Some(16_900),
        &ffmpeg,
        "ffprobe",
    )
    .unwrap();
    prepare_cfr_segment(
        dir.path(),
        &segments[..2],
        1,
        Some(31_759),
        &ffmpeg,
        "ffprobe",
    )
    .unwrap();
    assert_eq!(part_calls(&log), 2);
    // A trusted record made before unit_frames was added still names the
    // original one-second unit; only a wider-unit stitch needs the new field.
    let old_record = dir.path().join("stitch-cache/segment-000000.cfr.json");
    let mut data: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&old_record).unwrap()).unwrap();
    data.as_object_mut().unwrap().remove("unit_frames");
    std::fs::write(&old_record, serde_json::to_vec(&data).unwrap()).unwrap();
    cut_media::cache_trust::remember(&old_record);
    let stitched =
        stitch_complete_with_media(dir.path(), &segments, &ffmpeg, "ffprobe", "source.mp4")
            .unwrap();
    assert_eq!(part_calls(&log), 3, "only the last part is made at Stop");
    assert!(stitched.notes.is_empty(), "{:?}", stitched.notes);
    assert!(stitched.media.duration_ms.abs_diff(32_100) <= 34);
}

#[test]
fn real_ffmpeg_a_stale_prepared_part_is_not_reused() {
    let dir = tempdir().unwrap();
    still_screen_capture(dir.path());
    let segments = crate::read_manifest(dir.path()).unwrap().checkpoints;
    let (ffmpeg, log) = logging_ffmpeg(dir.path(), None);
    // Prepared with a wrong next start: its hold does not match the journal.
    prepare_cfr_segment(
        dir.path(),
        &segments[..1],
        0,
        Some(16_000),
        &ffmpeg,
        "ffprobe",
    )
    .unwrap();
    let stitched =
        stitch_complete_with_media(dir.path(), &segments, &ffmpeg, "ffprobe", "source.mp4")
            .unwrap();
    assert_eq!(part_calls(&log), 4);
    assert!(stitched.media.duration_ms.abs_diff(32_100) <= 34);
}

#[test]
fn real_ffmpeg_a_part_that_cannot_be_made_keeps_the_take_and_names_the_gap() {
    let dir = tempdir().unwrap();
    still_screen_capture(dir.path());
    let segments = crate::read_manifest(dir.path()).unwrap().checkpoints;
    let (ffmpeg, _log) = logging_ffmpeg(dir.path(), Some("segment-000001.mp4"));
    let stitched =
        stitch_complete_with_media(dir.path(), &segments, &ffmpeg, "ffprobe", "source.mp4")
            .unwrap();
    assert_eq!(stitched.notes.len(), 1, "{:?}", stitched.notes);
    assert!(
        stitched.notes[0].contains("from 16.9 s to 30.9 s"),
        "{:?}",
        stitched.notes
    );
    // Still the whole recording, with segment 0's picture over the lost part.
    assert!(stitched.media.duration_ms.abs_diff(32_100) <= 34);
    assert_eq!(dominant(color_at(&stitched.path, 20_000)), 0);
    // A second stitch of the same capture accepts that exact file and its notes.
    let again =
        stitch_complete_with_media(dir.path(), &segments, "ffmpeg", "ffprobe", "source.mp4")
            .unwrap();
    assert_eq!(again.notes, stitched.notes);
}

/// Real ScreenCaptureKit checkpoints (B-frames, a 600 timescale, sparse still
/// segments): `SHELLX_CUT_SCK_CAPTURE_DIR` names a copied capture folder with
/// `capture.manifest.jsonl` and `checkpoints/`. The take is stitched in a
/// private copy; the folder itself is never changed.
#[test]
#[ignore = "requires a copied real ScreenCaptureKit capture folder"]
fn real_screencapturekit_capture_folder_stitches_on_the_capture_clock() {
    let source = PathBuf::from(
        std::env::var("SHELLX_CUT_SCK_CAPTURE_DIR").expect("SHELLX_CUT_SCK_CAPTURE_DIR"),
    );
    let dir = tempdir().unwrap();
    std::fs::copy(
        source.join(crate::MANIFEST_FILE),
        dir.path().join(crate::MANIFEST_FILE),
    )
    .unwrap();
    std::fs::create_dir(dir.path().join("checkpoints")).unwrap();
    for entry in std::fs::read_dir(source.join("checkpoints")).unwrap() {
        let entry = entry.unwrap();
        std::fs::copy(
            entry.path(),
            dir.path().join("checkpoints").join(entry.file_name()),
        )
        .unwrap();
    }
    let segments = crate::read_manifest(dir.path()).unwrap().checkpoints;
    // With SHELLX_CUT_SCK_PREPARED=1 every part but the last is made first, as
    // the recorder does while recording: the timing is then what Stop costs.
    if std::env::var_os("SHELLX_CUT_SCK_PREPARED").is_some() {
        for index in 0..segments.len().saturating_sub(1) {
            let next = segments[index + 1].facts.start_ms;
            prepare_cfr_segment(
                dir.path(),
                &segments[..=index],
                index,
                Some(next),
                "ffmpeg",
                "ffprobe",
            )
            .unwrap();
        }
    }
    let started = std::time::Instant::now();
    let stitched =
        stitch_complete_with_media(dir.path(), &segments, "ffmpeg", "ffprobe", "source.mp4")
            .unwrap();
    let took = started.elapsed();
    if let Some(keep) = std::env::var_os("SHELLX_CUT_SCK_KEEP") {
        std::fs::copy(
            &stitched.path,
            std::path::Path::new(&keep).join("source.mp4"),
        )
        .unwrap();
    }
    let end_ms = segments.last().unwrap().facts.end_ms;
    let media = verify_media("ffmpeg", "ffprobe", &stitched.path).unwrap();
    println!(
        "segments={} end_ms={end_ms} stitched_ms={} frames={} rate={:?} notes={:?} took_ms={}",
        segments.len(),
        media.duration_ms,
        media.decoded_video_frames,
        media.avg_frame_rate,
        stitched.notes,
        took.as_millis()
    );
    let thirty = FrameRate::new(30, 1).ok();
    assert_eq!((media.avg_frame_rate, media.r_frame_rate), (thirty, thirty));
    assert!(media.duration_ms.abs_diff(end_ms) <= 34, "{media:?}");
    assert!(stitched.notes.is_empty(), "{:?}", stitched.notes);
}

/// A real UI take: part boundaries that are not whole frames. Parts
/// trimmed by time left one frame 0.3 ms short and the person read a false
/// "Saved with a problem: ... not steady" note. Parts are cut by frames now.
#[test]
fn real_ffmpeg_parts_that_end_between_frames_still_make_an_exact_30_fps_take() {
    let dir = tempdir().unwrap();
    let mut start = CaptureStart::new("between", 15_000);
    start.output_cadence = Some(FrameRate::new(30, 1).unwrap());
    let mut owner = ManifestOwner::begin(dir.path(), start).unwrap();
    publish_sparse(&mut owner, 0, "red", &[0, 4_000, 14_900], 23, 15_000);
    publish_sparse(&mut owner, 1, "lime", &[0, 9_000, 14_000], 16_214, 30_271);
    publish_sparse(&mut owner, 2, "blue", &[0, 3_000, 3_960], 31_089, 35_149);
    let segments = crate::read_manifest(dir.path()).unwrap().checkpoints;
    let stitched =
        stitch_complete_with_media(dir.path(), &segments, "ffmpeg", "ffprobe", "source.mp4")
            .unwrap();
    assert!(stitched.notes.is_empty(), "{:?}", stitched.notes);
    let thirty = FrameRate::new(30, 1).ok();
    assert_eq!(
        (stitched.media.avg_frame_rate, stitched.media.r_frame_rate),
        (thirty, thirty)
    );
    assert!(stitched.media.duration_ms.abs_diff(35_149) <= 34);
}

/// A real still-screen take: 0.85 s of pictures, then the screen held for 43 s.
/// The held stretch is one short unit repeated by the concat, not 1300 encoded
/// frames; the take is still exact and holds the last picture.
#[test]
fn real_ffmpeg_a_long_still_hold_is_a_repeated_unit_and_stays_exact() {
    let dir = tempdir().unwrap();
    let mut start = CaptureStart::new("hold", 15_000);
    start.output_cadence = Some(FrameRate::new(30, 1).unwrap());
    let mut owner = ManifestOwner::begin(dir.path(), start).unwrap();
    publish_sparse(&mut owner, 0, "red", &[0, 300, 817], 17, 15_065);
    publish_sparse(&mut owner, 1, "lime", &[0, 30, 60], 58_361, 60_306);
    let segments = crate::read_manifest(dir.path()).unwrap().checkpoints;
    prepare_cfr_segment(
        dir.path(),
        &segments[..1],
        0,
        Some(58_361),
        "ffmpeg",
        "ffprobe",
    )
    .unwrap();
    let unit = dir.path().join("stitch-cache/segment-000000.unit.mp4");
    let unit_facts = verify_media("ffmpeg", "ffprobe", &unit).unwrap();
    assert_eq!(unit_facts.decoded_video_frames, 30);
    let main = verify_media(
        "ffmpeg",
        "ffprobe",
        &dir.path().join("stitch-cache/segment-000000.cfr.mp4"),
    )
    .unwrap();
    assert!(main.decoded_video_frames < 60, "{main:?}");
    let stitched =
        stitch_complete_with_media(dir.path(), &segments, "ffmpeg", "ffprobe", "source.mp4")
            .unwrap();
    assert!(stitched.notes.is_empty(), "{:?}", stitched.notes);
    let media = verify_media("ffmpeg", "ffprobe", &stitched.path).unwrap();
    let thirty = FrameRate::new(30, 1).ok();
    assert_eq!((media.avg_frame_rate, media.r_frame_rate), (thirty, thirty));
    assert!(media.duration_ms.abs_diff(60_306) <= 34, "{media:?}");
    assert_eq!(media.decoded_video_frames, 1_809);
    // The whole held stretch shows the last red picture; the next segment is lime.
    assert_eq!(dominant(color_at(&stitched.path, 30_000)), 0);
    assert_eq!(dominant(color_at(&stitched.path, 58_100)), 0);
    assert_eq!(dominant(color_at(&stitched.path, 59_000)), 1);
}

#[test]
fn real_ffmpeg_a_five_minute_take_with_two_long_gaps_stitches() {
    let dir = tempdir().unwrap();
    let mut start = CaptureStart::new("long-pauses", 15_000);
    start.output_cadence = Some(FrameRate::new(30, 1).unwrap());
    let mut owner = ManifestOwner::begin(dir.path(), start).unwrap();
    publish_sparse(&mut owner, 0, "red", &[0, 2_000, 14_000], 0, 15_000);
    publish_sparse(&mut owner, 1, "lime", &[0, 2_000, 14_000], 135_000, 150_000);
    publish_sparse(&mut owner, 2, "blue", &[0, 2_000, 14_000], 285_000, 300_000);
    let segments = crate::read_manifest(dir.path()).unwrap().checkpoints;
    let (ffmpeg, log) = logging_ffmpeg(dir.path(), None);
    prepare_cfr_segment(
        dir.path(),
        &segments[..1],
        0,
        Some(135_000),
        &ffmpeg,
        "ffprobe",
    )
    .unwrap();
    assert_eq!(part_calls(&log), 1);
    let refusal = crate::stitch::stitch_complete_with_media_bounded(
        dir.path(),
        &segments,
        &ffmpeg,
        "ffprobe",
        "source.mp4",
        1,
    )
    .unwrap_err();
    assert!(matches!(refusal, crate::ManifestError::StitchLimit(_)));
    let stitched = crate::stitch::stitch_complete_with_media_bounded(
        dir.path(),
        &segments,
        &ffmpeg,
        "ffprobe",
        "source.mp4",
        50,
    )
    .unwrap();
    assert_eq!(
        part_calls(&log),
        4,
        "the old one-second part was remade for the wider unit"
    );
    assert!(stitched.notes.is_empty(), "{:?}", stitched.notes);
    assert!(stitched.media.duration_ms.abs_diff(300_000) <= 34);
    assert_eq!(stitched.media.decoded_video_frames, 9_000);
    assert_eq!(dominant(color_at(&stitched.path, 120_000)), 0);
    assert_eq!(dominant(color_at(&stitched.path, 280_000)), 1);
    assert_eq!(dominant(color_at(&stitched.path, 295_000)), 2);
}
