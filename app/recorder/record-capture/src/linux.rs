//! linux.rs — live screen + input capture on Linux via the XDG Desktop Portal.
//!
//! Compiled ONLY for `cfg(target_os = "linux")` + the `capture-linux` feature.
//! - SCREEN: the **ScreenCast portal** (`ashpd`) grants a PipeWire node (consent
//!   dialog, with a reusable restore token cached so later runs skip it). The node
//!   is encoded by shelling to **GStreamer** `pipewiresrc ! videoconvert ! x264enc
//!   ! mp4mux` — chosen over `pipewire-rs` because gst handles PipeWire buffer /
//!   DMA-BUF / format negotiation. This is the Wayland-correct path (works on X11
//!   GNOME too); it replaces the legacy ffmpeg `x11grab`.
//! - CONSTANT FPS: mutter's screencast is DAMAGE-DRIVEN — a static screen emits a
//!   tiny burst then nothing. So gst captures sparse (real PTS) and we normalize to
//!   constant fps + EXACT wall-clock duration with an ffmpeg `fps,tpad=clone` pass
//!   (holds the last frame across pauses — correct, and matches the constant-fps
//!   source the renderer expects). Proven on Linux: 7-frame static raw → 180f/6.000s.
//! - INPUT: the shared rdevin hook (X11). On a Wayland session rdevin can't hook
//!   globally (by design); Wayland global input requires libei + the RemoteDesktop portal.
//! - MIC: cpal (shared mic.rs).
//!
//! RUNTIME: needs a logged-in desktop with xdg-desktop-portal + a PipeWire server,
//! and the user session bus (XDG_RUNTIME_DIR / DBUS_SESSION_BUS_ADDRESS) — inherited
//! when run inside the session. gst-launch-1.0 with the pipewire plugin must be
//! installed (gstreamer1.0-pipewire).

use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use ashpd::desktop::screencast::{CursorMode, Screencast, SelectSourcesOptions, SourceType};
use ashpd::desktop::PersistMode;
use enumflags2::BitFlags;

use record_core::{
    error_codes, ClickSample, CursorSample, EventTrack, KeySample, Monitor as RMonitor,
    RecordError, Result, ScrollSample, Settings,
};

use crate::{input, Capture, CaptureConfig, CaptureOutput};

fn ffmpeg_bin() -> String {
    std::env::var("SHELLX_RECORD_FFMPEG").unwrap_or_else(|_| "ffmpeg".to_string())
}
fn ffprobe_bin() -> String {
    std::env::var("SHELLX_RECORD_FFPROBE").unwrap_or_else(|_| "ffprobe".to_string())
}
fn gst_bin() -> String {
    std::env::var("SHELLX_RECORD_GST").unwrap_or_else(|_| "gst-launch-1.0".to_string())
}

/// Pick the input backend: evdev on Wayland (rdevin can't hook it), else rdevin
/// (X11, absolute coords). Override with `SHELLX_RECORD_INPUT=evdev|rdevin`.
fn use_evdev_input() -> bool {
    match std::env::var("SHELLX_RECORD_INPUT").ok().as_deref() {
        Some("evdev") => return true,
        Some("rdevin") => return false,
        _ => {}
    }
    if std::env::var("XDG_SESSION_TYPE")
        .map(|v| v.eq_ignore_ascii_case("wayland"))
        .unwrap_or(false)
    {
        return true;
    }
    std::env::var("WAYLAND_DISPLAY").is_ok() && std::env::var("DISPLAY").is_err()
}

/// True on a Wayland session (where gst can't get the cursor metadata, so we use the
/// pipewire-rs unified path). X11 stays on gst+rdevin (already pixel-perfect cursor).
fn on_wayland() -> bool {
    std::env::var("XDG_SESSION_TYPE")
        .map(|v| v.eq_ignore_ascii_case("wayland"))
        .unwrap_or(false)
        || (std::env::var("WAYLAND_DISPLAY").is_ok() && std::env::var("DISPLAY").is_err())
}

/// Whether to use the pipewire-rs unified Wayland capture (frames + absolute cursor).
/// Override with `SHELLX_RECORD_WAYLAND_CAPTURE=pipewire|gst` (gst = legacy, cursor
/// falls back to the evdev relative approximation).
fn use_wayland_pw() -> bool {
    match std::env::var("SHELLX_RECORD_WAYLAND_CAPTURE")
        .ok()
        .as_deref()
    {
        Some("gst") => false,
        Some("pipewire") => true,
        _ => on_wayland(),
    }
}

