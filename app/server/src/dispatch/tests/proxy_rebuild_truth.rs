//! A rebuilt editing proxy carries what the first one did: the HDR tone map
//! (so the preview matches the export) and the decode check (so a cut-off
//! source keeps its warning).

use super::{test_actor, wait_job};
use crate::dispatch::{dispatch, update_asset};
use crate::jobs::JobState;
use crate::state::AppState;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

fn ffmpeg(args: &[&str]) {
    let ok = std::process::Command::new("ffmpeg")
        .args(["-v", "error", "-y"])
        .args(args)
        .status()
        .unwrap();
    assert!(ok.success(), "ffmpeg {args:?}");
}

/// 1 s of 10-bit HEVC tagged HLG/BT.2020 (tags by the bitstream filter,
/// because libx265 leaves transfer and primaries unset).
fn make_hlg(dir: &Path) -> PathBuf {
    let raw = dir.join("hlg-raw.mp4");
    let out = dir.join("hlg.mp4");
    ffmpeg(&[
        "-f",
        "lavfi",
        "-i",
        "testsrc2=size=320x180:rate=30:duration=1",
        "-vf",
        "format=yuv420p10le",
        "-c:v",
        "libx265",
        "-preset",
        "ultrafast",
        "-x265-params",
        "log-level=error",
        "-profile:v",
        "main10",
        "-tag:v",
        "hvc1",
        raw.to_str().unwrap(),
    ]);
    ffmpeg(&["-i", raw.to_str().unwrap(), "-c", "copy", "-bsf:v",
        "hevc_metadata=colour_primaries=9:transfer_characteristics=18:matrix_coefficients=9:video_full_range_flag=0",
        out.to_str().unwrap()]);
    out
}

/// The first quarter of an 8 s clip whose index is at the front.
fn make_cut_off(dir: &Path) -> PathBuf {
    let full = dir.join("full.mp4");
    ffmpeg(&[
        "-f",
        "lavfi",
        "-i",
        "testsrc2=size=320x180:rate=25:duration=8",
        "-c:v",
        "libx264",
        "-preset",
        "veryfast",
        "-pix_fmt",
        "yuv420p",
        "-movflags",
        "+faststart",
        full.to_str().unwrap(),
    ]);
    let bytes = std::fs::read(&full).unwrap();
    let out = dir.join("cut-off.mp4");
    std::fs::write(&out, &bytes[..bytes.len() / 4]).unwrap();
    out
}

fn transfer_of(path: &Path) -> String {
    let out = std::process::Command::new("ffprobe")
        .args(["-v", "error", "-select_streams", "v:0", "-show_entries"])
        .args(["stream=color_transfer", "-of", "default=nw=1:nk=1"])
        .arg(path)
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

async fn import(state: &AppState, media: &Path) {
    let imported = dispatch(state, "media.import", json!({"path": media}), test_actor()).await;
    assert!(imported.ok, "{:?}", imported.error);
    let job = wait_job(
        state,
        imported.result.unwrap()["job_id"].as_str().unwrap(),
        60,
    )
    .await;
    assert_eq!(job.state, JobState::Done, "{job:?}");
    let proxy_job = job.result.unwrap()["proxy_job"]
        .as_str()
        .unwrap()
        .to_string();
    wait_job(state, &proxy_job, 120).await;
}

#[tokio::test]
async fn a_rebuilt_proxy_is_tone_mapped_and_rechecks_decoding() {
    let root = tempfile::tempdir().unwrap();
    let state = AppState::new();
    let dir = root.path().join("rebuild.cutproj");
    let created = dispatch(
        &state,
        "project.create",
        json!({"name": "rebuild", "dir": dir}),
        test_actor(),
    )
    .await;
    assert!(created.ok, "{:?}", created.error);
    import(&state, &make_hlg(root.path())).await;
    import(&state, &make_cut_off(root.path())).await;
    let project_dir = {
        let guard = state.project.read().await;
        guard.as_ref().unwrap().dir.clone()
    };

    // Lose the generated caches (as after a purge; a real purge keeps outputs
    // younger than its retention age) and the decode checks, so only the
    // rebuild can bring them back.
    for id in ["a1", "a2"] {
        for rel in [format!("proxies/{id}.mp4"), format!("filmstrip/{id}.jpg")] {
            let _ = std::fs::remove_file(project_dir.join(rel));
        }
    }
    for id in ["a1", "a2"] {
        update_asset(&state, id, |a| {
            if let Some(Value::Object(p)) = a.probe.as_mut() {
                p.remove("decode_check");
            }
        })
        .await
        .unwrap();
    }
    let started = dispatch(
        &state,
        "project.cache_rebuild",
        json!({"asset_ids": ["a1", "a2"]}),
        test_actor(),
    )
    .await;
    assert!(started.ok, "{:?}", started.error);
    let started = started.result.unwrap();
    let job_id = started["job_id"]
        .as_str()
        .unwrap_or_else(|| panic!("no rebuild job: {started}"))
        .to_string();
    let job = wait_job(&state, &job_id, 240).await;
    assert_eq!(job.state, JobState::Done, "{job:?}");

    assert_eq!(
        transfer_of(&project_dir.join("proxies/a1.mp4")),
        "bt709",
        "the rebuilt HLG proxy is tone-mapped to SDR BT.709"
    );
    let s = dispatch(&state, "project.state", json!({}), test_actor()).await;
    let assets = s.result.unwrap()["assets"].clone();
    assert_eq!(
        assets["a2"]["probe"]["decode_check"]["code"], "decode_short",
        "{assets}"
    );
    assert!(assets["a1"]["probe"]["decode_check"]["checked_by"] == "proxy");
    let warnings = job.result.unwrap()["warnings"].clone();
    assert!(
        warnings.as_array().unwrap().iter().any(|w| w
            .as_str()
            .unwrap_or_default()
            .starts_with("Only the first ")),
        "the rebuild job says the source is cut off: {warnings}"
    );
}
