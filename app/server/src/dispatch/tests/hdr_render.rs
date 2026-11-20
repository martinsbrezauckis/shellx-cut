//! An HDR (HLG, 10-bit) source renders to SDR BT.709 with true tags, and the
//! plan, the job result and the receipt say what happened to its colour.
//!
//! The HQ runs found 10-bit HLG sources written as 8-bit video that still
//! carried the HLG tags: players treat such a file as HDR and it bands.

use super::{test_actor, wait_job};
use crate::dispatch::dispatch;
use crate::jobs::JobState;
use crate::state::AppState;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

/// A 10-bit HEVC clip tagged HLG/BT.2020 (tags written by the bitstream
/// filter, because libx265 alone leaves transfer/primaries unset).
fn make_hlg(dir: &Path) -> PathBuf {
    let raw = dir.join("hlg-untagged.mp4");
    let ok = std::process::Command::new("ffmpeg")
        .args([
            "-v",
            "error",
            "-y",
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=320x180:rate=30:duration=1",
        ])
        .args([
            "-vf",
            "format=yuv420p10le",
            "-c:v",
            "libx265",
            "-preset",
            "ultrafast",
        ])
        .args([
            "-x265-params",
            "log-level=error",
            "-profile:v",
            "main10",
            "-tag:v",
            "hvc1",
        ])
        .arg(&raw)
        .status()
        .unwrap();
    assert!(ok.success());
    let out = dir.join("hlg.mp4");
    let ok = std::process::Command::new("ffmpeg")
        .args(["-v", "error", "-y", "-i"])
        .arg(&raw)
        .args(["-c", "copy", "-bsf:v"])
        .arg("hevc_metadata=colour_primaries=9:transfer_characteristics=18:matrix_coefficients=9:video_full_range_flag=0")
        .arg(&out)
        .status()
        .unwrap();
    assert!(ok.success());
    out
}

fn ffprobe_video(path: &str) -> Value {
    let out = std::process::Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-select_streams",
            "v:0",
            "-print_format",
            "json",
        ])
        .args([
            "-show_entries",
            "stream=pix_fmt,profile,color_transfer,color_primaries,color_space",
        ])
        .arg(path)
        .output()
        .unwrap();
    serde_json::from_slice::<Value>(&out.stdout).unwrap()["streams"][0].clone()
}

#[tokio::test]
async fn an_hlg_source_exports_as_true_sdr_and_every_surface_says_so() {
    let root = tempfile::tempdir().unwrap();
    let state = AppState::new();
    let created = dispatch(
        &state,
        "project.create",
        json!({"name": "hlg", "dir": root.path().join("hlg.cutproj")}),
        test_actor(),
    )
    .await;
    assert!(created.ok, "{:?}", created.error);
    let media = make_hlg(root.path());
    let imported = dispatch(
        &state,
        "media.import",
        json!({"path": media, "proxy": false}),
        test_actor(),
    )
    .await;
    assert!(imported.ok, "{:?}", imported.error);
    let job = wait_job(
        &state,
        imported.result.unwrap()["job_id"].as_str().unwrap(),
        60,
    )
    .await;
    assert_eq!(job.state, JobState::Done, "{job:?}");

    let plan = dispatch(
        &state,
        "render.final",
        json!({"dry_run": true, "hardware": "off"}),
        test_actor(),
    )
    .await;
    assert!(plan.ok, "{:?}", plan.error);
    let color = plan.result.unwrap()["encode"]["color"].clone();
    assert_eq!(color["output"], "SDR BT.709", "{color}");
    assert_eq!(color["processing_bit_depth"], 8);
    assert_eq!(
        color["sources"][0]["handling"], "tone_mapped_to_sdr",
        "{color}"
    );
    assert_eq!(color["sources"][0]["transfer"], "arib-std-b67");
    assert_eq!(color["sources"][0]["bit_depth"], 10);
    assert_eq!(
        color["note"],
        "hlg.mp4 is HDR (HLG, 10-bit) and was converted to SDR BT.709 for this export; exports cannot keep HDR yet."
    );

    let rendered = dispatch(
        &state,
        "render.final",
        json!({"hardware": "off"}),
        test_actor(),
    )
    .await;
    assert!(rendered.ok, "{:?}", rendered.error);
    let job = wait_job(
        &state,
        rendered.result.unwrap()["job_id"].as_str().unwrap(),
        180,
    )
    .await;
    assert_eq!(job.state, JobState::Done, "{job:?}");
    let result = job.result.unwrap();
    assert_eq!(
        result["encode"]["color"], color,
        "the job result says the same"
    );
    let receipt: Value =
        serde_json::from_slice(&std::fs::read(result["receipt"].as_str().unwrap()).unwrap())
            .unwrap();
    assert_eq!(
        receipt["encode"]["color"], color,
        "the receipt says the same"
    );
    let out = ffprobe_video(result["path"].as_str().unwrap());
    assert_eq!(out["pix_fmt"], "yuv420p", "{out}");
    assert_eq!(
        out["color_transfer"], "bt709",
        "8-bit output never carries HLG tags: {out}"
    );
    assert_eq!(out["color_primaries"], "bt709", "{out}");
    assert_eq!(out["color_space"], "bt709", "{out}");
}