/// Cursor position at (nearest) time `t_ms` from a sorted-ish metadata cursor track —
/// used to give Wayland clicks an exact position (evdev only gives relative motion).
fn cursor_at(cursor: &[CursorSample], t_ms: u64) -> Option<(f64, f64)> {
    let mut best: Option<(&CursorSample, i64)> = None;
    for s in cursor {
        let d = (s.t_ms as i64 - t_ms as i64).abs();
        if best.map(|(_, bd)| d < bd).unwrap_or(true) {
            best = Some((s, d));
        }
    }
    best.map(|(s, _)| (s.x, s.y))
}

fn cap_err(ctx: &str, e: impl std::fmt::Display) -> RecordError {
    RecordError::new(error_codes::CAPTURE, ctx, e.to_string()).with_action(
        "ensure a desktop session is logged in with xdg-desktop-portal + PipeWire, \
         the user session bus is reachable (XDG_RUNTIME_DIR/DBUS_SESSION_BUS_ADDRESS), \
         and gst-launch-1.0 + the pipewire plugin are installed (gstreamer1.0-pipewire)",
    )
}

fn token_path_from(
    xdg_cache_home: Option<&std::ffi::OsStr>,
    home: Option<&std::ffi::OsStr>,
) -> Option<std::path::PathBuf> {
    let absolute = |value: Option<&std::ffi::OsStr>| {
        value
            .filter(|value| !value.is_empty())
            .map(std::path::PathBuf::from)
            .filter(|value| value.is_absolute())
    };
    let base =
        absolute(xdg_cache_home).or_else(|| absolute(home).map(|home| home.join(".cache")))?;
    Some(base.join("shellx-record/screencast.token"))
}

/// Cache path for the ScreenCast restore token (skip the consent dialog on re-runs).
fn token_path() -> Option<std::path::PathBuf> {
    let xdg_cache_home = std::env::var_os("XDG_CACHE_HOME");
    let home = std::env::var_os("HOME");
    token_path_from(xdg_cache_home.as_deref(), home.as_deref())
}
fn read_token() -> Option<String> {
    std::fs::read_to_string(token_path()?)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}
fn write_token(tok: &str) {
    let Some(p) = token_path() else {
        return;
    };
    if let Some(dir) = p.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(&p, tok);
}

/// Probe a file's video dimensions.
fn probe_dims(path: &str) -> Option<(u32, u32)> {
    let out = Command::new(ffprobe_bin())
        .args([
            "-v",
            "error",
            "-select_streams",
            "v:0",
            "-show_entries",
            "stream=width,height",
            "-of",
            "csv=p=0:s=,",
            path,
        ])
        .output()
        .ok()?;
    let s = String::from_utf8_lossy(&out.stdout);
    let mut it = s.lines().next()?.trim().split(',');
    Some((it.next()?.parse().ok()?, it.next()?.parse().ok()?))
}

/// What the async portal+gst capture phase returns to the sync caller.
struct CapPhase {
    w: u32,
    h: u32,
    duration_ms: u64,
    audio: Option<String>,
    cursor: Vec<CursorSample>,
    clicks: Vec<ClickSample>,
    scrolls: Vec<ScrollSample>,
    keys: Vec<KeySample>,
}

/// Process-global tokio runtime that drives ALL portal / D-Bus work.
///
/// MUST be shared across captures — NOT rebuilt per call. ashpd caches the session-bus
/// `zbus::Connection` in a process-global `static OnceLock` (ashpd `proxy.rs`), and with
/// the `zbus/tokio` feature that connection's socket-reader task is spawned via
/// `tokio::task::spawn` onto whatever runtime is current the FIRST time the connection is
/// built (zbus `abstractions/executor.rs`). If `capture()` built a fresh runtime per call
/// and dropped it (the old code), dropping runtime #1 ABORTS that reader task — leaving the
/// globally-cached connection with a dead I/O driver. The 2nd capture in the same process
/// then reuses the dead connection and its first D-Bus call never gets a reply → the portal
/// wedge. One long-lived runtime keeps the reader alive for the whole process, so
/// every later capture reuses a LIVE connection. (Pairs with the explicit `session.close()`
/// below, which frees the server-side ScreenCast session so mutter's concurrent-session cap
/// is never hit.)
fn shared_runtime() -> Result<&'static tokio::runtime::Runtime> {
    static RT: std::sync::OnceLock<tokio::runtime::Runtime> = std::sync::OnceLock::new();
    if let Some(rt) = RT.get() {
        return Ok(rt);
    }
    // Build outside get_or_init (init is fallible). On a lost init race our runtime is
    // dropped unused — harmless, since no zbus connection was bound to it yet.
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|e| cap_err("tokio runtime", e))?;
    Ok(RT.get_or_init(|| rt))
}

