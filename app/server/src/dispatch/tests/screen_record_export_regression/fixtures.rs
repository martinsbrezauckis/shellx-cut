//! Real media creation, export completion, and decoding for recorder regressions.

use super::*;
use crate::jobs::JobState;
use std::time::Duration;

pub(super) fn run_ffmpeg(args: &[&str], out: &Path) {
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

pub(super) fn synth_video(out: &Path, duration_s: u32) {
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

pub(super) fn synth_tone(out: &Path, frequency: u32, duration_s: u32) {
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

pub(super) async fn wait_export(state: &AppState, job_id: &str) -> Value {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(120);
    loop {
        let record = state.jobs.get(job_id).expect("export job record");
        match record.state {
            JobState::Done => return record.result.expect("completed export result"),
            JobState::Failed => panic!("export job failed: {:?}", record.error),
            _ if tokio::time::Instant::now() < deadline => {
                tokio::time::sleep(Duration::from_millis(10)).await
            }
            _ => panic!("export job {job_id} did not complete within 120 seconds"),
        }
    }
}

pub(super) async fn export_capture(
    state: &AppState,
    source: &Path,
    plan: &Path,
    output: &Path,
) -> Value {
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

pub(super) fn ffprobe(path: &Path) -> Value {
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

pub(super) fn audio_rms(path: &Path, start_s: f64) -> f64 {
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
        .as_chunks::<2>()
        .0
        .iter()
        .map(|sample| i16::from_le_bytes([sample[0], sample[1]]) as f64)
        .collect();
    assert!(!samples.is_empty(), "expected decoded audio samples");
    (samples.iter().map(|sample| sample * sample).sum::<f64>() / samples.len() as f64).sqrt()
}

pub(super) fn recording_project(source: &Path, mic: Option<&Path>, duration_ms: u64) -> Value {
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
