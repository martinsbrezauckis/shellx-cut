//! Real-FFmpeg regression for recorder stop → autoedit → export timebase/audio.

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

fn synth_video(out: &Path, duration_s: u32) {
    run_ffmpeg(
        &[
            "-f",
            "lavfi",
            "-i",
            &format!("testsrc2=size=160x90:rate=25:duration={duration_s}"),
            "-an",
            "-c:v",
            "libx264",
            "-pix_fmt",
            "yuv420p",
        ],
        out,
    );
}

fn synth_tone(out: &Path, frequency: u32, duration_s: u32) {
    run_ffmpeg(
        &[
            "-f",
            "lavfi",
            "-i",
            &format!("sine=frequency={frequency}:sample_rate=48000:duration={duration_s}"),
            "-ac",
            "2",
            "-c:a",
            "pcm_s16le",
        ],
        out,
    );
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

async fn export_capture(state: &AppState, source: &Path, plan: &Path, output: &Path) -> Value {
    let queued = dispatch(
        state,
        "screen_record.export",
        json!({
            "source": source.display().to_string(),
            "plan": plan.display().to_string(),
            "path": output.display().to_string(),
        }),
        test_actor(),
    )
    .await;
    assert!(queued.ok, "export queue failed: {:?}", queued.error);
    let job_id = queued.result.unwrap()["job_id"]
        .as_str()
        .expect("queued export job id")
        .to_string();
    wait_export(state, &job_id).await
}

fn ffprobe(path: &Path) -> Value {
    let output = std::process::Command::new(cut_media::toolpath::ffprobe())
        .args([
            "-v",
            "error",
            "-count_frames",
            "-show_entries",
            "stream=codec_type,avg_frame_rate,nb_read_frames,sample_rate,channels:format=duration",
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

fn audio_rms(path: &Path, start_s: f64) -> f64 {
    let output = std::process::Command::new(cut_media::toolpath::ffmpeg())
        .args([
            "-v",
            "error",
            "-ss",
            &format!("{start_s:.3}"),
            "-t",
            "0.100",
            "-i",
        ])
        .arg(path)
        .args(["-map", "0:a:0", "-ac", "1", "-f", "s16le", "-"])
        .output()
        .expect("decode fixture audio");
    assert!(
        output.status.success(),
        "audio decode failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let samples: Vec<f64> = output
        .stdout
        .chunks_exact(2)
        .map(|sample| i16::from_le_bytes([sample[0], sample[1]]) as f64)
        .collect();
    assert!(!samples.is_empty(), "expected decoded audio samples");
    (samples.iter().map(|sample| sample * sample).sum::<f64>() / samples.len() as f64).sqrt()
}

fn recording_project(source: &Path, mic: Option<&Path>, duration_ms: u64) -> Value {
    json!({
        "schema": "shellx-record/1",
        "settings": {"width": 160, "height": 90, "fps": 25.0, "audio_rate": 48000},
        "source_video": source.display().to_string(),
        "audio": mic.map(|path| path.display().to_string()),
        "events": {
            "duration_ms": duration_ms,
            "screen_w": 160,
            "screen_h": 90,
            "monitors": [], "cursor": [], "clicks": [], "scrolls": [], "keys": []
        }
    })
}

#[tokio::test]
async fn stop_autoedit_export_preserves_capture_timebase_and_aligned_capture_audio() {
    let temp = tempfile::tempdir().unwrap();
    let project_dir = temp.path().join("export_timebase.cutproj");
    let state = AppState::new();
    let created = dispatch(
        &state,
        "project.create",
        json!({"name": "export_timebase", "dir": project_dir}),
        test_actor(),
    )
    .await;
    assert!(created.ok, "project create failed: {:?}", created.error);

    let capture = crate::screen_record::screen_record_cache_dir(&project_dir)
        .unwrap()
        .join("cap-timebase");
    std::fs::create_dir_all(&capture).unwrap();
    let source = capture.join("source.mp4");
    let mic = capture.join("mic.wav");
    let system = capture.join("system.wav");
    synth_video(&source, 60);
    synth_tone(&mic, 440, 60);
    synth_tone(&system, 880, 60);
    std::fs::write(
        capture.join("system-audio.json"),
        br#"{"schema":"shellx-cut/system-audio-timing/1","first_packet_offset_ms":200}"#,
    )
    .unwrap();
    std::fs::write(
        capture.join("project.json"),
        serde_json::to_vec(&recording_project(&source, Some(&mic), 60_000)).unwrap(),
    )
    .unwrap();

    let stopped = dispatch(
        &state,
        "screen_record.stop",
        json!({"capture_id": "cap-timebase", "autoedit": true}),
        test_actor(),
    )
    .await;
    assert!(stopped.ok, "stop failed: {:?}", stopped.error);
    let plan = PathBuf::from(
        stopped.result.unwrap()["plan"]
            .as_str()
            .expect("stop autoedit plan"),
    );
    assert!(
        plan.is_file(),
        "stop returned a missing plan: {}",
        plan.display()
    );
    let plan_json: Value = serde_json::from_slice(&std::fs::read(&plan).unwrap()).unwrap();
    assert_eq!(plan_json["fps"], 25.0);
    assert_eq!(plan_json["duration_ms"], 60_000);

    let exports = project_dir.join("exports");
    std::fs::create_dir_all(&exports).unwrap();
    let output = exports.join("timebase-audio.mp4");
    let result = export_capture(&state, &source, &plan, &output).await;
    assert_eq!(result["frames"], 1_500);
    assert!(
        output.is_file(),
        "export output missing: {}",
        output.display()
    );
    let facts = ffprobe(&output);
    let streams = facts["streams"].as_array().expect("ffprobe streams");
    let video = streams
        .iter()
        .find(|stream| stream["codec_type"] == "video")
        .expect("output video stream");
    assert_eq!(video["avg_frame_rate"], "25/1");
    assert_eq!(video["nb_read_frames"], "1500");
    let duration = facts["format"]["duration"]
        .as_str()
        .expect("output duration")
        .parse::<f64>()
        .unwrap();
    assert!((duration - 60.0).abs() < 0.12, "wrong duration: {duration}");
    let audio = streams
        .iter()
        .find(|stream| stream["codec_type"] == "audio")
        .expect("output audio stream");
    assert!(
        audio["sample_rate"]
            .as_str()
            .and_then(|rate| rate.parse::<u32>().ok())
            .is_some_and(|rate| rate >= 48_000),
        "output audio has no usable sample rate: {audio}"
    );
    assert_eq!(audio["channels"], 2);
    let mic_only_rms = audio_rms(&output, 0.050);
    let mixed_rms = audio_rms(&output, 0.500);
    assert!(mic_only_rms > 100.0, "mic was not audible: {mic_only_rms}");
    assert!(
        mixed_rms > mic_only_rms * 1.20,
        "system tone was not mixed after its 200ms alignment delay: before={mic_only_rms}, after={mixed_rms}"
    );
}

#[tokio::test]
async fn export_system_only_audio_materializes_its_packet_offset() {
    let temp = tempfile::tempdir().unwrap();
    let project_dir = temp.path().join("export_system_only.cutproj");
    let state = AppState::new();
    let created = dispatch(
        &state,
        "project.create",
        json!({"name": "export_system_only", "dir": project_dir}),
        test_actor(),
    )
    .await;
    assert!(created.ok, "project create failed: {:?}", created.error);

    let capture = crate::screen_record::screen_record_cache_dir(&project_dir)
        .unwrap()
        .join("cap-system-only");
    std::fs::create_dir_all(&capture).unwrap();
    let source = capture.join("source.mp4");
    let system = capture.join("system.wav");
    synth_video(&source, 1);
    synth_tone(&system, 880, 1);
    std::fs::write(
        capture.join("system-audio.json"),
        br#"{"schema":"shellx-cut/system-audio-timing/1","first_packet_offset_ms":200}"#,
    )
    .unwrap();
    let plan = capture.join("system-only.plan.json");
    std::fs::write(
        &plan,
        serde_json::to_vec(&record_core::EditPlan::empty(160, 90, 1_000, 25.0)).unwrap(),
    )
    .unwrap();
    let exports = project_dir.join("exports");
    std::fs::create_dir_all(&exports).unwrap();
    let output = exports.join("system-only.mp4");
    export_capture(&state, &source, &plan, &output).await;

    let silent_rms = audio_rms(&output, 0.020);
    let audible_rms = audio_rms(&output, 0.400);
    assert!(
        audible_rms > 100.0,
        "system audio was not audible: {audible_rms}"
    );
    assert!(
        audible_rms > silent_rms * 10.0,
        "system-only export ignored its 200ms packet offset: before={silent_rms}, after={audible_rms}"
    );
}
