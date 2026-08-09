//! Real-FFmpeg regression for sparse/VFR recorder export timing.

use std::path::{Path, PathBuf};
use std::time::Duration;

use super::super::dispatch;
use super::test_actor;
use crate::jobs::JobState;
use crate::state::AppState;
use serde_json::{json, Value};

fn run_ffmpeg(args: &[&str], out: &Path) {
    let status = std::process::Command::new(cut_media::toolpath::ffmpeg())
        .args(["-nostats", "-loglevel", "error", "-y"])
        .args(args)
        .arg(out)
        .status()
        .expect("run fixture ffmpeg");
    assert!(
        status.success(),
        "fixture ffmpeg failed for {}",
        out.display()
    );
}

/// A short VFR source with 60fps nominal timing but an actual ~30fps cadence.
/// It models sparse ScreenCaptureKit output without requiring a macOS desktop.
fn synth_sparse_30fps_video(out: &Path) {
    run_ffmpeg(
        &[
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=160x90:rate=60:duration=7.95",
            "-vf",
            "select='eq(n,0)+eq(n,1)+not(mod(n,2))',setpts=PTS",
            "-fps_mode",
            "vfr",
            "-an",
            "-c:v",
            "libx264",
            "-pix_fmt",
            "yuv420p",
        ],
        out,
    );
}

fn synth_tone(out: &Path) {
    run_ffmpeg(
        &[
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=880:sample_rate=48000:duration=8",
            "-ac",
            "2",
            "-c:a",
            "pcm_s16le",
        ],
        out,
    );
}

fn ffprobe(path: &Path) -> Value {
    let output = std::process::Command::new(cut_media::toolpath::ffprobe())
        .args([
            "-v",
            "error",
            "-count_frames",
            "-show_entries",
            "stream=codec_type,r_frame_rate,avg_frame_rate,duration,nb_read_frames:format=duration",
            "-of",
            "json",
        ])
        .arg(path)
        .output()
        .expect("run fixture ffprobe");
    assert!(
        output.status.success(),
        "ffprobe failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("parse fixture ffprobe")
}

async fn wait_export(state: &AppState, job_id: &str) -> Value {
    for _ in 0..2_400 {
        let record = state.jobs.get(job_id).expect("export job record");
        match record.state {
            JobState::Done => return record.result.expect("completed export result"),
            JobState::Failed => panic!("export job failed: {:?}", record.error),
            _ => tokio::time::sleep(Duration::from_millis(10)).await,
        }
    }
    panic!("export job {job_id} did not complete")
}

#[tokio::test]
async fn sparse_30fps_capture_export_uses_plan_rate_not_nominal_stream_rate() {
    let temp = tempfile::tempdir().unwrap();
    let project_dir = temp.path().join("export_sparse_timebase.cutproj");
    let state = AppState::new();
    let created = dispatch(
        &state,
        "project.create",
        json!({"name": "export_sparse_timebase", "dir": project_dir}),
        test_actor(),
    )
    .await;
    assert!(created.ok, "project create failed: {:?}", created.error);

    let capture = crate::screen_record::screen_record_cache_dir(&project_dir)
        .unwrap()
        .join("cap-sparse-timebase");
    std::fs::create_dir_all(&capture).unwrap();
    let source = capture.join("source.mp4");
    let system = capture.join("system.wav");
    synth_sparse_30fps_video(&source);
    let source_facts = ffprobe(&source);
    let source_video = source_facts["streams"]
        .as_array()
        .unwrap()
        .iter()
        .find(|stream| stream["codec_type"] == "video")
        .unwrap();
    assert_eq!(source_video["r_frame_rate"], "60/1");
    assert_ne!(source_video["avg_frame_rate"], "60/1");
    synth_tone(&system);
    std::fs::write(
        capture.join("system-audio.json"),
        br#"{"schema":"shellx-cut/system-audio-timing/1","first_packet_offset_ms":0}"#,
    )
    .unwrap();
    std::fs::write(
        capture.join("project.json"),
        serde_json::to_vec(&json!({
            "schema": "shellx-record/1",
            "settings": {"width": 160, "height": 90, "fps": 30.0, "audio_rate": 48000},
            "source_video": source.display().to_string(),
            "events": {
                "duration_ms": 7_909,
                "screen_w": 160,
                "screen_h": 90,
                "monitors": [], "cursor": [], "clicks": [], "scrolls": [], "keys": []
            }
        }))
        .unwrap(),
    )
    .unwrap();

    let stopped = dispatch(
        &state,
        "screen_record.stop",
        json!({"capture_id": "cap-sparse-timebase", "autoedit": true}),
        test_actor(),
    )
    .await;
    assert!(stopped.ok, "stop failed: {:?}", stopped.error);
    let plan = PathBuf::from(stopped.result.unwrap()["plan"].as_str().unwrap());
    let plan_json: Value = serde_json::from_slice(&std::fs::read(&plan).unwrap()).unwrap();
    assert_eq!(plan_json["fps"], 30.0);
    assert_eq!(plan_json["duration_ms"], 7_909);

    let output = project_dir.join("exports/sparse-timebase.mp4");
    std::fs::create_dir_all(output.parent().unwrap()).unwrap();
    let queued = dispatch(
        &state,
        "screen_record.export",
        json!({"source": source, "plan": plan, "path": output}),
        test_actor(),
    )
    .await;
    assert!(queued.ok, "export queue failed: {:?}", queued.error);
    let result = wait_export(&state, queued.result.unwrap()["job_id"].as_str().unwrap()).await;
    let rendered_frames = result["frames"].as_u64().unwrap();
    assert!(
        (237..=239).contains(&rendered_frames),
        "sparse capture emitted {rendered_frames} planned 30fps frames"
    );

    let facts = ffprobe(&output);
    let streams = facts["streams"].as_array().unwrap();
    let video = streams
        .iter()
        .find(|stream| stream["codec_type"] == "video")
        .unwrap();
    assert_eq!(video["avg_frame_rate"], "30/1");
    assert_eq!(
        video["nb_read_frames"]
            .as_str()
            .unwrap()
            .parse::<u64>()
            .unwrap(),
        rendered_frames
    );
    let duration = facts["format"]["duration"]
        .as_str()
        .unwrap()
        .parse::<f64>()
        .unwrap();
    assert!(
        (duration - 7.933).abs() < 0.08,
        "sparse capture export drifted from its 7.909s plan: {duration}"
    );
    let audio = streams
        .iter()
        .find(|stream| stream["codec_type"] == "audio")
        .unwrap();
    let audio_duration = audio["duration"].as_str().unwrap().parse::<f64>().unwrap();
    assert!(
        (audio_duration - duration).abs() < 0.12,
        "audio no longer follows planned video: video={duration}, audio={audio_duration}"
    );
}
