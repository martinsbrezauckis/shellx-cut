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
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;

use ashpd::desktop::screencast::{CursorMode, Screencast, SelectSourcesOptions, SourceType};
use ashpd::desktop::PersistMode;
use enumflags2::BitFlags;

use record_core::{EventTrack, Monitor as RMonitor, Result, Settings};

use crate::active_capture_preview::{ControllerExclusionStatus, RecursionStatus};
use crate::linux_capture_state::{CapPhase, CapturedInput, RecordedInput};
use crate::linux_portal;
use crate::linux_runtime::{cap_err, ffmpeg_bin, ffprobe_bin, shared_runtime};
use crate::linux_token::{read_token, write_token};
use crate::{checkpoint::Checkpoints, cursor_correlation, Capture, CaptureConfig, CaptureOutput};

/// Live Linux capture backend (ScreenCast portal + GStreamer + rdevin + cpal).
pub struct LinuxCapture;

impl LinuxCapture {
    pub fn new() -> Self {
        Self
    }
}

impl Capture for LinuxCapture {
    fn capture(&self, cfg: &CaptureConfig, stop: Arc<AtomicBool>) -> Result<CaptureOutput> {
        // The portal may let a user share a display, but it does not publish a
        // durable fact that this app's controller was excluded or safely hidden.
        // Do not turn portal consent, a generic Stop, or a browser visibility
        // event into an exclusion claim.
        if let Some(placement) = cfg.controller_placement.as_ref() {
            placement.unavailable(
                "Linux portal capture does not provide a verifiable controller-exclusion or safe auto-hide state.",
            );
        }
        // Open-ended capture: `None` means "run until the external `stop` flag
        // is set" — represented as a huge cap (≈2.9e15 ms ≈ 92 000 years) so the
        // wayland-path deadline `start + dur` is effectively never reached and ONLY
        // `stop` ends it; the gst/x11 path polls `stop` directly (below). A concrete
        // `duration_ms` still acts as an upper bound (whichever fires first wins).
        let dur = cfg.duration_ms.unwrap_or(u64::MAX / 4);
        // All live backends share the v1 integer request policy. Keeping this
        // conversion in record-core prevents Linux's old `as u32` truncation
        // from diverging from Windows/macOS for fractional requests.
        let fps = record_core::backend_fps_v1(cfg.fps);

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
        let active_preview = cfg.active_preview.clone();

        let phase: Result<CapPhase> = rt.block_on(async move {
            let portal_deadline = linux_portal::pre_first_frame_deadline();
            let proxy = linux_portal::await_pre_first_frame(
                "connect ScreenCast portal",
                stop_async.as_ref(),
                portal_deadline,
                async {
                    Screencast::new()
                        .await
                        .map_err(|e| cap_err("connect ScreenCast portal", e))
                },
            )
            .await?;
            let session = linux_portal::await_pre_first_frame(
                "create portal session",
                stop_async.as_ref(),
                portal_deadline,
                async {
                    proxy
                        .create_session(Default::default())
                        .await
                        .map_err(|e| cap_err("create portal session", e))
                },
            )
            .await?;

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
            if let Err(error) = linux_portal::await_pre_first_frame(
                "select portal sources",
                stop_async.as_ref(),
                portal_deadline,
                async {
                    proxy
                        .select_sources(&session, opts)
                        .await
                        .map_err(|e| cap_err("select portal sources", e))
                },
            )
            .await
            {
                linux_portal::close_session(&session).await;
                return Err(error);
            }

            let streams = match linux_portal::await_pre_first_frame(
                "start portal cast (consent)",
                stop_async.as_ref(),
                portal_deadline,
                async {
                    let parent_window = linux_portal::portal_parent_window();
                    proxy
                        .start(&session, parent_window.as_ref(), Default::default())
                        .await
                        .map_err(|e| cap_err("start portal cast (consent)", e))
                },
            )
            .await
            {
                Ok(request) => match request
                    .response()
                    .map_err(|e| cap_err("portal cast response", e))
                {
                    Ok(streams) => streams,
                    Err(error) => {
                        linux_portal::close_session(&session).await;
                        return Err(error);
                    }
                },
                Err(error) => {
                    linux_portal::close_session(&session).await;
                    return Err(error);
                }
            };
            let sv = streams.streams();
            let stream = sv
                .first()
                .ok_or_else(|| cap_err("portal granted no streams", "empty stream list"))?;
            let node = stream.pipe_wire_node_id();
            if let Some(preview) = active_preview.as_ref() {
                // The portal grants the selected pixels, including Cut whenever
                // Cut is visible there. It does not prove controller exclusion.
                preview.set_controller_safety(
                    RecursionStatus::Possible,
                    ControllerExclusionStatus::NotConfirmed,
                );
                preview.enable();
            }
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

            // The portal node ID is scoped to this session's PipeWire remote.
            // Both consumers need it: pipewire-rs consumes it directly, while
            // GStreamer receives an explicitly inherited duplicate for
            // `pipewiresrc fd=…`. A default PipeWire connection cannot access a
            // portal-granted node on GNOME.
            let mut pw_fd = match linux_portal::await_pre_first_frame(
                "open PipeWire stream",
                stop_async.as_ref(),
                portal_deadline,
                async {
                    proxy
                        .open_pipe_wire_remote(&session, Default::default())
                        .await
                        .map_err(|e| cap_err("open pipewire remote", e))
                },
            )
            .await
            {
                Ok(fd) => Some(fd),
                Err(error) => {
                    linux_portal::close_session(&session).await;
                    return Err(error);
                }
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
            let mut input = Some(crate::linux_input::start(
                input_mode.use_evdev,
                start,
                stop.clone(),
                capture_keys,
                sw,
                sh,
            )?);

            let mic_handle = if audio_wanted {
                let ready = Arc::new(AtomicBool::new(false));
                let mic_path = format!("{out_dir_async}/mic.wav");
                Some(match cfg.microphone_level.clone() {
                    Some(level) => crate::mic_endpoint::spawn_microphone_capture_with_level(
                        mic_path,
                        cfg.microphone_source.clone(),
                        stop.clone(),
                        ready,
                        start,
                        level,
                    ),
                    None => crate::mic_endpoint::spawn_microphone_capture(
                        mic_path,
                        cfg.microphone_source.clone(),
                        stop.clone(),
                        ready,
                        start,
                    ),
                })
            } else {
                None
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
                let segment_preview = active_preview.as_ref().and_then(|preview| {
                    preview
                        .begin_segment()
                        .map(|generation| (preview.clone(), generation))
                });
                let (capture_start_ms, ended_ms) = if wayland_pw {
                    let fd = pw_fd
                        .take()
                        .expect("every capture segment owns one portal PipeWire remote");
                    let ff = ff_for_async.clone();
                    let start_c = start;
                    let stop_c = stop.clone();
                    let readiness = cfg.readiness.clone();
                    let cur = tokio::task::spawn_blocking(move || {
                        crate::wayland_pw::capture(crate::wayland_pw::PipewireCaptureRequest {
                            pw_fd: Some(fd),
                            node,
                            dur_ms: interval_end,
                            start: start_c,
                            stop: stop_c,
                            raw_path: segment_path,
                            ff_bin: ff,
                            readiness,
                            active_preview: segment_preview,
                        })
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
                    let fd = pw_fd
                        .take()
                        .expect("every capture segment owns one portal PipeWire remote");
                    crate::linux_gst_capture::capture_segment(
                        fd,
                        node,
                        &segment_path,
                        interval_end,
                        start,
                        stop.clone(),
                        cfg.readiness.clone(),
                        segment_preview,
                    )
                    .await?
                };
                if let Some(preview) = active_preview.as_ref() {
                    preview.clear_current_generation();
                }
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
                // A PipeWire remote belongs to one encoder/consumer connection.
                // Reopen it for every checkpoint regardless of consumer, so a
                // terminated GStreamer segment cannot leave a stale fd in the
                // next one. Stop takes precedence over an unnecessary reopen.
                pw_fd = Some(
                    proxy
                        .open_pipe_wire_remote(&session, Default::default())
                        .await
                        .map_err(|e| cap_err("reopen pipewire remote", e))?,
                );
                // The next encoder does not exist until any portal-remote reopen and
                // prior segment verification finish. Measure its new wall-clock start
                // so stitching materializes all of that restart gap.
                segment_start_ms = u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX);
            };
            // End external mic/system-audio/input on the same capture clock before
            // any CPU-heavy stitch. The stitched timeline already includes measured
            // restart gaps, so post-capture publication time must not extend sidecars.
            stop.store(true, Ordering::Relaxed);
            // X11's measured video boundary is final. Seal its RECORD listener
            // before stitch/audio finalization; retain the Wayland evdev ordering.
            let sealed_x11_input = if input_mode.use_evdev {
                None
            } else {
                Some(
                    input
                        .take()
                        .expect("X11 input owner remains live until video stops")
                        .finish(duration_ms)?,
                )
            };
            if let Some(owner) = checkpoints.as_ref() {
                owner.stitch(&ff_for_async, &ffprobe_bin(), "raw.mp4")?;
            }
            let (audio, microphone_outcome) =
                crate::microphone_result::finish(audio_wanted, mic_handle);
            let (cursor, mut clicks, scrolls, keys) = match sealed_x11_input {
                Some(snapshot) => snapshot,
                None => input
                    .take()
                    .expect("Wayland evdev owner remains live through audio finalization")
                    .finish(duration_ms)?,
            };
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
            linux_portal::close_session(&session).await;
            // session drops here — capture is done.
            Ok(CapPhase {
                w: sw,
                h: sh,
                duration_ms,
                audio,
                microphone_outcome,
                input,
                keys,
            })
        });
        if let Some(preview) = cfg.active_preview.as_ref() {
            preview.clear_current_generation();
        }
        let phase = phase?;

        // CFR normalize: sparse raw → constant fps + EXACT wall-clock duration.
        // fps=<fps> makes it CFR; tpad clones the last frame across pauses; -t cuts
        // to the true captured duration. (Proven on Linux: static raw → exact 6.000s.)
        // `raw.mp4` was already checkpoint-verified, but this is the user-facing
        // normal source after CFR normalization. Decode it again before the
        // RecordingProject can name it; a successful encoder exit alone is not
        // proof that its final container has the gap-padded clock we promised.
        let normalized = crate::linux_source_publication::normalize_and_publish(
            Path::new(&raw),
            Path::new(&path),
            phase.duration_ms,
            fps,
            &ffmpeg_bin(),
            &ffprobe_bin(),
            cfg.quality.as_ref(),
        )
        .map_err(|error| cap_err("normalize and publish source", error))?;
        let verified_media = normalized.media;
        let capture_quality = normalized.quality;
        let _ = std::fs::remove_file(&raw); // drop the throwaway sparse capture

        // Reuse final verifier dimensions; a fallback has no proof that portal
        // logical size equals encoded video.
        let finalized_dimensions = verified_media.width.zip(verified_media.height);
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
            camera_artifact: None,
            webcam_video: None,
            audio: phase.audio,
            microphone_outcome: phase.microphone_outcome,
            settings: Settings {
                width: w,
                height: h,
                fps: fps as f32,
                audio_rate: 48_000,
            },
            capture_quality,
            verified_media: Some(verified_media),
        })
    }
}
