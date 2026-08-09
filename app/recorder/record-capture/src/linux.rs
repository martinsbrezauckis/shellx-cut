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

use std::path::Path;
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use ashpd::desktop::screencast::{CursorMode, Screencast, SelectSourcesOptions, SourceType};
use ashpd::desktop::PersistMode;
use enumflags2::BitFlags;

use record_core::{error_codes, EventTrack, Monitor as RMonitor, RecordError, Result, Settings};

use crate::linux_capture_state::{CapPhase, CapturedInput, RecordedInput};
use crate::linux_media::probe_dims;
use crate::linux_token::{read_token, write_token};
use crate::{
    checkpoint::Checkpoints, cursor_correlation, input, Capture, CaptureConfig, CaptureOutput,
};

fn ffmpeg_bin() -> String {
    std::env::var("SHELLX_RECORD_FFMPEG").unwrap_or_else(|_| "ffmpeg".to_string())
}
fn ffprobe_bin() -> String {
    std::env::var("SHELLX_RECORD_FFPROBE").unwrap_or_else(|_| "ffprobe".to_string())
}
fn gst_bin() -> String {
    std::env::var("SHELLX_RECORD_GST").unwrap_or_else(|_| "gst-launch-1.0".to_string())
}

fn cap_err(ctx: &str, e: impl std::fmt::Display) -> RecordError {
    RecordError::new(error_codes::CAPTURE, ctx, e.to_string()).with_action(
        "ensure a desktop session is logged in with xdg-desktop-portal + PipeWire, \
         the user session bus is reachable (XDG_RUNTIME_DIR/DBUS_SESSION_BUS_ADDRESS), \
         and gst-launch-1.0 + the pipewire plugin are installed (gstreamer1.0-pipewire)",
    )
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
        let checkpoint_config = cfg.checkpoint.clone();
        let capture_clock = cfg.clock.clone();
        let out_dir_async = out_dir.clone();
        // The EXTERNAL stop flag (set by `screen_record.stop`) drives mic + input
        // + the capture loop. Moved into the async scope below.
        let stop_async = stop.clone();
        // Wayland → pipewire-rs unified capture (frames + absolute cursor metadata).
        let input_mode = cursor_correlation::session_input_mode();
        let wayland_pw = input_mode.use_pipewire_metadata;
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
            let cursor_geometry = cursor_correlation::PortalCursorGeometry::from_portal(
                stream.position(),
                stream.size(),
            );
            if let Some(tok) = streams.restore_token() {
                write_token(tok);
            }

            // Wayland: open the PipeWire remote fd so pipewire-rs can read frames +
            // SPA_META_Cursor from the node (gst connects on its own, so the gst path
            // doesn't need this).
            let mut pw_fd = if wayland_pw {
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
            let start = capture_clock
                .as_ref()
                .map(crate::CaptureClock::start)
                .unwrap_or_else(Instant::now);
            let mic_handle = if audio_wanted {
                let ready = Arc::new(AtomicBool::new(false));
                Some(crate::mic::spawn_mic(
                    format!("{out_dir_async}/mic.wav"),
                    stop.clone(),
                    ready,
                    start,
                ))
            } else {
                None
            };

            let input = if input_mode.use_evdev {
                crate::input_evdev::spawn_evdev_listener(start, stop.clone(), capture_keys, sw, sh)
            } else {
                input::spawn_listener(start, stop.clone(), capture_keys)
            };

            // Each interval closes its own MP4 before publication. A process death can
            // lose only the currently-open staging file; it can never promote raw.mp4.
            let mut checkpoints = Checkpoints::open(checkpoint_config.as_ref())?;
            let mut segment_start_ms = 0u64;
            let mut meta_cursor: Option<cursor_correlation::PipewireCursorCapture> = None;
            let duration_ms = loop {
                let segment = checkpoints
                    .as_mut()
                    .map(|owner| owner.begin(segment_start_ms))
                    .transpose()?;
                let segment_path = segment
                    .as_ref()
                    .map(|(_, path)| path.display().to_string())
                    .unwrap_or_else(|| raw_for_async.clone());
                let interval_end = checkpoints
                    .as_ref()
                    .map(|owner| segment_start_ms.saturating_add(owner.interval_ms()))
                    .unwrap_or(dur)
                    .min(dur);
                let (capture_start_ms, ended_ms) = if let Some(fd) = pw_fd.take() {
                    let ff = ff_for_async.clone();
                    let start_c = start;
                    let stop_c = stop.clone();
                    let cur = tokio::task::spawn_blocking(move || {
                        crate::wayland_pw::capture(
                            Some(fd),
                            node,
                            interval_end,
                            start_c,
                            stop_c,
                            &segment_path,
                            &ff,
                        )
                    })
                    .await
                    .map_err(|e| cap_err("wayland_pw join", e))?
                    .map_err(|e| cap_err("wayland_pw capture", e))?;
                    let cur_start_ms = cur.capture_start_ms;
                    let cur_end_ms = cur.capture_end_ms;
                    match meta_cursor.as_mut() {
                        Some(accumulated)
                            if accumulated.frame_width == cur.frame_width
                                && accumulated.frame_height == cur.frame_height =>
                        {
                            accumulated.metadata.extend(cur.metadata);
                            accumulated.capture_start_ms =
                                accumulated.capture_start_ms.min(cur.capture_start_ms);
                            accumulated.capture_end_ms =
                                accumulated.capture_end_ms.max(cur.capture_end_ms);
                        }
                        Some(_) => {
                            return Err(cap_err(
                                "rotate PipeWire checkpoint",
                                "captured frame dimensions changed across checkpoints",
                            ));
                        }
                        None => meta_cursor = Some(cur),
                    }
                    (cur_start_ms, cur_end_ms)
                } else {
                    let mut child = tokio::process::Command::new(gst_bin())
                        .arg("-e")
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
                            &format!("location={segment_path}"),
                        ])
                        .spawn()
                        .map_err(|e| cap_err("spawn gst-launch-1.0", e))?;
                    // GStreamer has no first-frame callback here; successful spawn is
                    // its encoder-start boundary on the shared capture clock.
                    let capture_start_ms =
                        u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX);
                    while !stop.load(Ordering::Relaxed)
                        && start.elapsed() < Duration::from_millis(interval_end)
                    {
                        tokio::time::sleep(Duration::from_millis(50)).await;
                    }
                    let capture_end_ms =
                        u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX);
                    if let Some(pid) = child.id() {
                        let _ = Command::new("kill")
                            .args(["-INT", &pid.to_string()])
                            .status();
                    }
                    let st = child.wait().await.map_err(|e| cap_err("wait gst", e))?;
                    if !st.success() {
                        return Err(cap_err(
                            "gst screen capture failed",
                            format!("gst exit {st}"),
                        ));
                    }
                    (capture_start_ms, capture_end_ms)
                };
                if let (Some(owner), Some((sequence, staging))) = (checkpoints.as_mut(), segment) {
                    owner.publish(
                        sequence,
                        &staging,
                        record_recovery::CheckpointFacts {
                            start_ms: capture_start_ms,
                            end_ms: ended_ms,
                            event_offset_ms: capture_start_ms,
                            // External sidecars remain on this global clock; their
                            // first packet is not available in this backend, so do
                            // not publish a guessed zero offset.
                            audio_offset_ms: None,
                        },
                    )?;
                }
                if stop.load(Ordering::Relaxed) || ended_ms >= dur {
                    break ended_ms;
                }
                if wayland_pw {
                    pw_fd = Some(
                        proxy
                            .open_pipe_wire_remote(&session, Default::default())
                            .await
                            .map_err(|e| cap_err("reopen pipewire remote", e))?,
                    );
                }
                // The next encoder does not exist until any portal-remote reopen and
                // prior segment verification finish. Measure its new wall-clock start
                // so stitching materializes all of that restart gap.
                segment_start_ms = u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX);
            };
            // End external mic/system-audio/input on the same capture clock before
            // any CPU-heavy stitch. The stitched timeline already includes measured
            // restart gaps, so post-capture publication time must not extend sidecars.
            stop.store(true, Ordering::Relaxed);
            if let Some(owner) = checkpoints.as_ref() {
                owner.stitch(&ff_for_async, &ffprobe_bin(), "raw.mp4")?;
            }
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
            let (cursor, mut clicks, scrolls, keys) = input.lock().unwrap().snapshot();
            // rdevin supplies desktop-global coordinates. Its selected portal
            // surface must be scaled against the *finalized* video dimensions, not
            // `stream.size()` (which is logical under compositor scaling). Defer it
            // until the synchronous caller has probed source.mp4 below.
            let input = if input_mode.use_evdev {
                let correlated = cursor_correlation::correlate_clicks(
                    input_mode,
                    &mut clicks,
                    cursor,
                    scrolls,
                    meta_cursor,
                    cursor_geometry,
                    None,
                );
                CapturedInput::Correlated(RecordedInput {
                    cursor: correlated.cursor,
                    clicks,
                    scrolls: correlated.scrolls,
                    cursor_correlation: correlated.status,
                })
            } else {
                CapturedInput::RdevinPending {
                    mode: input_mode,
                    cursor,
                    clicks,
                    scrolls,
                    portal_geometry: cursor_geometry,
                }
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
                input,
                keys,
            })
        });
        let phase = phase?;

        // CFR normalize: sparse raw → constant fps + EXACT wall-clock duration.
        // fps=<fps> makes it CFR; tpad clones the last frame across pauses; -t cuts
        // to the true captured duration. (Proven on Linux: static raw → exact 6.000s.)
        // `raw.mp4` was already checkpoint-verified, but this is the user-facing
        // normal source after CFR normalization. Decode it again before the
        // RecordingProject can name it; a successful encoder exit alone is not
        // proof that its final container has the gap-padded clock we promised.
        crate::linux_source_publication::normalize_and_publish(
            Path::new(&raw),
            Path::new(&path),
            phase.duration_ms,
            fps as u32,
            &ffmpeg_bin(),
            &ffprobe_bin(),
        )
        .map_err(|error| cap_err("normalize and publish source", error))?;
        let _ = std::fs::remove_file(&raw); // drop the throwaway sparse capture

        // The UI must still receive a usable track dimension if ffprobe is absent,
        // but rdevin coordinates may not be promoted to exact on that fallback:
        // there is no evidence that the logical portal size equals the encoded video.
        let finalized_dimensions = probe_dims(&ffprobe_bin(), &path);
        let (w, h) = finalized_dimensions.unwrap_or((phase.w, phase.h));
        let (cursor, clicks, scrolls, cursor_correlation) = match phase.input {
            CapturedInput::Correlated(input) => (
                input.cursor,
                input.clicks,
                input.scrolls,
                input.cursor_correlation,
            ),
            CapturedInput::RdevinPending {
                mode,
                cursor,
                mut clicks,
                scrolls,
                portal_geometry,
            } => {
                let correlated = cursor_correlation::correlate_clicks(
                    mode,
                    &mut clicks,
                    cursor,
                    scrolls,
                    None,
                    portal_geometry,
                    finalized_dimensions,
                );
                (
                    correlated.cursor,
                    clicks,
                    correlated.scrolls,
                    correlated.status,
                )
            }
        };
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
            cursor,
            clicks,
            scrolls,
            keys: phase.keys,
            cursor_correlation,
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
