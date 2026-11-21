//! Every clip contributes its own pictures, including one-frame clips and
//! seams between frame boundaries. The source has a burnt-in binary counter;
//! checking duration alone would miss a video track shortened under its sound.
//!
//! Run these exports with `cargo test -p cut-media --test frame_seams`.

cut_core::isolate_test_process!();

use cut_core::{edl_from_project, timeline_frame_count, Project};
use cut_media::{render_final, PathFence, RenderOptions, RenderPreset};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

fn private_dir() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    dir
}

/// Four sync cells followed by twelve counter bits, with the complementary
/// row below. The counter remains readable after the normal export encoder.
fn counter_filter(counter: &str) -> String {
    format!(
        "nullsrc=s=16x2,format=gray,geq=lum='255*if(eq(Y,0),\
         if(lt(X,4),lt(X,2),mod(floor(({counter})/pow(2,15-X)),2)),\
         1-if(lt(X,4),lt(X,2),mod(floor(({counter})/pow(2,15-X)),2)))',\
         scale=320:180:flags=neighbor,format=yuv420p"
    )
}

fn source(dir: &Path, fps: u32, frames: u32, audio_ms: u32) -> PathBuf {
    let out = dir.join("counter.mp4");
    let filter =
        counter_filter("N").replacen("nullsrc=s=16x2", &format!("nullsrc=s=16x2:r={fps}"), 1);
    let args: Vec<String> = ["-f", "lavfi", "-i"]
        .into_iter()
        .map(String::from)
        .chain([filter])
        .chain(["-f", "lavfi", "-i"].into_iter().map(String::from))
        .chain([format!(
            "sine=frequency=880:sample_rate=48000:duration={}",
            audio_ms as f64 / 1000.0
        )])
        .chain(
            ["-map", "0:v", "-map", "1:a", "-frames:v"]
                .into_iter()
                .map(String::from),
        )
        .chain([frames.to_string()])
        .chain(
            [
                "-c:v",
                "libx264",
                "-preset",
                "ultrafast",
                "-crf",
                "0",
                "-c:a",
                "aac",
            ]
            .into_iter()
            .map(String::from),
        )
        .chain([out.display().to_string()])
        .collect();
    cut_media::ffmpeg::run_ffmpeg(&args).expect("generate numbered source with sound");
    out
}

fn still(dir: &Path, counter: u32) -> PathBuf {
    let out = dir.join(format!("still-{counter}.png"));
    let args = vec![
        "-f".into(),
        "lavfi".into(),
        "-i".into(),
        counter_filter(&counter.to_string()),
        "-frames:v".into(),
        "1".into(),
        out.display().to_string(),
    ];
    cut_media::ffmpeg::run_ffmpeg(&args).expect("generate numbered still");
    out
}

fn asset(path: &Path) -> Value {
    json!({"path": path.display().to_string(), "hash": format!("sha256:{}", path.display()),
        "probe": serde_json::to_value(cut_media::probe(path).expect("probe source")).unwrap()})
}

fn project(fps: f64, assets: Value, clips: Vec<Value>, audio_ms: u64) -> Project {
    serde_json::from_value(json!({
        "schema": "shellx-cut/1", "name": "frame-seams",
        "settings": {"width": 320, "height": 180, "fps": fps, "audio_rate": 48000},
        "assets": assets,
        "tracks": [
            {"id": "v1", "kind": "video", "clips": clips},
            {"id": "a1", "kind": "audio", "clips": [
                {"id": "sound", "asset": "source", "src_in_ms": 0, "src_out_ms": audio_ms}
            ]}
        ]
    }))
    .expect("test project")
}

