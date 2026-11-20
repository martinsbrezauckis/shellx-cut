//! Item 170: windowed final renders with reversed clips finish, and every
//! frame is the right one.
//!
//! The source frames carry a barcode (sync cell, 3-bit clip id, 12-bit frame
//! index), so each rendered frame can be read back and compared with the frame
//! the edit defines. Every frame of every render is checked, not a sample.
//!
//! The recipe that hung in 0.6.115 is reproduced at test scale on Linux: a
//! reversed clip whose frames need more memory than the window's ffmpeg may
//! use (a cgroup MemoryHigh it cannot swap out of). Before the fix that window
//! never finished (the test ends it at its deadline); with the fix the window
//! is split so its reversed part fits.

use cut_core::{edl_from_project, Project};
use cut_media::{render_final, PathFence, RenderOptions, RenderPreset};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

/// Tests here change process-wide render settings (environment).
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

/// A barcoded clip: `id` 1..7, `secs` long, 30 fps, `w`x`h`, with a tone.
fn gen_barcoded(dir: &Path, name: &str, id: u32, w: u32, h: u32, secs: u32) -> PathBuf {
    let out = dir.join(name);
    let cell = format!(
        "if(eq(X,0),1,if(lt(X,4),mod(floor({id}/pow(2,3-X)),2),mod(floor(N/pow(2,15-X)),2)))"
    );
    let lum = format!("255*if(eq(Y,0),{cell},1-{cell})");
    let args: Vec<String> = vec![
        "-f".into(),
        "lavfi".into(),
        "-i".into(),
        format!("testsrc2=s={w}x{h}:r=30:d={secs}"),
        "-f".into(),
        "lavfi".into(),
        "-i".into(),
        format!("color=c=black:s=16x2:r=30:d={secs},format=gray,geq=lum='{lum}'"),
        "-f".into(),
        "lavfi".into(),
        "-i".into(),
        format!("sine=f=440:sample_rate=48000:d={secs}"),
        "-filter_complex".into(),
        format!(
            "[1:v]scale={w}:{}:flags=neighbor,format=yuv420p[bc];[0:v][bc]overlay=0:0[v]",
            h / 4
        ),
        "-map".into(),
        "[v]".into(),
        "-map".into(),
        "2:a".into(),
        "-c:v".into(),
        "libx264".into(),
        "-preset".into(),
        "ultrafast".into(),
        "-crf".into(),
        "16".into(),
        "-g".into(),
        "30".into(),
        "-pix_fmt".into(),
        "yuv420p".into(),
        "-c:a".into(),
        "aac".into(),
        "-t".into(),
        secs.to_string(),
        out.display().to_string(),
    ];
    cut_media::ffmpeg::run_ffmpeg(&args).expect("generate a barcoded clip");
    out
}

/// `(clip id, frame index)` of every frame of `media`; `None` when unreadable.
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
            if bits[0] != 1 {
                return None;
            }
            let id = bits[1..4].iter().fold(0, |n, b| n * 2 + b);
            let frame = bits[4..].iter().fold(0, |n, b| n * 2 + b);
            Some((id, frame))
        })
        .collect()
}

fn asset(path: &Path, w: u32, h: u32) -> Value {
    json!({"path": path.display().to_string(), "hash": format!("sha256:{}", path.display()),
           "probe": {"kind": "video", "width": w, "height": h, "fps": 30.0, "has_audio": true,
                     "raw": {"streams": [{"codec_type": "video", "pix_fmt": "yuv420p"}]}}})
}

/// One V1 clip: `(src_in, src_out, reverse, speed)`.
type Part = (u64, u64, bool, f64);

/// V1 = `parts` of asset a1 (barcode id 1); V2 = a 1 s overlay of a2 (id 2),
/// so the render takes the windowed path.
fn project(w: u32, h: u32, a1: &Path, a2: &Path, parts: &[Part]) -> Project {
    let v1: Vec<Value> = parts
        .iter()
        .enumerate()
        .map(|(i, (a, b, rev, speed))| {
            json!({"id": format!("c{}", i + 1), "asset": "a1", "src_in_ms": a,
                   "src_out_ms": b, "reverse": rev, "speed": speed})
        })
        .collect();
    serde_json::from_value(json!({
        "schema": "shellx-cut/1", "name": "reverse-windows",
        "settings": {"width": w, "height": h, "fps": 30.0, "audio_rate": 48000},
        "assets": {"a1": asset(a1, w, h), "a2": asset(a2, w, h)},
        "tracks": [
            {"id": "v1", "kind": "video", "clips": v1},
            {"id": "v2", "kind": "video", "clips": [
                {"id": "o1", "asset": "a2", "src_in_ms": 0, "src_out_ms": 1000}]},
        ],
    }))
    .expect("test project")
}

