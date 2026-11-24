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
            "stream=codec_name,pix_fmt,profile,color_transfer,color_primaries,color_space",
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
        "hlg.mp4 is HDR (HLG, 10-bit) and was converted to SDR BT.709 for this export; choose Keep HDR to export 10-bit HDR."
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

fn make_pq_hdr10(dir: &Path) -> PathBuf {
    let out = dir.join("pq_hdr10.mp4");
    let ok = std::process::Command::new("ffmpeg")
        .args([
            "-v",
            "error",
            "-y",
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=320x180:rate=30:duration=1",
            "-vf",
            "format=yuv420p,zscale=tin=bt709:pin=bt709:min=bt709:rin=tv:t=smpte2084:p=bt2020:m=bt2020nc:r=tv:npl=203,format=yuv420p10le",
            "-c:v",
            "libx265",
            "-preset",
            "ultrafast",
            "-profile:v",
            "main10",
            "-tag:v",
            "hvc1",
            "-x265-params",
            "log-level=error:colorprim=bt2020:transfer=smpte2084:colormatrix=bt2020nc:hdr-opt=1:repeat-headers=1:master-display=G(13250,34500)B(7500,3000)R(34000,16000)WP(15635,16450)L(10000000,1):max-cll=1000,400",
            "-color_primaries",
            "bt2020",
            "-color_trc",
            "smpte2084",
            "-colorspace",
            "bt2020nc",
            "-color_range",
            "tv",
        ])
        .arg(&out)
        .status()
        .unwrap();
    assert!(ok.success());
    out
}

fn make_sdr(dir: &Path) -> PathBuf {
    let out = dir.join("sdr.mp4");
    let ok = std::process::Command::new("ffmpeg")
        .args([
            "-v",
            "error",
            "-y",
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=320x180:rate=30:duration=1",
            "-vf",
            "format=yuv420p",
            "-c:v",
            "libx264",
            "-preset",
            "ultrafast",
            "-color_primaries",
            "bt709",
            "-color_trc",
            "bt709",
            "-colorspace",
            "bt709",
            "-color_range",
            "tv",
        ])
        .arg(&out)
        .status()
        .unwrap();
    assert!(ok.success());
    out
}

fn ffprobe_video_and_frame0(path: &str) -> (Value, Value) {
    let out = std::process::Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-select_streams",
            "v:0",
            "-print_format",
            "json",
            "-show_streams",
            "-show_frames",
            "-read_intervals",
            "%+#1",
        ])
        .arg(path)
        .output()
        .unwrap();
    let parsed: Value = serde_json::from_slice(&out.stdout).unwrap();
    (parsed["streams"][0].clone(), parsed["frames"][0].clone())
}

