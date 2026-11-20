//! A source cut off part-way: the asset says so, and the render receipt does
//! not vouch for a picture that stops early.
//!
//! The file keeps its index at the front, so ffprobe reports the full length
//! (8 s) while only the first quarter of the data exists. The proxy job decodes
//! what is there and records the shortfall on the asset; a render of the clip
//! ends with a short video stream under full-length sound, and the per-stream
//! `duration_matches_edl` check fails with a plain reason.

use super::{test_actor, wait_job};
use crate::dispatch::dispatch;
use crate::jobs::JobState;
use crate::state::AppState;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

fn make_clip(dir: &Path, name: &str, keep_percent: u64) -> PathBuf {
    let full = dir.join(format!("full-{name}"));
    let made = std::process::Command::new("ffmpeg")
        .args([
            "-v",
            "error",
            "-y",
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=320x180:rate=25:duration=8",
        ])
        .args([
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440:sample_rate=48000:duration=8",
        ])
        .args([
            "-c:v", "libx264", "-preset", "veryfast", "-pix_fmt", "yuv420p",
        ])
        .args(["-c:a", "aac", "-b:a", "96k", "-movflags", "+faststart"])
        .arg(&full)
        .status()
        .unwrap();
    assert!(made.success());
    let bytes = std::fs::read(&full).unwrap();
    let keep = bytes.len() * keep_percent as usize / 100;
    let out = dir.join(name);
    std::fs::write(&out, &bytes[..keep]).unwrap();
    out
}

/// The whole clip, with an eighth of its bytes zeroed in the middle (the HQ
/// Linux run zeroed 4 MB inside the real phone clip).
fn make_damaged_inside(dir: &Path, name: &str) -> PathBuf {
    let whole = make_clip(dir, &format!("whole-{name}"), 100);
    let mut bytes = std::fs::read(&whole).unwrap();
    let (start, len) = (bytes.len() / 2, bytes.len() / 8);
    bytes[start..start + len].fill(0);
    let out = dir.join(name);
    std::fs::write(&out, &bytes).unwrap();
    out
}