/// The `(id, frame, speed)` the edit shows at output frame `k` (30 fps): the
/// overlay for the first second, else the V1 clip under it. Reversed: the last
/// source frame of its range plays first.
fn expected(parts: &[Part], k: u32) -> (u32, f64, f64) {
    let t = f64::from(k) / 30.0;
    if t < 1.0 {
        return (2, f64::from(k), 1.0);
    }
    let mut start = 0.0;
    for (a, b, rev, speed) in parts {
        let (a, b) = (*a as f64 / 1000.0, *b as f64 / 1000.0);
        let dur = (b - a) / speed;
        if t < start + dur - 1e-9 {
            let off = (t - start) * speed;
            let src = if *rev { b - off - 1.0 / 30.0 } else { a + off };
            return (1, src * 30.0, *speed);
        }
        start += dur;
    }
    panic!("frame {k} is past the edit");
}

struct Check {
    frames: usize,
    wrong: Vec<String>,
}

fn check_every_frame(codes: &[Option<(u32, u32)>], parts: &[Part]) -> Check {
    let mut wrong = Vec::new();
    for (k, code) in codes.iter().enumerate() {
        let (id, want, speed) = expected(parts, k as u32);
        // At normal speed every frame is exact (a frame lost or doubled at a
        // seam shifts the rest by one); a sped-up clip is resampled to 30 fps,
        // so there the frame may be its neighbour.
        let tolerance = if speed == 1.0 { 0.01 } else { 1.0 + speed };
        match code {
            Some((got_id, got)) if *got_id == id && (f64::from(*got) - want).abs() <= tolerance => {
            }
            other => wrong.push(format!(
                "frame {k}: want id {id} frame {want:.1}, got {other:?}"
            )),
        }
    }
    Check {
        frames: codes.len(),
        wrong,
    }
}