#[tokio::test]
async fn hlg_keep_hdr_exports_10bit_hlg_bt2020_and_reports_plan_job_receipt() {
    let root = tempfile::tempdir().unwrap();
    let state = AppState::new();
    let created = dispatch(
        &state,
        "project.create",
        json!({"name": "hlg_keep", "dir": root.path().join("hlg_keep.cutproj")}),
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
        json!({"color": "keep_hdr", "hardware": "off", "dry_run": true}),
        test_actor(),
    )
    .await;
    assert!(plan.ok, "{:?}", plan.error);
    let color = plan.result.unwrap()["encode"]["color"].clone();
    assert!(
        color["output"].as_str().unwrap_or("").contains("HLG")
            && color["output"].as_str().unwrap_or("").contains("10-bit")
            && color["output"].as_str().unwrap_or("").contains("BT.2020"),
        "{color}"
    );
    assert_eq!(color["processing_bit_depth"], 10, "{color}");
    assert_eq!(color["file_bit_depth"], 10, "{color}");
    assert_eq!(color["sources"][0]["handling"], "kept_hdr", "{color}");

    let rendered = dispatch(
        &state,
        "render.final",
        json!({"color": "keep_hdr", "hardware": "off", "profile": "silent_screen_demo"}),
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
    assert_eq!(result["encode"]["color"], color);
    let receipt: Value =
        serde_json::from_slice(&std::fs::read(result["receipt"].as_str().unwrap()).unwrap())
            .unwrap();
    assert_eq!(receipt["encode"]["color"], color);
    let out = ffprobe_video(result["path"].as_str().unwrap());
    assert_eq!(out["codec_name"], "hevc", "{out}");
    assert_eq!(out["profile"], "Main 10", "{out}");
    assert_eq!(out["pix_fmt"], "yuv420p10le", "{out}");
    assert_eq!(out["color_transfer"], "arib-std-b67", "{out}");
    assert_eq!(out["color_primaries"], "bt2020", "{out}");
    assert_eq!(out["color_space"], "bt2020nc", "{out}");
}

#[tokio::test]
async fn pq_hdr10_keep_hdr_carries_static_metadata_in_hevc_and_av1() {
    let root = tempfile::tempdir().unwrap();
    let state = AppState::new();
    let created = dispatch(
        &state,
        "project.create",
        json!({"name": "pq_keep", "dir": root.path().join("pq_keep.cutproj")}),
        test_actor(),
    )
    .await;
    assert!(created.ok, "{:?}", created.error);
    let media = make_pq_hdr10(root.path());
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

    for (fmt, expected_codec) in [("hevc", "hevc"), ("av1", "av1")] {
        let rendered = dispatch(
            &state,
            "render.final",
            json!({"color": "keep_hdr", "format": fmt, "preset": "draft", "hardware": "off", "profile": "silent_screen_demo"}),
            test_actor(),
        )
        .await;
        assert!(rendered.ok, "{fmt}: {:?}", rendered.error);
        let job = wait_job(
            &state,
            rendered.result.unwrap()["job_id"].as_str().unwrap(),
            180,
        )
        .await;
        assert_eq!(job.state, JobState::Done, "{fmt}: {job:?}");
        let result = job.result.unwrap();
        let (stream, frame0) = ffprobe_video_and_frame0(result["path"].as_str().unwrap());
        assert_eq!(stream["codec_name"], expected_codec, "{fmt}: {stream}");
        assert_eq!(stream["pix_fmt"], "yuv420p10le", "{fmt}: {stream}");
        assert_eq!(stream["color_transfer"], "smpte2084", "{fmt}: {stream}");
        assert_eq!(stream["color_primaries"], "bt2020", "{fmt}: {stream}");
        assert_eq!(stream["color_space"], "bt2020nc", "{fmt}: {stream}");
        let has_md = |sd: &Value| {
            sd.as_array().is_some_and(|arr| {
                arr.iter()
                    .any(|x| x["side_data_type"] == "Mastering display metadata")
            })
        };
        let has_cll = |sd: &Value| {
            sd.as_array().is_some_and(|arr| {
                arr.iter()
                    .any(|x| x["side_data_type"] == "Content light level metadata")
            })
        };
        assert!(
            has_md(&stream["side_data_list"]) || has_md(&frame0["side_data_list"]),
            "{fmt}: missing mastering display in stream={} frame0={}",
            stream["side_data_list"],
            frame0["side_data_list"]
        );
        assert!(
            has_cll(&stream["side_data_list"]) || has_cll(&frame0["side_data_list"]),
            "{fmt}: missing MaxCLL/MaxFALL in stream={} frame0={}",
            stream["side_data_list"],
            frame0["side_data_list"]
        );
    }
}

#[tokio::test]
async fn keep_hdr_refuses_incompatible_format_h264_and_sdr_only_timeline() {
    let root = tempfile::tempdir().unwrap();
    let state = AppState::new();
    dispatch(
        &state,
        "project.create",
        json!({"name": "refuse_hdr", "dir": root.path().join("refuse_hdr.cutproj")}),
        test_actor(),
    )
    .await;
    let sdr = make_sdr(root.path());
    let imp = dispatch(
        &state,
        "media.import",
        json!({"path": sdr, "proxy": false}),
        test_actor(),
    )
    .await;
    let job = wait_job(&state, imp.result.unwrap()["job_id"].as_str().unwrap(), 60).await;
    assert_eq!(job.state, JobState::Done);

    let no_hdr = dispatch(
        &state,
        "render.final",
        json!({"color": "keep_hdr", "format": "hevc", "hardware": "off"}),
        test_actor(),
    )
    .await;
    assert!(!no_hdr.ok, "SDR-only timeline must refuse keep_hdr");
    let msg = no_hdr.error.as_ref().unwrap().message.to_lowercase();
    assert!(msg.contains("hdr"), "{:?}", no_hdr.error);

    let hlg = make_hlg(root.path());
    let imp2 = dispatch(
        &state,
        "media.import",
        json!({"path": hlg, "proxy": false}),
        test_actor(),
    )
    .await;
    let job2 = wait_job(&state, imp2.result.unwrap()["job_id"].as_str().unwrap(), 60).await;
    assert_eq!(job2.state, JobState::Done);

    let bad_fmt = dispatch(
        &state,
        "render.final",
        json!({"color": "keep_hdr", "format": "h264", "hardware": "off"}),
        test_actor(),
    )
    .await;
    assert!(!bad_fmt.ok, "H.264 must refuse keep_hdr");
    let err = bad_fmt.error.unwrap();
    assert!(
        err.message.contains("H.265") || err.message.contains("HEVC"),
        "{err:?}"
    );
    assert!(err.suggested_action.is_some(), "{err:?}");
}

#[tokio::test]
async fn mixed_hlg_pq_sdr_and_graded_hdr_clip_behave_honestly() {
    let root = tempfile::tempdir().unwrap();
    let state = AppState::new();
    dispatch(
        &state,
        "project.create",
        json!({"name": "mixed_hdr", "dir": root.path().join("mixed_hdr.cutproj")}),
        test_actor(),
    )
    .await;
    let hlg = make_hlg(root.path());
    let pq = make_pq_hdr10(root.path());
    let sdr = make_sdr(root.path());
    for (idx, p) in [&hlg, &pq, &sdr].iter().enumerate() {
        let imp = dispatch(
            &state,
            "media.import",
            json!({"path": p, "proxy": false}),
            test_actor(),
        )
        .await;
        assert!(imp.ok, "{:?}", imp.error);
        let imp_res = imp.result.unwrap();
        let asset_id = imp_res["asset_id"].as_str().unwrap().to_string();
        let job = wait_job(&state, imp_res["job_id"].as_str().unwrap(), 60).await;
        assert_eq!(job.state, JobState::Done);
        if idx > 0 {
            let ins = dispatch(
                &state,
                "edit.insert",
                json!({"track": "v1", "asset": asset_id, "at_ms": idx as u64 * 1000}),
                test_actor(),
            )
            .await;
            assert!(ins.ok, "{:?}", ins.error);
        }
    }
    let st = dispatch(&state, "project.state", json!({}), test_actor()).await;
    let clips = st.result.unwrap()["tracks"][0]["clips"]
        .as_array()
        .unwrap()
        .clone();
    let hlg_clip_id = clips[0]["id"].as_str().unwrap().to_string();

    let mixed_plan = dispatch(
        &state,
        "render.final",
        json!({"color": "keep_hdr", "hardware": "off", "dry_run": true}),
        test_actor(),
    )
    .await;
    assert!(mixed_plan.ok, "{:?}", mixed_plan.error);
    let mcolor = mixed_plan.result.unwrap()["encode"]["color"].clone();
    assert!(
        mcolor["output"].as_str().unwrap().contains("HLG"),
        "{mcolor}"
    );
    assert_eq!(mcolor["sources"].as_array().unwrap().len(), 3, "{mcolor}");

    let rendered = dispatch(
        &state,
        "render.final",
        json!({"color": "keep_hdr", "preset": "draft", "hardware": "off", "profile": "silent_screen_demo"}),
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
    let out = ffprobe_video(job.result.unwrap()["path"].as_str().unwrap());
    assert_eq!(out["pix_fmt"], "yuv420p10le", "{out}");
    assert_eq!(out["color_transfer"], "arib-std-b67", "{out}");
    assert_eq!(out["color_primaries"], "bt2020", "{out}");
    assert_eq!(out["color_space"], "bt2020nc", "{out}");

    // Apply an SDR-only grade to the HLG clip -> notice explains honestly
    let g = dispatch(
        &state,
        "edit.grade",
        json!({"clip": hlg_clip_id, "brightness": 0.05}),
        test_actor(),
    )
    .await;
    assert!(g.ok, "{:?}", g.error);
    let graded_plan = dispatch(
        &state,
        "render.final",
        json!({"color": "keep_hdr", "hardware": "off", "dry_run": true}),
        test_actor(),
    )
    .await;
    assert!(graded_plan.ok, "{:?}", graded_plan.error);
    let graded_color = graded_plan.result.unwrap()["encode"]["color"].clone();
    let note = graded_color["note"].as_str().unwrap_or("");
    assert!(
        note.contains("hlg.mp4") && note.contains("SDR"),
        "expected SDR grade notice naming clip: {graded_color}"
    );
    assert_eq!(graded_color["sources"][0]["handling"], "hdr_graded_in_sdr");

    let graded_render = dispatch(
        &state,
        "render.final",
        json!({"color": "keep_hdr", "preset": "draft", "hardware": "off", "profile": "silent_screen_demo"}),
        test_actor(),
    )
    .await;
    assert!(graded_render.ok, "{:?}", graded_render.error);
    let gjob = wait_job(
        &state,
        graded_render.result.unwrap()["job_id"].as_str().unwrap(),
        180,
    )
    .await;
    assert_eq!(gjob.state, JobState::Done, "{gjob:?}");
    let gout = ffprobe_video(gjob.result.unwrap()["path"].as_str().unwrap());
    assert_eq!(gout["pix_fmt"], "yuv420p10le", "{gout}");
    assert_eq!(gout["color_transfer"], "arib-std-b67", "{gout}");
}