#[tokio::test]
async fn damage_inside_a_source_is_flagged_and_named_by_the_render_receipt() {
    // render.final without a path writes into the process-wide session output
    // folder when another test has set one; hold that folder's lock so the
    // render lands in this project's exports and not in a folder another test
    // deletes when it ends.
    let _output_fixture = crate::output_paths::test_fixture::SessionOutputDirFixture::new();
    let root = tempfile::tempdir().unwrap();
    let state = AppState::new();
    let created = dispatch(
        &state,
        "project.create",
        json!({"name": "damaged", "dir": root.path().join("damaged.cutproj")}),
        test_actor(),
    )
    .await;
    assert!(created.ok, "{:?}", created.error);
    let media = make_damaged_inside(root.path(), "damaged.mp4");
    let (asset, _) = import_and_settle(&state, &media).await;
    let check = &asset["probe"]["decode_check"];
    let version = cut_media::ffmpeg::run_owned_command(
        std::process::Command::new(cut_media::toolpath::ffmpeg())
            .args(["-hide_banner", "-version"]),
        &cut_media::ffmpeg::OwnedProcessControl::bounded(
            std::time::Duration::from_secs(10),
            || false,
        ),
        "read selected ffmpeg version",
    )
    .unwrap();
    assert!(version.status.success());
    let version = String::from_utf8(version.stdout).unwrap();
    let version = version.split_whitespace().nth(2).unwrap();
    let major = version
        .trim_start_matches('n')
        .split('.')
        .next()
        .unwrap()
        .parse::<u32>()
        .ok();
    let older_decoder = major.is_some_and(|major| major < 7);
    if older_decoder {
        // FFmpeg 6.1.1 produces 202 proxy frames for this damaged 200-frame
        // source. A frame-count check cannot see that concealed damage, so
        // the receipt cannot flag it either.
        assert_eq!(check["claimed_frames"], 200, "{check}");
        assert!(
            check["decodable_frames"].as_u64().unwrap() >= 200,
            "{check}"
        );
        assert!(check["code"].is_null(), "{check}");
    } else {
        assert_eq!(check["code"], "decode_gap", "{check}");
        assert!(
            check["warning"]
                .as_str()
                .unwrap()
                .contains(" of 200 frames of picture cannot be read"),
            "{check}"
        );
    }

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
    let receipt_path = job.result.as_ref().unwrap()["receipt"]
        .as_str()
        .unwrap()
        .to_string();
    let receipt: Value = serde_json::from_slice(&std::fs::read(&receipt_path).unwrap()).unwrap();
    let sources = receipt["checks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == "sources_decodable")
        .expect("a decode-checked source puts sources_decodable in the receipt");
    if older_decoder {
        assert_eq!(sources["pass"], true, "{sources}");
    } else {
        let warning = check["warning"].as_str().unwrap();
        assert_eq!(sources["pass"], false);
        assert_eq!(
            sources["details"]["reason"].as_str().unwrap(),
            format!("damaged.mp4: {warning}")
        );
        assert_eq!(receipt["pass"], false);
    }
}

async fn import_and_settle(state: &AppState, media: &Path) -> (Value, crate::jobs::JobRecord) {
    let imported = dispatch(state, "media.import", json!({"path": media}), test_actor()).await;
    assert!(imported.ok, "{:?}", imported.error);
    let job = wait_job(
        state,
        imported.result.unwrap()["job_id"].as_str().unwrap(),
        60,
    )
    .await;
    assert_eq!(job.state, JobState::Done, "{job:?}");
    let proxy_job = job.result.as_ref().unwrap()["proxy_job"]
        .as_str()
        .expect("a video import makes a proxy")
        .to_string();
    let proxy = wait_job(state, &proxy_job, 120).await;
    let s = dispatch(state, "project.state", json!({}), test_actor()).await;
    (s.result.unwrap()["assets"]["a1"].clone(), proxy)
}

#[tokio::test]
async fn a_cut_off_source_is_flagged_on_the_asset() {
    let root = tempfile::tempdir().unwrap();
    let state = AppState::new();
    let created = dispatch(
        &state,
        "project.create",
        json!({"name": "cut-off", "dir": root.path().join("cut-off.cutproj")}),
        test_actor(),
    )
    .await;
    assert!(created.ok, "{:?}", created.error);
    let media = make_clip(root.path(), "cut-off.mp4", 25);
    let (asset, proxy_job) = import_and_settle(&state, &media).await;
    assert_eq!(
        asset["probe"]["duration_ms"], 8000,
        "ffprobe still claims 8 s"
    );
    let check = &asset["probe"]["decode_check"];
    assert_eq!(check["code"], "decode_short", "{asset}");
    let warning = check["warning"].as_str().unwrap();
    assert!(warning.starts_with("Only the first "), "{warning}");
    assert!(warning.contains(" of 8.0 s of "), "{warning}");
    assert!(
        check["decodable_picture_ms"].as_u64().unwrap() < 4000,
        "{check}"
    );
    assert!(
        proxy_job.result.as_ref().unwrap()["warnings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|w| w.as_str() == Some(warning)),
        "the proxy job carries the same warning: {proxy_job:?}"
    );
}

#[tokio::test]
async fn a_render_of_a_cut_off_source_fails_the_per_stream_duration_check() {
    // As above: keep the default render in this project's exports.
    let _output_fixture = crate::output_paths::test_fixture::SessionOutputDirFixture::new();
    let root = tempfile::tempdir().unwrap();
    let state = AppState::new();
    let created = dispatch(
        &state,
        "project.create",
        json!({"name": "cut-off-render", "dir": root.path().join("cut-off-render.cutproj")}),
        test_actor(),
    )
    .await;
    assert!(created.ok, "{:?}", created.error);
    let media = make_clip(root.path(), "cut-off.mp4", 25);
    import_and_settle(&state, &media).await;
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
    let receipt_path = job.result.as_ref().unwrap()["receipt"]
        .as_str()
        .unwrap()
        .to_string();
    let receipt: Value = serde_json::from_slice(&std::fs::read(&receipt_path).unwrap()).unwrap();
    assert_eq!(
        receipt["duration_ms"], 8000,
        "the container is as long as the sound"
    );
    let duration = receipt["checks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == "duration_matches_edl")
        .expect("duration check in the receipt");
    assert_eq!(duration["pass"], false, "{duration}");
    let reason = duration["details"]["reason"].as_str().unwrap();
    assert!(reason.starts_with("The video in the file is "), "{reason}");
    assert!(
        reason.contains("but the timeline is 8.00 s (200 frames)"),
        "{reason}"
    );
    assert_eq!(receipt["pass"], false);
}

#[tokio::test]
async fn a_whole_source_has_no_decode_warning() {
    let root = tempfile::tempdir().unwrap();
    let state = AppState::new();
    let created = dispatch(
        &state,
        "project.create",
        json!({"name": "whole", "dir": root.path().join("whole.cutproj")}),
        test_actor(),
    )
    .await;
    assert!(created.ok, "{:?}", created.error);
    let media = make_clip(root.path(), "whole.mp4", 100);
    let (asset, proxy_job) = import_and_settle(&state, &media).await;
    let check = &asset["probe"]["decode_check"];
    assert_eq!(check["checked_by"], "proxy", "{asset}");
    assert!(check.get("warning").is_none(), "{check}");
    assert_eq!(proxy_job.state, JobState::Done);
}
