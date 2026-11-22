//! Source decoding must not be inferred from a rate-converted proxy's count.
use super::{test_actor, wait_job};
use crate::{dispatch::dispatch, jobs::JobState, state::AppState};
use serde_json::{json, Value};
use std::path::Path;

fn camera(path: &Path, clustered: bool, damage: &str) {
    let filter = if clustered {
        // Capture-clock bursts: four distinct pictures in each 30 Hz tick.
        // The editing transcode coalesces some near-identical timestamps.
        "settb=1/120000,setpts=floor(N/4)*4000+mod(N\\,4)+16920,select='not(eq(mod(n,7),0))'"
    } else {
        "select='not(eq(mod(n,7),0))',setpts=PTS+0.137/TB"
    };
    let status = std::process::Command::new("ffmpeg")
        .args([
            "-v",
            "error",
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=640x480:rate=120:duration=39",
            "-vf",
            filter,
            "-fps_mode",
            "passthrough",
            "-enc_time_base",
            "1/120000",
            "-c:v",
            "libx264",
            "-preset",
            "ultrafast",
            "-movflags",
            "+faststart",
            "-y",
        ])
        .arg(path)
        .status()
        .unwrap();
    assert!(status.success());
    let mut bytes = std::fs::read(path).unwrap();
    match damage {
        "truncated" => bytes.truncate(bytes.len() / 4),
        "inside" => {
            let start = bytes.len() / 2;
            let end = start + bytes.len() / 8;
            bytes[start..end].fill(0);
        }
        _ => {}
    }
    std::fs::write(path, bytes).unwrap();
}

async fn check_camera(clustered: bool, damage: &str, cfr: bool) {
    let _output = crate::output_paths::test_fixture::SessionOutputDirFixture::new();
    let root = tempfile::tempdir().unwrap();
    let state = AppState::new();
    let created = dispatch(
        &state,
        "project.create",
        json!({"name": "camera", "dir": root.path().join("camera.cutproj")}),
        test_actor(),
    )
    .await;
    assert!(created.ok, "{:?}", created.error);
    let source = root.path().join("camera.mp4");
    camera(&source, clustered, damage);
    if cfr {
        let variable = root.path().join("variable.mp4");
        std::fs::rename(&source, &variable).unwrap();
        assert!(std::process::Command::new("ffmpeg")
            .args(["-v", "error", "-i"])
            .arg(&variable)
            .args([
                "-r",
                "30",
                "-fps_mode",
                "cfr",
                "-c:v",
                "libx264",
                "-preset",
                "ultrafast",
                "-y"
            ])
            .arg(&source)
            .status()
            .unwrap()
            .success());
    }
    let imported = dispatch(
        &state,
        "media.import",
        json!({"path": source}),
        test_actor(),
    )
    .await;
    assert!(imported.ok, "{:?}", imported.error);
    let job = wait_job(
        &state,
        imported.result.unwrap()["job_id"].as_str().unwrap(),
        120,
    )
    .await;
    assert_eq!(job.state, JobState::Done, "{job:?}");
    let proxy = wait_job(
        &state,
        job.result.unwrap()["proxy_job"].as_str().unwrap(),
        180,
    )
    .await;
    assert_eq!(proxy.state, JobState::Done, "{proxy:?}");
    let project = dispatch(&state, "project.state", json!({}), test_actor())
        .await
        .result
        .unwrap();
    let probe = &project["assets"]["a1"]["probe"];
    let check = &probe["decode_check"];
    eprintln!(
        "CAMERA clustered={clustered} damage={damage} fps={} nominal={} vfr={} check={check}",
        probe["fps"], probe["nominal_fps"], probe["variable_frame_rate"]
    );
    assert_eq!(probe["width"], 640);
    assert_eq!(probe["height"], 480);
    assert_eq!(
        probe["variable_frame_rate"].as_bool().unwrap_or(false),
        !cfr
    );
    if cfr {
        assert_eq!(probe["fps"], 30.0);
    } else {
        assert!((100.0..105.0).contains(&probe["fps"].as_f64().unwrap()));
        assert!(
            probe["raw"]["streams"][0]["start_time"]
                .as_str()
                .unwrap()
                .parse::<f64>()
                .unwrap()
                > 0.1
        );
    }
    let proxy_path = root
        .path()
        .join("camera.cutproj")
        .join(project["assets"]["a1"]["proxy"].as_str().unwrap());
    let cheap = cut_media::decode_check::decode_check(
        probe,
        &cut_media::probe::stream_extents(&proxy_path).unwrap(),
    );
    eprintln!("CHEAP clustered={clustered} damage={damage} cfr={cfr}: {cheap}");
    // Supported encoders can merge OR duplicate capture-clock timestamps.
    // Test the promised outcome, not one encoder's proxy frame count.
    if cheap["code"] == "decode_gap" {
        assert_eq!(
            check["source_count_confirmation"]["confirmed"], true,
            "{check}"
        );
    }
    let damaged = !damage.is_empty();
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
        240,
    )
    .await;
    assert_eq!(job.state, JobState::Done, "{job:?}");
    let receipt: Value = serde_json::from_slice(
        &std::fs::read(job.result.unwrap()["receipt"].as_str().unwrap()).unwrap(),
    )
    .unwrap();
    let sources = receipt["checks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == "sources_decodable")
        .unwrap();
    eprintln!("SOURCES clustered={clustered} damage={damage} cfr={cfr}: {sources}");
    // Report both paths even in the red run: the import fact supplies the
    // Assets badge, and the export copies its warning into sources_decodable.
    assert_eq!(
        sources["pass"], !damaged,
        "import={check}; export={sources}"
    );
    if damaged {
        assert!(check["warning"].is_string(), "{check}");
        assert_eq!(
            check["code"],
            if damage == "truncated" {
                "decode_short"
            } else {
                "decode_gap"
            },
            "{check}"
        );
        if damage == "inside" {
            assert!(check["warning"]
                .as_str()
                .unwrap()
                .contains("frames of picture cannot be read"));
        }
    } else {
        assert!(check.get("warning").is_none(), "{check}");
    }
}

#[tokio::test]
async fn regular_vfr_camera_has_no_import_or_export_damage_warning() {
    check_camera(false, "", false).await;
}
#[tokio::test]
async fn capture_clock_vfr_camera_has_no_import_or_export_damage_warning() {
    check_camera(true, "", false).await;
}
#[tokio::test]
async fn truncated_vfr_camera_is_flagged_on_import_and_export() {
    check_camera(true, "truncated", false).await;
}
#[tokio::test]
async fn internally_damaged_vfr_camera_is_flagged_on_import_and_export() {
    check_camera(true, "inside", false).await;
}
#[tokio::test]
async fn constant_rate_copy_has_no_import_or_export_damage_warning() {
    check_camera(true, "", true).await;
}