fn render(
    project: &Project,
    dir: &Path,
    name: &str,
    deadline: Duration,
) -> Result<PathBuf, cut_core::CutError> {
    let edl = edl_from_project(project);
    let fence = PathFence::new(dir).unwrap();
    let out = dir.join(name);
    let control = cut_media::ffmpeg::RenderProcessControl::bounded(deadline, || false);
    cut_media::ffmpeg::with_render_process_control(&control, || {
        render_final(
            project,
            &edl,
            &fence,
            &out,
            &RenderPreset::default(),
            RenderOptions::default(),
            None,
        )
    })?;
    Ok(out)
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

/// Two reversed clips, one of them sped up, across many 4 s window seams:
/// every frame of the windowed render is the frame the edit defines, and the
/// same as the whole-timeline render's.
#[test]
fn reversed_clips_across_window_seams_keep_every_frame() {
    let _env = ENV.lock().unwrap_or_else(|e| e.into_inner());
    let dir = private_dir();
    let a1 = gen_barcoded(dir.path(), "a1.mp4", 1, 320, 180, 60);
    let a2 = gen_barcoded(dir.path(), "a2.mp4", 2, 320, 180, 2);
    let parts: [Part; 3] = [
        (0, 5000, false, 1.0),
        (10_000, 25_000, true, 1.0),
        (30_000, 45_000, true, 1.5),
    ];
    let p = project(320, 180, &a1, &a2, &parts);
    let whole = {
        let _single = EnvGuard::set(&[("SHELLX_CUT_RENDER_SEGMENT_SEC", "100000")]);
        render(&p, dir.path(), "whole.mp4", Duration::from_secs(300)).expect("whole render")
    };
    let windowed = {
        let _windows = EnvGuard::set(&[("SHELLX_CUT_RENDER_WINDOW_SEC", "4")]);
        render(&p, dir.path(), "windowed.mp4", Duration::from_secs(300)).expect("windowed render")
    };
    let (w, s) = (read_barcodes(&whole), read_barcodes(&windowed));
    let check = check_every_frame(&s, &parts);
    eprintln!(
        "reversed across seams: {} frames, {} wrong",
        check.frames,
        check.wrong.len()
    );
    assert_eq!(check.frames, 900, "a 30 s edit at 30 fps");
    assert!(
        check.wrong.is_empty(),
        "{:#?}",
        &check.wrong[..check.wrong.len().min(12)]
    );
    let differ: Vec<usize> = (0..w.len().min(s.len()))
        .filter(|k| w[*k] != s[*k])
        .collect();
    assert_eq!(w.len(), s.len());
    assert!(
        differ.is_empty(),
        "windowed differs from whole at frames {differ:?}"
    );
}

/// The 0.6.115 recipe at test scale: a reversed clip in a window whose ffmpeg
/// may use 600 MB (cgroup MemoryHigh, no swap). 30 s of reversed 960x540 is
/// 700 MB of frames: before the fix that window never finished. Now it is
/// split to the window budget (256 MiB here) and every frame is right.
#[cfg(target_os = "linux")]
#[test]
fn a_reversed_clip_bigger_than_the_window_memory_limit_finishes() {
    let _env = ENV.lock().unwrap_or_else(|e| e.into_inner());
    let dir = private_dir();
    let a1 = gen_barcoded(dir.path(), "a1.mp4", 1, 960, 540, 60);
    let a2 = gen_barcoded(dir.path(), "a2.mp4", 2, 960, 540, 2);
    let parts: [Part; 1] = [(0, 60_000, true, 1.0)];
    let p = project(960, 540, &a1, &a2, &parts);
    let limited = systemd_user_scope().then(|| {
        let wrapper = dir.path().join("ffmpeg-limited.sh");
        std::fs::write(
            &wrapper,
            format!(
                // The engine already runs this inside a scope named after its pid
                // (run-p<pid>-...), so this inner scope needs a name of its own.
                "#!/bin/sh\nexec systemd-run --user --scope --quiet --collect \
                 --unit=cut-test-limit-$$-$(date +%s%N) -p MemoryHigh=600M \
                 -p MemoryMax=2G -p MemorySwapMax=0 -- {} \"$@\"\n",
                cut_media::ffmpeg::ffmpeg_bin().to_string_lossy()
            ),
        )
        .unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&wrapper, std::fs::Permissions::from_mode(0o700)).unwrap();
        wrapper
    });
    if limited.is_none() {
        eprintln!(
            "reverse window memory: no systemd user session here, so the 600 MB limit cannot be \
             set; this run checks the frames only"
        );
    }
    let wrapper = limited.as_ref().map(|p| p.display().to_string());
    let mut env = vec![
        ("SHELLX_CUT_RENDER_WINDOW_SEC", "30"),
        ("SHELLX_CUT_RENDER_WINDOW_BUDGET_MB", "256"),
    ];
    if let Some(w) = wrapper.as_deref() {
        env.push(("SHELLX_CUT_FFMPEG", w));
    }
    let _guard = EnvGuard::set(&env);
    let started = std::time::Instant::now();
    let out = render(&p, dir.path(), "limited.mp4", Duration::from_secs(240))
        .unwrap_or_else(|e| panic!("the windowed reversed render did not finish: {e:?}"));
    eprintln!("reverse window memory: rendered in {:?}", started.elapsed());
    let check = check_every_frame(&read_barcodes(&out), &parts);
    assert_eq!(check.frames, 1800);
    assert!(
        check.wrong.is_empty(),
        "{:#?}",
        &check.wrong[..check.wrong.len().min(12)]
    );
}

#[cfg(target_os = "linux")]
fn systemd_user_scope() -> bool {
    std::process::Command::new("systemd-run")
        .args([
            "--user",
            "--scope",
            "--quiet",
            "-p",
            "MemoryHigh=64M",
            "--",
            "true",
        ])
        .stdin(std::process::Stdio::null())
        .output()
        .is_ok_and(|o| o.status.success())
}

cut_core::isolate_test_process!();
