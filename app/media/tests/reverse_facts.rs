//! A reversed clip's memory is planned from facts this engine measured on the
//! source file, not from the project's cached probe. A received project can
//! describe a 1080p 30 fps source as 16x16 at 1 fps: planned from that, the
//! reversed part looked a few kilobytes big and got a window far above the
//! memory bound. A source that cannot be measured is planned at the
//! conservative bound.

cut_core::isolate_test_process!();

use cut_core::{edl_from_project, Project};
use cut_media::render::{
    measured_for_reverse, measured_reverse_part_bytes, resources::estimate_memory,
    reverse_part_bytes, reverse_refusal,
};
use serde_json::json;
use std::path::Path;

fn source(dir: &Path) -> std::path::PathBuf {
    let out = dir.join("src1080.mp4");
    let args: Vec<String> = [
        "-f",
        "lavfi",
        "-i",
        "testsrc2=s=1920x1080:r=30:d=4",
        "-c:v",
        "libx264",
        "-preset",
        "ultrafast",
        "-pix_fmt",
        "yuv420p",
    ]
    .iter()
    .map(|s| s.to_string())
    .chain([out.display().to_string()])
    .collect();
    cut_media::ffmpeg::run_ffmpeg(&args).expect("generate the source");
    out
}

/// A reversed 4 s clip whose cached probe claims 16x16 at 1 fps.
fn forged(path: &str) -> Project {
    serde_json::from_value(json!({
        "schema": "shellx-cut/1", "name": "forged-probe",
        "settings": {"width": 1920, "height": 1080, "fps": 30.0, "audio_rate": 48000},
        "assets": {"a1": {"path": path, "hash": "sha256:forged",
            "probe": {"kind": "video", "width": 16, "height": 16, "fps": 1.0,
                      "raw": {"streams": [{"codec_type": "video", "pix_fmt": "gray"}]}}}},
        "tracks": [{"id": "v1", "kind": "video", "clips": [
            {"id": "c1", "asset": "a1", "src_in_ms": 0, "src_out_ms": 4000, "reverse": true}]}],
    }))
    .expect("test project")
}

fn held_mb(p: &Project) -> u64 {
    let edl = edl_from_project(p);
    estimate_memory(
        p,
        &edl,
        1920,
        1080,
        &["-c:v".to_string(), "libx264".to_string()],
    )
    .held_frames_mb
}

fn dims(p: &Project) -> (Option<u64>, Option<u64>, Option<f64>) {
    let probe = p.assets["a1"].probe.as_ref().unwrap();
    (
        probe["width"].as_u64(),
        probe["height"].as_u64(),
        probe["fps"].as_f64(),
    )
}

#[test]
fn a_forged_small_probe_does_not_shrink_the_reversed_plan() {
    let dir = tempfile::tempdir().unwrap();
    let src = source(dir.path());
    let p = forged(&src.display().to_string());
    let planned = measured_for_reverse(&p, dir.path()).expect("a reversed clip is measured");
    // The render plans with the source's own facts, not the forged 16x16 at 1 fps.
    assert_eq!(dims(&planned), (Some(1920), Some(1080), Some(30.0)));
    // The memory estimate follows: 120 1080p frames held, not 4 tiny ones.
    assert!(
        held_mb(&planned) >= held_mb(&p) + 100,
        "planned {} MiB, as cached {} MiB",
        held_mb(&planned),
        held_mb(&p)
    );
}

#[test]
fn a_source_that_cannot_be_measured_is_planned_at_the_bound() {
    let dir = tempfile::tempdir().unwrap();
    let p = forged(&dir.path().join("missing.mp4").display().to_string());
    let planned = measured_for_reverse(&p, dir.path()).expect("a reversed clip is measured");
    assert_eq!(dims(&planned), (Some(7680), Some(4320), Some(240.0)));
    assert!(held_mb(&planned) > held_mb(&p) + 100);
}

#[test]
fn ordinary_clips_also_use_measured_facts_without_changing_the_project() {
    let dir = tempfile::tempdir().unwrap();
    let src = source(dir.path());
    let mut p = forged(&src.display().to_string());
    if let cut_core::Clip::Media(m) = &mut p.tracks[0].clips[0] {
        m.reverse = false;
    }
    let before = serde_json::to_value(&p).unwrap();
    let planned = measured_for_reverse(&p, dir.path()).expect("ordinary sources are measured too");
    assert_eq!(dims(&planned), (Some(1920), Some(1080), Some(30.0)));
    assert_eq!(serde_json::to_value(&p).unwrap(), before);
}

#[test]
fn reverse_admission_uses_the_render_measurement_and_a_conservative_unknown() {
    let dir = tempfile::tempdir().unwrap();
    let source = source(dir.path());
    let project = forged(&source.display().to_string());
    let asset = &project.assets["a1"];
    let cached = reverse_part_bytes(asset, 30.0, 1.0, 1_000);
    let (part, measured) = measured_reverse_part_bytes(asset, dir.path(), 30.0, 1.0, 1_000);
    assert!(measured);
    assert!(part > cached * 1_000, "measured={part}, cached={cached}");

    let missing = forged(&dir.path().join("missing.mp4").display().to_string());
    let (part, measured) =
        measured_reverse_part_bytes(&missing.assets["a1"], dir.path(), 30.0, 1.0, 1_000);
    assert!(!measured);
    assert!(reverse_refusal(part, Some(8_000_000_000)).is_some());
}