/// Live Linux capture backend (ScreenCast portal + GStreamer + rdevin + cpal).
pub struct LinuxCapture;

impl LinuxCapture {
    pub fn new() -> Self {
        Self
    }
}

impl Capture for LinuxCapture {
    fn capture(&self, cfg: &CaptureConfig, stop: Arc<AtomicBool>) -> Result<CaptureOutput> {
        // Open-ended capture: `None` means "run until the external `stop` flag
        // is set" — represented as a huge cap (≈2.9e15 ms ≈ 92 000 years) so the
        // wayland-path deadline `start + dur` is effectively never reached and ONLY
        // `stop` ends it; the gst/x11 path polls `stop` directly (below). A concrete
        // `duration_ms` still acts as an upper bound (whichever fires first wins).
        let dur = cfg.duration_ms.unwrap_or(u64::MAX / 4);
        let fps = cfg.fps.max(1.0);

        let out_dir = cfg.out_dir.trim_end_matches('/').to_string();
        std::fs::create_dir_all(&out_dir).map_err(|e| cap_err("create output dir", e))?;
        let raw = format!("{out_dir}/raw.mp4");
        let path = format!("{out_dir}/source.mp4");

        // The ScreenCast session must stay alive for the whole gst capture, so the
        // portal handshake AND the capture window run inside one async scope. The
        // input hook + mic are sync threads started AFTER consent (so their wall
        // clock aligns with the source's frame 0).
        //
        // Use the PROCESS-PERSISTENT runtime (not a fresh per-capture one) so the
        // ashpd-cached zbus connection's reader task survives between captures — see
        // `shared_runtime` for the full rationale. A fresh-then-dropped runtime per call
        // is exactly what wedged the 2nd capture.
        let rt = shared_runtime()?;

        let raw_for_async = raw.clone();
        let audio_wanted = cfg.audio;
        let capture_keys = cfg.capture_keys;
        let out_dir_async = out_dir.clone();
        // The EXTERNAL stop flag (set by `screen_record.stop`) drives mic + input
        // + the capture loop. Moved into the async scope below.
        let stop_async = stop.clone();
        // Wayland → pipewire-rs unified capture (frames + absolute cursor metadata).
        let wayland_pw = use_wayland_pw();
        let ff_for_async = ffmpeg_bin();

        let phase: Result<CapPhase> = rt.block_on(async move {
            let proxy = Screencast::new()
                .await
                .map_err(|e| cap_err("connect ScreenCast portal", e))?;
            let session = proxy
                .create_session(Default::default())
                .await
                .map_err(|e| cap_err("create portal session", e))?;

            let mut opts = SelectSourcesOptions::default()
                // Wayland-pw needs the cursor as METADATA; the gst path hides it (we
                // re-render a synthetic cursor) and reads position from rdevin/evdev.
                .set_cursor_mode(if wayland_pw {
                    CursorMode::Metadata
                } else {
                    CursorMode::Hidden
                })
                .set_sources(BitFlags::from(SourceType::Monitor))
                .set_multiple(false)
                // ExplicitlyRevoked (NOT Application): captures are short-lived, and
                // PersistMode::Application scopes the grant to the *running application's*
                // lifetime — so it lapses when the capture/app exits and the cached restore
                // token is re-prompted on the next run. ExplicitlyRevoked keeps the grant
                // (and token) valid until the user revokes screen sharing in desktop settings,
                // which is what makes
                // unattended / agent-driven re-captures skip the consent dialog. The first
                // capture still prompts once to mint the durable token.
                .set_persist_mode(PersistMode::ExplicitlyRevoked);
            if let Some(tok) = read_token() {
                // set_restore_token copies internally, so the borrow need only last the call.
                opts = opts.set_restore_token(tok.as_str());
            }
            proxy
                .select_sources(&session, opts)
                .await
                .map_err(|e| cap_err("select portal sources", e))?;

            let streams = proxy
                .start(&session, None, Default::default())
                .await
                .map_err(|e| cap_err("start portal cast (consent)", e))?
                .response()
                .map_err(|e| cap_err("portal cast response", e))?;
            let sv = streams.streams();
            let stream = sv
                .first()
                .ok_or_else(|| cap_err("portal granted no streams", "empty stream list"))?;
            let node = stream.pipe_wire_node_id();
            let (sw, sh) = stream
                .size()
                .map(|(w, h)| (w.max(0) as u32, h.max(0) as u32))
                .unwrap_or((1920, 1080));
            if let Some(tok) = streams.restore_token() {
                write_token(tok);
            }

            // Wayland: open the PipeWire remote fd so pipewire-rs can read frames +
            // SPA_META_Cursor from the node (gst connects on its own, so the gst path
            // doesn't need this).
            let pw_fd = if wayland_pw {
                Some(
                    proxy
                        .open_pipe_wire_remote(&session, Default::default())
                        .await
                        .map_err(|e| cap_err("open pipewire remote", e))?,
                )
            } else {
                None
            };

            // ----- capture window begins (input + mic aligned to source frame 0) -----
            // Use the EXTERNAL stop flag (passed in) rather than a fresh internal
            // one — that is what lets `screen_record.stop` end this capture early.
            let stop = stop_async;
            // Mic in PARALLEL — never block the screen on it (an 8 s "ready" wait starved
            // the screen capture when no/slow input device was present; the Record surface
            // pre-warms via `mic::warm`). No device → mic thread Errs → audio None.
            let mic_handle = if audio_wanted {
                let ready = Arc::new(AtomicBool::new(false));
                Some(crate::mic::spawn_mic(
                    format!("{out_dir_async}/mic.wav"),
                    stop.clone(),
                    ready,
                ))
            } else {
                None
            };

            let start = Instant::now();
            let input = if use_evdev_input() {
                crate::input_evdev::spawn_evdev_listener(start, stop.clone(), capture_keys, sw, sh)
            } else {
                input::spawn_listener(start, stop.clone(), capture_keys)
            };

            // Screen capture → raw.mp4. Two backends; the session is held alive for both.
            let meta_cursor: Option<Vec<CursorSample>> = if let Some(fd) = pw_fd {
                // WAYLAND: pipewire-rs reads frames (→ raw.mp4) AND SPA_META_Cursor in one
                // stream. Blocking PipeWire loop → run on a blocking thread so the tokio
                // executor keeps driving the portal session's zbus task (session alive).
                let raw = raw_for_async.clone();
                let ff = ff_for_async.clone();
                let start_c = start;
                let stop_c = stop.clone();
                let cur = tokio::task::spawn_blocking(move || {
                    crate::wayland_pw::capture(Some(fd), node, dur, start_c, stop_c, &raw, &ff)
                })
                .await
                .map_err(|e| cap_err("wayland_pw join", e))?
                .map_err(|e| cap_err("wayland_pw capture", e))?;
                stop.store(true, Ordering::Relaxed);
                Some(cur)
            } else {
                // X11 (or forced gst): gst SPARSE capture (real PTS) of the node → raw.mp4.
                let mut child = tokio::process::Command::new(gst_bin())
                    .arg("-e") // SIGINT → EOS → finalize the mp4
                    .args([
                        "pipewiresrc",
                        &format!("path={node}"),
                        "!",
                        "videoconvert",
                        "!",
                        "x264enc",
                        "speed-preset=ultrafast",
                        "tune=zerolatency",
                        "!",
                        "mp4mux",
                        "!",
                        "filesink",
                        &format!("location={raw_for_async}"),
                    ])
                    .spawn()
                    .map_err(|e| cap_err("spawn gst-launch-1.0", e))?;

                // Poll the external stop flag (and the deadline) instead of a
                // single fixed sleep — open-ended capture (`dur == u64::MAX/4`) would
                // otherwise sleep ~forever, ignoring `screen_record.stop`. Whichever
                // fires first ends the capture; check ~10×/s for prompt stop response.
                while !stop.load(Ordering::Relaxed) && start.elapsed() < Duration::from_millis(dur)
                {
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
                if let Some(pid) = child.id() {
                    // SIGINT so `-e` finalizes the muxer (SIGKILL would corrupt the mp4).
                    let _ = Command::new("kill")
                        .args(["-INT", &pid.to_string()])
                        .status();
                }
                let st = child.wait().await.map_err(|e| cap_err("wait gst", e))?;
                stop.store(true, Ordering::Relaxed);
                if !st.success() {
                    return Err(cap_err(
                        "gst screen capture failed",
                        format!("gst exit {st}"),
                    ));
                }
                None
            };
            let duration_ms = start.elapsed().as_millis() as u64;

            let audio = mic_handle.and_then(|h| {
                match crate::mic::join_bounded(h, Duration::from_secs(2)) {
                    Some(Ok(p)) => Some(p),
                    Some(Err(e)) => {
                        eprintln!("warning: mic capture failed, recording without audio: {e}");
                        None
                    }
                    None => {
                        eprintln!(
                            "warning: mic capture did not stop within 2s, recording without audio"
                        );
                        None
                    }
                }
            });
            let (evdev_cursor, mut clicks, scrolls, keys) = input.lock().unwrap().snapshot();
            // Cursor source: Wayland metadata (absolute, exact) when present, else the
            // rdevin/evdev track. With the metadata track, give each click the cursor
            // position at its time (evdev clicks only carried the relative approximation).
            let cursor = match meta_cursor {
                Some(c) if !c.is_empty() => {
                    for cl in clicks.iter_mut() {
                        if let Some((x, y)) = cursor_at(&c, cl.t_ms) {
                            cl.x = x;
                            cl.y = y;
                        }
                    }
                    c
                }
                _ => evdev_cursor,
            };
            // Explicitly CLOSE the portal ScreenCast session so mutter frees the
            // server-side session + its PipeWire node NOW. ashpd's `Session` has NO `Drop`
            // (see ashpd `session.rs`) — dropping it sends no `Close` — and the underlying
            // D-Bus connection is process-global (cached), so without this the granted
            // session lingers until the process exits and a later capture can collide with
            // mutter's concurrent-ScreenCast-session cap. Best-effort: a failed close must
            // never fail an otherwise-good capture. (The capture window is already over —
            // gst/pipewire have released the node — so closing here is safe.)
            if let Err(e) = session.close().await {
                eprintln!("warning: portal session close failed (non-fatal): {e}");
            }
            // session drops here — capture is done.
            Ok(CapPhase {
                w: sw,
                h: sh,
                duration_ms,
                audio,
                cursor,
                clicks,
                scrolls,
                keys,
            })
        });
        let phase = phase?;

        // CFR normalize: sparse raw → constant fps + EXACT wall-clock duration.
        // fps=<fps> makes it CFR; tpad clones the last frame across pauses; -t cuts
        // to the true captured duration. (Proven on Linux: static raw → exact 6.000s.)
        let dur_s = format!("{:.3}", phase.duration_ms as f64 / 1000.0);
        let vf = format!("fps={},tpad=stop_mode=clone:stop_duration=3600", fps as u32);
        let status = Command::new(ffmpeg_bin())
            .args([
                "-v", "error", "-y", "-i", &raw, "-vf", &vf, "-t", &dur_s, "-c:v", "libx264",
                "-pix_fmt", "yuv420p", "-crf", "20", "-preset", "medium", &path,
            ])
            .status()
            .map_err(|e| cap_err("ffmpeg CFR normalize spawn", e))?;
        if !status.success() {
            return Err(cap_err(
                "CFR normalize failed",
                format!("ffmpeg exit {status}"),
            ));
        }
        let _ = std::fs::remove_file(&raw); // drop the throwaway sparse capture

        let (w, h) = probe_dims(&path).unwrap_or((phase.w, phase.h));
        let events = EventTrack {
            duration_ms: phase.duration_ms,
            screen_w: w,
            screen_h: h,
            monitors: vec![RMonitor {
                id: 0,
                x: 0,
                y: 0,
                w,
                h,
                primary: true,
            }],
            cursor: phase.cursor,
            clicks: phase.clicks,
            scrolls: phase.scrolls,
            keys: phase.keys,
        };
        Ok(CaptureOutput {
            source_video: path,
            events,
            webcam_video: None,
            audio: phase.audio,
            settings: Settings {
                width: w,
                height: h,
                fps: fps as f32,
                audio_rate: 48_000,
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::token_path_from;
    use std::ffi::OsStr;
    use std::path::PathBuf;

    #[test]
    fn restore_token_path_requires_an_absolute_cache_base() {
        assert_eq!(
            token_path_from(Some(OsStr::new("/cache")), Some(OsStr::new("/home/user"))),
            Some(PathBuf::from("/cache/shellx-record/screencast.token"))
        );
        assert_eq!(
            token_path_from(None, Some(OsStr::new("/home/user"))),
            Some(PathBuf::from(
                "/home/user/.cache/shellx-record/screencast.token"
            ))
        );
        assert_eq!(token_path_from(None, None), None);
        assert_eq!(
            token_path_from(Some(OsStr::new("relative")), Some(OsStr::new("home"))),
            None
        );
        assert_eq!(
            token_path_from(Some(OsStr::new("")), Some(OsStr::new(""))),
            None
        );
    }
}
