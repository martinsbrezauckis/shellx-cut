//! Row 194: the rendered video carries exactly the timeline's frames.
//!
//! The source frames carry a barcode (sync cell, 3-bit clip id, 12-bit frame
//! index). Ten 513 ms cuts from a 30 fps source (every cut between source
//! frames) must render `timeline_frame_count` frames and every output frame
//! must show the source frame the edit defines, in a 60 fps project and in a
//! 30 fps project, whole and in windows that start inside clips (which must be
//! frame-identical to the whole render). Before the fix each 513 ms clip
//! rendered 32 frames at 60 fps and 16 at 30 fps instead of its slot (30.8 /
//! 15.4), so the file was longer than the timeline and every cut moved the
//! later pictures late; and a window starting inside a rate-converted clip
//! showed other source frames than the whole render.
//!
//! The expected picture: a cut at `t` ms takes effect at output frame
//! `round(t * fps / 1000)`; inside a clip, output frame `j` (from the clip's
//! first frame) shows the clip's own frame `j`: source frame `first + j` at the
//! same rate, `first + floor(j / 2)` at 30 -> 60 fps, `first` being the first
//! source frame at or after the in point.

cut_core::isolate_test_process!();

use cut_core::{edl_from_project, timeline_frame_count, Project};
use cut_media::{render_final, PathFence, RenderOptions, RenderPreset};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

static ENV: Mutex<()> = Mutex::new(());

fn private_dir() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    dir
}