fn read_counters(path: &Path) -> Vec<Option<u32>> {
    let out = std::process::Command::new(cut_media::ffmpeg::ffmpeg_bin())
        .args(["-v", "error", "-xerror", "-nostdin", "-i"])
        .arg(path)
        .args([
            "-vf",
            "scale=16:2:flags=area,format=gray",
            "-fps_mode",
            "passthrough",
            "-f",
            "rawvideo",
            "-",
        ])
        .output()
        .expect("decode pictures");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(out.stdout.len() % 32, 0, "complete decoded pictures");
    out.stdout
        .as_chunks::<32>()
        .0
        .iter()
        .map(|frame| {
            let mut bits = Vec::new();
            for x in 0..16 {
                let bit = |v: u8| {
                    if v > 150 {
                        Some(1u32)
                    } else if v < 100 {
                        Some(0)
                    } else {
                        None
                    }
                };
                let a = bit(frame[x])?;
                if a == bit(frame[16 + x])? {
                    return None;
                }
                bits.push(a);
            }
            (bits[..4] == [1, 1, 0, 0]).then(|| bits[4..].iter().fold(0, |n, b| n * 2 + b))
        })
        .collect()
}

fn check_export(p: &Project, dir: &Path, expected: &[Option<u32>]) {
    let edl = edl_from_project(p);
    assert_eq!(
        timeline_frame_count(edl.duration_ms, p.settings.fps) as usize,
        expected.len()
    );
    let out = dir.join("export.mp4");
    render_final(
        p,
        &edl,
        &PathFence::new(dir).unwrap(),
        &out,
        &RenderPreset::default(),
        RenderOptions::default(),
        None,
    )
    .expect("export timeline");
    let observed = read_counters(&out);
    let mismatches: Vec<_> = expected
        .iter()
        .zip(&observed)
        .enumerate()
        .filter(|(_, (want, got))| want != got)
        .take(12)
        .collect();
    assert!(
        observed == expected,
        "picture sequence: got {} frames, expected {}; first mismatches: {mismatches:?}",
        observed.len(),
        expected.len()
    );
    let probe = cut_media::probe(&out).expect("probe export");
    let extents = cut_media::probe::stream_extents_from_raw(&probe.raw);
    for kind in ["video", "audio"] {
        let ms = extents
            .iter()
            .find(|s| s.kind == kind)
            .and_then(|s| s.duration_ms)
            .expect("stream duration");
        assert!(
            ms.abs_diff(edl.duration_ms) <= (1000.0 / p.settings.fps).ceil() as u64,
            "{kind}: {ms} ms, timeline {} ms",
            edl.duration_ms
        );
    }
}

fn one_frame_clips(swapped: bool) {
    let dir = private_dir();
    let src = source(dir.path(), 25, 180, 7200);
    assert_eq!(
        read_counters(&src),
        (0..180).map(Some).collect::<Vec<_>>(),
        "source counter"
    );
    let mut order: Vec<u32> = (0..180).collect();
    if swapped {
        order.swap(5, 6);
        order.swap(22, 23);
    }
    let clips = order.iter().enumerate().map(|(k, n)| json!({
        "id": format!("c{k}"), "asset": "source", "src_in_ms": n * 40, "src_out_ms": (n + 1) * 40
    })).collect();
    let p = project(25.0, json!({"source": asset(&src)}), clips, 7200);
    check_export(
        &p,
        dir.path(),
        &order.into_iter().map(Some).collect::<Vec<_>>(),
    );
}

#[test]
fn every_one_frame_clip_keeps_its_picture() {
    one_frame_clips(false);
}

#[test]
fn reordered_one_frame_clips_keep_their_own_pictures() {
    one_frame_clips(true);
}

#[test]
fn fractional_frame_still_seams_keep_every_picture() {
    let dir = private_dir();
    let src = source(dir.path(), 30, 441, 14708);
    let first = still(dir.path(), 1000);
    let second = still(dir.path(), 1001);
    let expected_source: Vec<_> = (0..441).map(Some).collect();
    assert_eq!(read_counters(&src), expected_source, "source counter");
    let p = project(
        30.0,
        json!({"source": asset(&src), "first": asset(&first), "second": asset(&second)}),
        vec![
            json!({"id": "first", "asset": "first", "src_in_ms": 0, "src_out_ms": 34}),
            json!({"id": "second", "asset": "second", "src_in_ms": 0, "src_out_ms": 34}),
            json!({"id": "video", "asset": "source", "src_in_ms": 0, "src_out_ms": 14708}),
        ],
        14776,
    );
    let expected: Vec<_> = [Some(1000), Some(1001)]
        .into_iter()
        .chain(expected_source)
        .collect();
    check_export(&p, dir.path(), &expected);
}
