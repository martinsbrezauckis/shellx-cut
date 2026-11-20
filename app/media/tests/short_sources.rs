//! A clip whose source has fewer frames than its range asks for still fills
//! exactly its slot (seen in an installed Windows run: a recovered
//! recording, container 6.900 s, video 206 frames at 30 fps, exported 2 frames
//! short). Two sources of that kind: one whose container lasts longer than its
//! video (its sound runs on), and a variable-frame-rate one (every tenth frame
//! missing, as a screen recorder writes when it cannot keep up). Whole, in
//! windows and as the draft preview, the picture has exactly the timeline's
//! frames, the three agree frame by frame, and past the short source's last
//! frame that frame is held.

cut_core::isolate_test_process!();

use cut_core::{edl_from_project, timeline_frame_count, Project};
use cut_media::{render_final, PathFence, RenderOptions, RenderPreset};
use serde_json::json;
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

/// A 30 fps 320x180 clip whose frames carry a barcode (clip `id`, frame
/// index); `video` filters it (frame count, dropped frames), `inputs` adds
/// inputs (a sound that outlasts the picture) and `extra` adds
/// output args (the mapping of that sound, the variable-rate muxing).
fn gen(dir: &Path, name: &str, id: u32, video: &str, inputs: &[&str], extra: &[&str]) -> PathBuf {
    let out = dir.join(name);
    let cell = format!(
        "if(eq(X,0),1,if(lt(X,4),mod(floor({id}/pow(2,3-X)),2),mod(floor(N/pow(2,15-X)),2)))"
    );
    let lum = format!("255*if(eq(Y,0),{cell},1-{cell})");
    let mut args: Vec<String> = [
        "-f",
        "lavfi",
        "-i",
        "testsrc2=s=320x180:r=30:d=10",
        "-f",
        "lavfi",
        "-i",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    args.push(format!(
        "color=c=black:s=16x2:r=30:d=10,format=gray,geq=lum='{lum}'"
    ));
    args.extend(inputs.iter().map(|s| s.to_string()));
    args.push("-filter_complex".into());
    args.push(format!(
        "[1:v]scale=320:45:flags=neighbor,format=yuv420p[bc];[0:v][bc]overlay=0:0{video}[v]"
    ));
    args.extend(["-map", "[v]"].iter().map(|s| s.to_string()));
    args.extend(extra.iter().map(|s| s.to_string()));
    args.extend(
        [
            "-c:v",
            "libx264",
            "-preset",
            "ultrafast",
            "-crf",
            "16",
            "-pix_fmt",
            "yuv420p",
        ]
        .iter()
        .map(|s| s.to_string()),
    );
    args.push(out.display().to_string());
    cut_media::ffmpeg::run_ffmpeg(&args).expect("generate a source");
    out
}

/// `(clip id, frame index)` of every frame; `None` when unreadable.
fn read_barcodes(media: &Path) -> Vec<Option<(u32, u32)>> {
    let (w, h) = (160usize, 90usize);
    let out = std::process::Command::new(cut_media::ffmpeg::ffmpeg_bin())
        .args(["-v", "error", "-nostdin", "-i"])
        .arg(media)
        .args([
            "-vf",
            "scale=160:90:flags=area,format=gray",
            "-f",
            "rawvideo",
            "-",
        ])
        .output()
        .expect("read back the render");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    out.stdout
        .chunks_exact(w * h)
        .map(|frame| {
            let (ya, yb) = (h / 16, (3 * h) / 16);
            let mut bits = Vec::with_capacity(16);
            for i in 0..16 {
                let x = ((i as f64 + 0.5) * w as f64 / 16.0) as usize;
                let bit = |v: u8| (v > 150).then_some(1).or((v < 100).then_some(0));
                let (a, b) = (bit(frame[ya * w + x])?, bit(frame[yb * w + x])?);
                if a == b {
                    return None;
                }
                bits.push(a);
            }
            (bits[0] == 1).then(|| {
                (
                    bits[1..4].iter().fold(0, |n, b| n * 2 + b),
                    bits[4..].iter().fold(0, |n, b| n * 2 + b),
                )
            })
        })
        .collect()
}

fn asset(path: &Path) -> serde_json::Value {
    json!({"path": path.display().to_string(), "hash": format!("sha256:{}", path.display()),
           "probe": serde_json::to_value(cut_media::probe(path).expect("probe")).unwrap()})
}

fn render(p: &Project, dir: &Path, name: &str, env: &[(&str, &str)]) -> Vec<Option<(u32, u32)>> {
    for (k, v) in env {
        std::env::set_var(k, v);
    }
    let out = dir.join(name);
    let r = render_final(
        p,
        &edl_from_project(p),
        &PathFence::new(dir).unwrap(),
        &out,
        &RenderPreset::default(),
        RenderOptions::default(),
        None,
    );
    for (k, _) in env {
        std::env::remove_var(k);
    }
    r.expect("render");
    read_barcodes(&out)
}

#[test]
fn a_source_shorter_than_its_range_fills_its_slot_whole_windowed_and_in_the_draft() {
    let dir = private_dir();
    // 200 frames (6.667 s) of picture; the sound runs to 7.5 s, so the file
    // (and its probed duration) is 7.5 s long.
    let short = gen(
        dir.path(),
        "short.mp4",
        1,
        ",trim=end_frame=200",
        &["-f", "lavfi", "-i", "sine=f=440:d=7.5"],
        &["-map", "2:a", "-c:a", "aac"],
    );
    // Every tenth frame missing, timestamps kept: a variable-rate source.
    let vfr = gen(
        dir.path(),
        "vfr.mp4",
        2,
        ",select='not(eq(mod(n\\,10)\\,3))'",
        &[],
        &["-fps_mode", "vfr"],
    );
    let p: Project = serde_json::from_value(json!({
        "schema": "shellx-cut/1", "name": "short-sources",
        "settings": {"width": 320, "height": 180, "fps": 60.0, "audio_rate": 48000},
        "assets": {"a1": asset(&short), "a2": asset(&vfr)},
        "tracks": [{"id": "v1", "kind": "video", "clips": [
            {"id": "c1", "asset": "a1", "src_in_ms": 0, "src_out_ms": 7000},
            {"id": "c2", "asset": "a2", "src_in_ms": 1000, "src_out_ms": 4000}]}],
    }))
    .expect("test project");
    let edl = edl_from_project(&p);
    let frames = timeline_frame_count(edl.duration_ms, 60.0) as usize;

    let whole = render(
        &p,
        dir.path(),
        "whole.mp4",
        &[("SHELLX_CUT_RENDER_SEGMENT_SEC", "100000")],
    );
    assert_eq!(whole.len(), frames, "whole: the timeline's frames");
    assert!(whole.iter().all(Option::is_some), "every frame readable");
    // c1 occupies frames 0..420 at 60 fps; its source ends at frame 199
    // (output frame 399): the rest of its slot holds that frame.
    assert_eq!(whole[399], Some((1, 199)));
    assert_eq!(
        whole[419],
        Some((1, 199)),
        "the short clip's slot ends on its last frame"
    );
    assert_eq!(
        whole[420].map(|c| c.0),
        Some(2),
        "the next clip starts on its own frame"
    );

    let windowed = render(
        &p,
        dir.path(),
        "windowed.mp4",
        &[
            ("SHELLX_CUT_RENDER_SEGMENT_SEC", "0"),
            ("SHELLX_CUT_RENDER_WINDOW_SEC", "2"),
        ],
    );
    assert_eq!(windowed, whole, "windowed equals whole frame by frame");

    let cache = dir.path().join("preview-cache");
    let draft = cut_media::render_preview_incremental(
        &p,
        &edl,
        dir.path(),
        &cache,
        &RenderPreset::named("draft").unwrap(),
    )
    .expect("draft preview");
    assert_eq!(
        read_barcodes(&draft.path),
        whole,
        "the draft equals the export"
    );
}