/// A barcoded 30 fps clip (clip id 1) of `secs` seconds at 320x180.
fn gen_barcoded(dir: &Path, secs: u32) -> PathBuf {
    let out = dir.join("src30.mp4");
    let cell = "if(eq(X,0),1,if(lt(X,4),mod(floor(1/pow(2,3-X)),2),mod(floor(N/pow(2,15-X)),2)))";
    let lum = format!("255*if(eq(Y,0),{cell},1-{cell})");
    let args: Vec<String> = vec![
        "-f".into(),
        "lavfi".into(),
        "-i".into(),
        format!("testsrc2=s=320x180:r=30:d={secs}"),
        "-f".into(),
        "lavfi".into(),
        "-i".into(),
        format!("color=c=black:s=16x2:r=30:d={secs},format=gray,geq=lum='{lum}'"),
        "-filter_complex".into(),
        "[1:v]scale=320:45:flags=neighbor,format=yuv420p[bc];[0:v][bc]overlay=0:0[v]".into(),
        "-map".into(),
        "[v]".into(),
        "-c:v".into(),
        "libx264".into(),
        "-preset".into(),
        "ultrafast".into(),
        "-crf".into(),
        "16".into(),
        "-pix_fmt".into(),
        "yuv420p".into(),
        out.display().to_string(),
    ];
    cut_media::ffmpeg::run_ffmpeg(&args).expect("generate the barcoded source");
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

const CUTS: u64 = 10;
const CLIP_MS: u64 = 513;

fn project(src: &Path, fps: f64) -> Project {
    let clips: Vec<Value> = (0..CUTS)
        .map(|k| {
            json!({"id": format!("c{k}"), "asset": "a1", "src_in_ms": k * 2500,
                   "src_out_ms": k * 2500 + CLIP_MS})
        })
        .collect();
    serde_json::from_value(json!({
        "schema": "shellx-cut/1", "name": "exact-length",
        "settings": {"width": 320, "height": 180, "fps": fps, "audio_rate": 48000},
        "assets": {"a1": {"path": src.display().to_string(), "hash": "sha256:exact-length-src",
            "probe": {"kind": "video", "width": 320, "height": 180, "fps": 30.0, "has_audio": false,
                      "raw": {"streams": [{"codec_type": "video", "pix_fmt": "yuv420p"}]}}}},
        "tracks": [{"id": "v1", "kind": "video", "clips": clips}],
    }))
    .expect("test project")
}

/// The source frame the edit defines at output frame `k` (30 or 60 fps).
fn expected(k: u64, fps: f64) -> u32 {
    let at = |t: u64| (t as f64 * fps / 1000.0).round() as u64;
    let clip = (0..CUTS)
        .rfind(|c| at(c * CLIP_MS) <= k)
        .expect("frame inside the edit");
    let j = k - at(clip * CLIP_MS);
    let first = (clip * 2500 * 30).div_ceil(1000);
    (first + j * 30 / fps as u64) as u32
}

struct EnvGuard(Vec<&'static str>);
impl EnvGuard {
    fn set(pairs: &[(&'static str, &str)]) -> Self {
        for (k, v) in pairs {
            std::env::set_var(k, v);
        }
        Self(pairs.iter().map(|(k, _)| *k).collect())
    }
}
impl Drop for EnvGuard {
    fn drop(&mut self) {
        for k in &self.0 {
            std::env::remove_var(k);
        }
    }
}

fn render(p: &Project, dir: &Path, name: &str) -> PathBuf {
    let edl = edl_from_project(p);
    let out = dir.join(name);
    render_final(
        p,
        &edl,
        &PathFence::new(dir).unwrap(),
        &out,
        &RenderPreset::default(),
        RenderOptions::default(),
        None,
    )
    .expect("render");
    out
}

fn check_exact(codes: &[Option<(u32, u32)>], fps: f64, frames: u64, what: &str) {
    assert_eq!(codes.len() as u64, frames, "{what}: frame count");
    let wrong: Vec<String> = codes
        .iter()
        .enumerate()
        .filter(|(k, code)| **code != Some((1, expected(*k as u64, fps))))
        .map(|(k, code)| format!("frame {k}: want {}, got {code:?}", expected(k as u64, fps)))
        .collect();
    assert!(
        wrong.is_empty(),
        "{what}: {:#?}",
        &wrong[..wrong.len().min(12)]
    );
}

#[test]
fn cuts_between_source_frames_render_exactly_the_timeline() {
    let _env = ENV.lock().unwrap_or_else(|e| e.into_inner());
    let dir = private_dir();
    let src = gen_barcoded(dir.path(), 30);
    // 60 fps over a 30 fps source: the verifiers lane's case (row 194).
    let p60 = project(&src, 60.0);
    let edl = edl_from_project(&p60);
    assert_eq!(edl.duration_ms, CUTS * CLIP_MS);
    let whole60 = {
        let _single = EnvGuard::set(&[("SHELLX_CUT_RENDER_SEGMENT_SEC", "100000")]);
        read_barcodes(&render(&p60, dir.path(), "whole60.mp4"))
    };
    check_exact(
        &whole60,
        60.0,
        timeline_frame_count(edl.duration_ms, 60.0),
        "60 fps whole",
    );
    // ... and in 2 s windows, which start inside rate-converted clips.
    let windowed60 = {
        let _windows = EnvGuard::set(&[
            ("SHELLX_CUT_RENDER_SEGMENT_SEC", "0"),
            ("SHELLX_CUT_RENDER_WINDOW_SEC", "2"),
        ]);
        read_barcodes(&render(&p60, dir.path(), "windowed60.mp4"))
    };
    assert_eq!(
        whole60, windowed60,
        "60 fps windowed must be frame-identical to whole"
    );
    // The same cuts at the same rate, whole and in 2 s windows (seams inside clips).
    let p30 = project(&src, 30.0);
    let frames30 = timeline_frame_count(edl.duration_ms, 30.0);
    let whole30 = {
        let _single = EnvGuard::set(&[("SHELLX_CUT_RENDER_SEGMENT_SEC", "100000")]);
        read_barcodes(&render(&p30, dir.path(), "whole30.mp4"))
    };
    let windowed30 = {
        let _windows = EnvGuard::set(&[
            ("SHELLX_CUT_RENDER_SEGMENT_SEC", "0"),
            ("SHELLX_CUT_RENDER_WINDOW_SEC", "2"),
        ]);
        read_barcodes(&render(&p30, dir.path(), "windowed30.mp4"))
    };
    check_exact(&whole30, 30.0, frames30, "30 fps whole");
    check_exact(&windowed30, 30.0, frames30, "30 fps windowed");
    assert_eq!(
        whole30, windowed30,
        "windowed must be frame-identical to whole"
    );
}

/// The CUDA fast-track (`SHELLX_CUT_RENDER_GPU=1` on an NVIDIA machine) shows
/// the same source frame at every output frame as the software graph. Before
/// the fix its clips kept their own frame counts (no frame lock). On a machine
/// without a working CUDA chain the opt-in renders in software, which the
/// first test covers; this one says so and checks nothing more.
#[test]
fn cuda_fast_track_renders_the_same_frames_as_software() {
    let _env = ENV.lock().unwrap_or_else(|e| e.into_inner());
    if !cut_media::hwencode::gpu_filters_available() {
        eprintln!("NOT RUN: this machine has no working CUDA filter chain");
        return;
    }
    let dir = private_dir();
    let src = gen_barcoded(dir.path(), 30);
    for fps in [60.0, 30.0] {
        let p = project(&src, fps);
        let edl = edl_from_project(&p);
        let frames = timeline_frame_count(edl.duration_ms, fps);
        let software = {
            let _single = EnvGuard::set(&[("SHELLX_CUT_RENDER_SEGMENT_SEC", "100000")]);
            read_barcodes(&render(&p, dir.path(), &format!("sw{fps}.mp4")))
        };
        let _gpu = EnvGuard::set(&[("SHELLX_CUT_RENDER_GPU", "1")]);
        let out = dir.path().join(format!("gpu{fps}.mp4"));
        let done = render_final(
            &p,
            &edl,
            &PathFence::new(dir.path()).unwrap(),
            &out,
            &RenderPreset::default(),
            RenderOptions::default(),
            None,
        )
        .expect("GPU render");
        assert_eq!(done.pipeline.as_deref(), Some("gpu"), "{fps} fps: GPU path");
        let gpu = read_barcodes(&out);
        check_exact(&gpu, fps, frames, &format!("{fps} fps GPU"));
        assert_eq!(
            gpu, software,
            "{fps} fps: GPU frames equal the software frames"
        );
    }
}

/// The draft preview (`render.preview draft:true`) shows the export's frames:
/// the same source frame at every index and the same length. Before the fix
/// each clip came from its own `-ss`/`-t` encode of the proxy (first frame
/// doubled, last dropped, no speed or reverse) and the draft drifted from the
/// export. After an edit only the pieces whose content changed are remade:
/// moving a clip's source keeps every other piece, and lengthening a clip by
/// whole frames moves the later pieces by whole frames, which are reused.
#[test]
fn the_draft_preview_shows_the_export_frames_and_remakes_only_what_changed() {
    let _env = ENV.lock().unwrap_or_else(|e| e.into_inner());
    let dir = private_dir();
    let src = gen_barcoded(dir.path(), 30);
    let draft = RenderPreset::named("draft").unwrap();
    for fps in [60.0, 30.0] {
        let mut p = project(&src, fps);
        let edl = edl_from_project(&p);
        let export = {
            let _single = EnvGuard::set(&[("SHELLX_CUT_RENDER_SEGMENT_SEC", "100000")]);
            read_barcodes(&render(&p, dir.path(), &format!("export{fps}.mp4")))
        };
        let cache = dir.path().join(format!("preview-cache-{fps}"));
        let first = cut_media::render_preview_incremental(&p, &edl, dir.path(), &cache, &draft)
            .expect("draft preview");
        assert_eq!(
            first.segments_rendered, CUTS as usize,
            "{fps}: one piece per clip"
        );
        let shown = read_barcodes(&first.path);
        assert_eq!(
            shown.len() as u64,
            timeline_frame_count(edl.duration_ms, fps),
            "{fps} fps draft: frame count"
        );
        assert_eq!(
            shown, export,
            "{fps} fps: draft frames equal the export frames"
        );

        // Unchanged: everything reused.
        let again =
            cut_media::render_preview_incremental(&p, &edl, dir.path(), &cache, &draft).unwrap();
        assert_eq!(
            (again.segments_rendered, again.segments_reused),
            (0, CUTS as usize)
        );

        // Clip 3 shows other source frames (same length): only its piece is remade.
        set_src(&mut p, "c3", 20_000, 20_000 + CLIP_MS);
        let edl = edl_from_project(&p);
        let moved =
            cut_media::render_preview_incremental(&p, &edl, dir.path(), &cache, &draft).unwrap();
        assert_eq!(
            (moved.segments_rendered, moved.segments_reused),
            (1, CUTS as usize - 1)
        );

        // Clip 3 grows by 100 ms (whole frames at 30 and 60 fps): its piece is
        // remade, the later pieces move by whole frames and are reused.
        set_src(&mut p, "c3", 20_000, 20_000 + CLIP_MS + 100);
        let edl = edl_from_project(&p);
        let grown =
            cut_media::render_preview_incremental(&p, &edl, dir.path(), &cache, &draft).unwrap();
        assert_eq!(
            (grown.segments_rendered, grown.segments_reused),
            (1, CUTS as usize - 1)
        );
        let export = {
            let _single = EnvGuard::set(&[("SHELLX_CUT_RENDER_SEGMENT_SEC", "100000")]);
            read_barcodes(&render(&p, dir.path(), &format!("grown{fps}.mp4")))
        };
        assert_eq!(
            read_barcodes(&grown.path),
            export,
            "{fps} fps: reused pieces still equal the export"
        );
    }
}

fn set_src(p: &mut Project, clip: &str, src_in: u64, src_out: u64) {
    let (track, i) = p
        .find_clip(clip)
        .map(|(t, i)| (t.to_string(), i))
        .expect("clip");
    if let cut_core::Clip::Media(c) = &mut p.track_mut(&track).unwrap().clips[i] {
        c.src_in_ms = src_in;
        c.src_out_ms = src_out;
    }
}
