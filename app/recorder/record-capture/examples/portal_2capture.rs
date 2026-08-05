//! portal_2capture.rs — live regression test for the second-capture-per-process
//! portal wedge on Linux/Wayland.
//!
//! The bug: `LinuxCapture::capture()` built a FRESH tokio runtime per call and
//! dropped it; dropping runtime #1 aborted ashpd's process-global zbus
//! connection reader, so the 2nd capture in the SAME process reused a dead
//! connection and wedged on its first portal D-Bus call. The fix
//! (`shared_runtime()` + explicit `session.close()`) keeps the reader alive
//! across captures.
//!
//! This harness reproduces the exact trigger: TWO captures back-to-back inside
//! ONE process. PASS = both return frames. A true wedge HANGS the 2nd capture
//! forever — so run this under an external `timeout` on the rig; a kill by
//! timeout is the FAIL signal.
//!
//! Run on a logged-in, UNLOCKED Wayland/X11 GNOME session:
//!   cargo run --release --features capture-linux --example portal_2capture
//! The FIRST capture mints the durable restore token (one consent dialog); the
//! 2nd should proceed without a prompt and is the one that used to wedge.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use record_capture::{live_capture, CaptureConfig};

/// Drive one short capture; return Ok(bytes) for the produced source video.
fn one_capture(cap: &dyn record_capture::Capture, n: u32) -> Result<u64, String> {
    let out_guard = tempfile::Builder::new()
        .prefix(&format!("shellx-cut-wedge-cap-{n}-"))
        .tempdir()
        .map_err(|e| format!("create secure capture directory: {e}"))?;
    let out_dir = out_guard.path().to_string_lossy().into_owned();

    let cfg = CaptureConfig {
        duration_ms: Some(3000), // short bounded window
        fps: 15.0,
        capture_cursor: false,
        monitor: None,
        window: None,
        audio: false,
        system_audio: false,
        capture_keys: false,
        out_dir: out_dir.clone(),
    };

    // Backstop: if the bounded deadline somehow doesn't fire, force-stop at 6s so
    // a *working* capture can't run away. A genuine portal WEDGE happens in the
    // D-Bus handshake BEFORE the capture loop, so `stop` can't rescue it — that is
    // intentional: the external `timeout` on the rig is what catches a real wedge.
    let stop = Arc::new(AtomicBool::new(false));
    let stop_bg = stop.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_secs(6));
        stop_bg.store(true, Ordering::SeqCst);
    });

    eprintln!("[harness] capture #{n}: START (out={out_dir})");
    let t = Instant::now();
    match cap.capture(&cfg, stop) {
        Ok(o) => {
            let bytes = std::fs::metadata(&o.source_video)
                .map(|m| m.len())
                .unwrap_or(0);
            eprintln!(
                "[harness] capture #{n}: OK in {:?} -> {} ({} bytes, audio={:?})",
                t.elapsed(),
                o.source_video,
                bytes,
                o.audio
            );
            if bytes == 0 {
                return Err(format!("capture #{n} produced a 0-byte source video"));
            }
            Ok(bytes)
        }
        Err(e) => Err(format!("capture #{n} errored in {:?}: {e}", t.elapsed())),
    }
}

fn main() {
    let cap = match live_capture() {
        Some(c) => c,
        None => {
            eprintln!(
                "FAIL: no live-capture backend compiled (need --features capture-linux on Linux)"
            );
            std::process::exit(3);
        }
    };

    // Capture #1 — mints / reuses the restore token.
    if let Err(e) = one_capture(cap.as_ref(), 1) {
        eprintln!("FAIL: {e}");
        std::process::exit(2);
    }

    eprintln!("[harness] ---- between captures (same process) ----");

    // Capture #2 — the historically-wedging one. If the fix regressed, this HANGS
    // (caught by the external timeout) or errors.
    if let Err(e) = one_capture(cap.as_ref(), 2) {
        eprintln!("FAIL: {e}");
        std::process::exit(2);
    }

    println!("PASS: both captures completed in one process — second-capture wedge not reproduced");
}
