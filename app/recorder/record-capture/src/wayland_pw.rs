//! wayland_pw.rs — unified Wayland capture via pipewire-rs: one PipeWire stream
//! delivers both the video frames AND the absolute cursor position metadata.
//!
//! WHY this exists (and isn't gst): GStreamer's `pipewiresrc` cannot expose the
//! ScreenCast cursor metadata (`SPA_META_Cursor`), so the gst path can only get the
//! cursor from evdev (relative → approximate). pipewire-rs reads the buffer's
//! `SPA_META_Cursor` directly → pixel-accurate absolute cursor on Wayland. It also
//! reads the frame pixels (mutter hands MemFd buffers — CPU-readable — because we do
//! NOT advertise DMA-BUF modifiers in the format) and pipes them to ffmpeg, so the
//! same stream produces the source video. X11 keeps the gst+rdevin path (perfect
//! cursor already); this module is the Wayland path.
//!
//! Mutter offers the cursor meta at a fixed
//! size `CURSOR_META_SIZE(384,384)` (see mutter `on_stream_param_changed`). The
//! consumer's requested `SPA_PARAM_META_size` MUST intersect that, or PipeWire
//! silently drops the cursor meta. We request a CHOICE_RANGE whose max spans 384×384,
//! which preserves pixel-accurate cursor coordinates across a 4K capture surface.
//!
//! Clean-room / MIT: the cursor struct + buffer walk use the MIT libspa-rs `MetaCursor`
//! wrapper, without incorporating GPL recorder source.

use std::cell::RefCell;
use std::os::fd::OwnedFd;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use pipewire as pw;
use pw::spa::param::video::{VideoFormat, VideoInfoRaw};
use pw::spa::param::ParamType;
use pw::spa::pod::serialize::PodSerializer;
use pw::spa::pod::{ChoiceValue, Object, Pod, Property, PropertyFlags, Value};
use pw::spa::utils::{Choice, ChoiceEnum, ChoiceFlags, Id, SpaTypes};
use pw::{properties::properties, spa};

use crate::cursor_correlation::{CursorMetadataSample, PipewireCursorCapture};

/// Serialize an `SPA_PARAM_Meta` object requesting `SPA_META_Cursor` with the size as a
/// CHOICE_RANGE whose max (512×512) spans mutter's fixed `CURSOR_META_SIZE(384,384)`.
/// A fixed or too-small request never intersects mutter's offer and the cursor meta is
/// silently dropped — this range is the fix.
fn cursor_meta_param_bytes() -> Vec<u8> {
    let cur = i32::try_from(std::mem::size_of::<spa::sys::spa_meta_cursor>())
        .expect("cursor metadata size fits i32");
    let bmp = i32::try_from(std::mem::size_of::<spa::sys::spa_meta_bitmap>())
        .expect("bitmap metadata size fits i32");
    let sz = |w: i32, h: i32| {
        w.checked_mul(h)
            .and_then(|pixels| pixels.checked_mul(4))
            .and_then(|bytes| bytes.checked_add(cur))
            .and_then(|bytes| bytes.checked_add(bmp))
            .expect("fixed cursor metadata bounds fit i32")
    };
    let obj = Object {
        type_: SpaTypes::ObjectParamMeta.as_raw(),
        id: ParamType::Meta.as_raw(),
        properties: vec![
            Property {
                key: spa::sys::SPA_PARAM_META_type,
                flags: PropertyFlags::empty(),
                value: Value::Id(Id(spa::sys::SPA_META_Cursor)),
            },
            Property {
                key: spa::sys::SPA_PARAM_META_size,
                flags: PropertyFlags::empty(),
                value: Value::Choice(ChoiceValue::Int(Choice(
                    ChoiceFlags::empty(),
                    ChoiceEnum::Range {
                        default: sz(384, 384),
                        min: sz(1, 1),
                        max: sz(512, 512),
                    },
                ))),
            },
        ],
    };
    PodSerializer::serialize(std::io::Cursor::new(Vec::new()), &Value::Object(obj))
        .expect("serialize cursor meta param")
        .0
        .into_inner()
}

/// ffmpeg input pixel-format string for a negotiated SPA video format. We advertise
/// BGRx/RGBx/RGB/BGRA/RGBA (no DMA-BUF modifiers → MemFd buffers).
fn ff_pix_fmt(fmt: VideoFormat) -> &'static str {
    match fmt {
        VideoFormat::BGRx => "bgr0",
        VideoFormat::BGRA => "bgra",
        VideoFormat::RGBx => "rgb0",
        VideoFormat::RGBA => "rgba",
        VideoFormat::RGB => "rgb24",
        VideoFormat::BGR => "bgr24",
        _ => "bgr0",
    }
}

/// Bytes per pixel for a negotiated SPA video format — needed to de-pad scanlines
/// (the buffer stride often exceeds width times bytes-per-pixel due to alignment).
/// Mirrors `ff_pix_fmt`: alpha/padding 32-bit formats are 4 bytes; RGB/BGR are 3.
fn bytes_per_pixel(fmt: VideoFormat) -> usize {
    match fmt {
        VideoFormat::RGB | VideoFormat::BGR => 3,
        // BGRx/BGRA/RGBx/RGBA and the bgr0 fallback are all 32-bit.
        _ => 4,
    }
}

/// Write a frame to `out` as packed rows, de-padding
/// from a source `data` block (the full mmap'd region) whose scanlines may be padded:
/// PipeWire/mutter align rows, so stride may exceed the packed row size and offset may be nonzero.
/// ffmpeg `-f rawvideo` assumes packed rows, so copying the raw block shears the image.
///
/// Returns false (writing nothing) if the geometry doesn't fit in `data` — the caller
/// drops the frame rather than reading out of bounds. `stride == 0` means "packed".
/// Pure + bounds-checked so it can be unit-tested without a live PipeWire stream.
fn write_depadded<W: std::io::Write>(
    out: &mut W,
    data: &[u8],
    width: usize,
    height: usize,
    bpp: usize,
    stride: usize,
    offset: usize,
) -> bool {
    let row_bytes = match width.checked_mul(bpp) {
        Some(v) if v > 0 => v,
        _ => return false,
    };
    if height == 0 {
        return false;
    }
    let eff_stride = if stride == 0 { row_bytes } else { stride };
    if eff_stride < row_bytes {
        return false;
    }
    // The final row end must fit in data. Every step stays checked so malformed
    // producer geometry cannot wrap before the range validation.
    let span = match (height - 1)
        .checked_mul(eff_stride)
        .and_then(|v| v.checked_add(row_bytes))
        .and_then(|v| v.checked_add(offset))
    {
        Some(v) => v,
        None => return false,
    };
    if span > data.len() {
        return false;
    }
    if eff_stride == row_bytes && offset == 0 {
        if out.write_all(&data[..span]).is_err() {
            return false;
        }
    } else {
        for row in 0..height {
            let Some(start) = row
                .checked_mul(eff_stride)
                .and_then(|value| value.checked_add(offset))
            else {
                return false;
            };
            let Some(end) = start.checked_add(row_bytes) else {
                return false;
            };
            let Some(bytes) = data.get(start..end) else {
                return false;
            };
            if out.write_all(bytes).is_err() {
                return false;
            }
        }
    }
    true
}

fn valid_chunk_data(data: &[u8], offset: usize, size: usize) -> Option<&[u8]> {
    if size == 0 {
        return None;
    }
    let end = offset.checked_add(size)?;
    data.get(..end)
}

/// Shared state across the pipewire `param_changed` (sets format, spawns ffmpeg) and
/// `process` (writes frames, collects cursor) callbacks.
struct State {
    start: Instant,
    raw_path: String,
    ff_bin: String,
    width: u32,
    height: u32,
    /// Bytes per pixel of the negotiated format (de-pad scanlines by stride).
    bpp: usize,
    ff_stdin: Option<ChildStdin>,
    ff_child: Option<Child>,
    cursor_metadata: Vec<CursorMetadataSample>,
    /// Buffers dequeued (incl. cursor-only updates) — debug only.
    frames: u64,
    /// REAL pixel frames actually written to ffmpeg — the "did we capture video?"
    /// signal (a cursor-only buffer writes 0 bytes and must NOT count).
    pixel_frames: u64,
    /// Shared-capture-clock instant of the first frame accepted by the encoder.
    capture_start_ms: Option<u64>,
    spawn_failed: bool,
}

/// Spawn ffmpeg to encode raw frames piped on stdin → `raw_path` (an .mp4). We use
/// `-use_wallclock_as_timestamps` so frames are stamped by arrival time (mutter is
/// damage-driven / variable-rate); the caller's CFR-normalize pass then produces a
/// constant-fps, exact-duration source video, exactly as for the gst path.
fn spawn_ffmpeg(st: &mut State, pix_fmt: &str) {
    let size = format!("{}x{}", st.width, st.height);
    match Command::new(&st.ff_bin)
        .args([
            "-v",
            "error",
            "-y",
            "-f",
            "rawvideo",
            "-pix_fmt",
            pix_fmt,
            "-s",
            &size,
            "-use_wallclock_as_timestamps",
            "1",
            "-i",
            "-",
            "-c:v",
            "libx264",
            "-preset",
            "ultrafast",
            "-tune",
            "zerolatency",
            "-pix_fmt",
            "yuv420p",
            &st.raw_path,
        ])
        .stdin(Stdio::piped())
        .spawn()
    {
        Ok(mut child) => {
            st.ff_stdin = child.stdin.take();
            st.ff_child = Some(child);
        }
        Err(e) => {
            st.spawn_failed = true;
            eprintln!("wayland_pw: failed to spawn ffmpeg: {e}");
        }
    }
}

/// Capture `dur_ms` of the granted PipeWire `node` over the portal remote `pw_fd`:
/// encode frames to `raw_path` (.mp4, variable PTS) and return absolute compositor-space
/// cursor metadata plus the negotiated physical frame size. Both metadata callbacks and
/// evdev clicks stamp the shared `start` clock. Stops early if `stop` is set.
///
pub fn capture(
    pw_fd: Option<OwnedFd>,
    node: u32,
    dur_ms: u64,
    start: Instant,
    stop: Arc<AtomicBool>,
    raw_path: &str,
    ff_bin: &str,
) -> Result<PipewireCursorCapture, String> {
    // NOTE: PipeWire callbacks are extern "C", so a panic inside one aborts the process
    // (non-unwinding) rather than propagating — the default panic hook still prints the
    // message+location to stderr before the abort, which is enough to diagnose. (An
    // earlier debug build also wrote the panic to a fixed /tmp path; removed — that's a
    // symlink-attack vector, and a library shouldn't replace the global panic hook.)
    pw::init();
    let mainloop = pw::main_loop::MainLoopRc::new(None).map_err(|e| format!("pw mainloop: {e}"))?;
    let context =
        pw::context::ContextRc::new(&mainloop, None).map_err(|e| format!("pw context: {e}"))?;
    // Portal path: connect over the restricted remote fd. A direct-session caller
    // (pw_fd = None): connect to the default user PipeWire and use the node id.
    let core = match pw_fd {
        Some(fd) => context
            .connect_fd_rc(fd, None)
            .map_err(|e| format!("pw connect_fd: {e}"))?,
        None => context
            .connect_rc(None)
            .map_err(|e| format!("pw connect: {e}"))?,
    };

    let state = Rc::new(RefCell::new(State {
        start,
        raw_path: raw_path.to_string(),
        ff_bin: ff_bin.to_string(),
        width: 0,
        height: 0,
        bpp: 4,
        ff_stdin: None,
        ff_child: None,
        cursor_metadata: Vec::new(),
        frames: 0,
        pixel_frames: 0,
        capture_start_ms: None,
        spawn_failed: false,
    }));

    let stream = pw::stream::StreamBox::new(
        &core,
        "shellx-record",
        properties! {
            *pw::keys::MEDIA_TYPE => "Video",
            *pw::keys::MEDIA_CATEGORY => "Capture",
            *pw::keys::MEDIA_ROLE => "Screen",
        },
    )
    .map_err(|e| format!("pw stream: {e}"))?;

    let s_param = state.clone();
    let s_proc = state.clone();
    let _listener = stream
        .add_local_listener_with_user_data(())
        .param_changed(move |stream, _, id, param| {
            let Some(param) = param else { return };
            if id != ParamType::Format.as_raw() {
                return;
            }
            let mut info = VideoInfoRaw::default();
            if info.parse(param).is_err() {
                return;
            }
            // Set format + spawn ffmpeg, then RELEASE the borrow BEFORE update_params:
            // update_params can synchronously re-enter the callbacks, and a second
            // borrow_mut while this one is held would panic (→ abort in the C trampoline).
            {
                let mut st = s_param.borrow_mut();
                if st.ff_stdin.is_some() || st.spawn_failed {
                    return; // already negotiated
                }
                st.width = info.size().width;
                st.height = info.size().height;
                st.bpp = bytes_per_pixel(info.format());
                let pf = ff_pix_fmt(info.format());
                spawn_ffmpeg(&mut st, pf);
            }
            // Declare the cursor meta so mutter attaches SPA_META_Cursor to buffers.
            let bytes = cursor_meta_param_bytes();
            if let Some(meta_pod) = Pod::from_bytes(&bytes) {
                if let Err(e) = stream.update_params(&mut [meta_pod]) {
                    eprintln!("wayland_pw: update_params(cursor meta) failed: {e:?}");
                }
            }
        })
        .process(move |stream, _| {
            // The safe Buffer guard ties the dequeued buffer to this stream and
            // automatically requeues it on every return path.
            let Some(mut buffer) = stream.dequeue_buffer() else {
                return;
            };
            // PipeWire can drive process() re-entrantly (e.g. from inside the
            // update_params() call in param_changed). Skip rather than panicking if
            // that callback still owns the shared RefCell borrow.
            let Ok(mut st) = s_proc.try_borrow_mut() else {
                return;
            };
            st.frames = st.frames.saturating_add(1);

            // Frame pixels to ffmpeg. A zero-size chunk is a cursor-only update,
            // and offset + size defines the producer-initialized part of the map.
            {
                let (w, h, bpp) = (
                    usize::try_from(st.width).ok(),
                    usize::try_from(st.height).ok(),
                    st.bpp,
                );
                if let (Some(w), Some(h), Some(data)) = (w, h, buffer.datas_mut().first_mut()) {
                    if !data.as_raw().chunk.is_null() {
                        let chunk = data.chunk();
                        let size = usize::try_from(chunk.size()).ok();
                        let offset = usize::try_from(chunk.offset()).ok();
                        let stride = usize::try_from(chunk.stride().max(0)).ok();
                        if let (Some(size), Some(offset), Some(stride), Some(mapped)) =
                            (size, offset, stride, data.data())
                        {
                            let wrote = valid_chunk_data(mapped, offset, size)
                                .and_then(|valid| {
                                    st.ff_stdin.as_mut().map(|stdin| {
                                        write_depadded(stdin, valid, w, h, bpp, stride, offset)
                                    })
                                })
                                .unwrap_or(false);
                            if wrote {
                                st.pixel_frames = st.pixel_frames.saturating_add(1);
                                if st.capture_start_ms.is_none() {
                                    st.capture_start_ms = Some(
                                        u64::try_from(st.start.elapsed().as_millis())
                                            .unwrap_or(u64::MAX),
                                    );
                                }
                            }
                        }
                    }
                }
            }

            // Typed metadata lookup validates the SPA meta type and minimum struct
            // size before exposing the cursor wrapper.
            if let Some(cursor) = buffer.find_meta::<spa::buffer::meta::MetaCursor>() {
                if cursor.id() != 0 {
                    let position = cursor.position();
                    let t_ms = u64::try_from(st.start.elapsed().as_millis()).unwrap_or(u64::MAX);
                    // Keep stationary samples too. A click after the cursor stops is
                    // still exact only when a recent metadata buffer confirms it.
                    st.cursor_metadata.push(CursorMetadataSample {
                        t_ms,
                        x: f64::from(position.x),
                        y: f64::from(position.y),
                    });
                }
            }
        })
        .register()
        .map_err(|e| format!("pw register: {e}"))?;

    // video/raw format — NO DMA-BUF modifiers, so mutter hands MemFd (CPU-readable).
    let obj = pw::spa::pod::object!(
        pw::spa::utils::SpaTypes::ObjectParamFormat,
        pw::spa::param::ParamType::EnumFormat,
        pw::spa::pod::property!(
            pw::spa::param::format::FormatProperties::MediaType,
            Id,
            pw::spa::param::format::MediaType::Video
        ),
        pw::spa::pod::property!(
            pw::spa::param::format::FormatProperties::MediaSubtype,
            Id,
            pw::spa::param::format::MediaSubtype::Raw
        ),
        pw::spa::pod::property!(
            pw::spa::param::format::FormatProperties::VideoFormat,
            Choice,
            Enum,
            Id,
            pw::spa::param::video::VideoFormat::BGRx,
            pw::spa::param::video::VideoFormat::RGBx,
            pw::spa::param::video::VideoFormat::BGRA,
            pw::spa::param::video::VideoFormat::RGBA,
            pw::spa::param::video::VideoFormat::RGB,
        ),
        pw::spa::pod::property!(
            pw::spa::param::format::FormatProperties::VideoSize,
            Choice,
            Range,
            Rectangle,
            pw::spa::utils::Rectangle {
                width: 1920,
                height: 1080
            },
            pw::spa::utils::Rectangle {
                width: 1,
                height: 1
            },
            pw::spa::utils::Rectangle {
                width: 8192,
                height: 8192
            }
        ),
        pw::spa::pod::property!(
            pw::spa::param::format::FormatProperties::VideoFramerate,
            Choice,
            Range,
            Fraction,
            pw::spa::utils::Fraction { num: 30, denom: 1 },
            pw::spa::utils::Fraction { num: 0, denom: 1 },
            pw::spa::utils::Fraction { num: 120, denom: 1 }
        ),
    );
    let values: Vec<u8> = PodSerializer::serialize(
        std::io::Cursor::new(Vec::new()),
        &pw::spa::pod::Value::Object(obj),
    )
    .map_err(|e| format!("serialize format: {e}"))?
    .0
    .into_inner();
    let mut params = [Pod::from_bytes(&values).ok_or_else(|| "build format pod".to_string())?];

    stream
        .connect(
            spa::utils::Direction::Input,
            Some(node),
            pw::stream::StreamFlags::AUTOCONNECT | pw::stream::StreamFlags::MAP_BUFFERS,
            &mut params,
        )
        .map_err(|e| format!("pw connect: {e}"))?;

    // Quit when the duration elapses OR the caller signals stop. Poll on a timer.
    let deadline = start.checked_add(Duration::from_millis(dur_ms));
    let weak = mainloop.downgrade();
    let _timer = mainloop.loop_().add_timer(move |_| {
        if stop.load(Ordering::Relaxed)
            || deadline.is_some_and(|deadline| Instant::now() >= deadline)
        {
            if let Some(ml) = weak.upgrade() {
                ml.quit();
            }
        }
    });
    let _ = _timer.update_timer(
        Some(Duration::from_millis(100)),
        Some(Duration::from_millis(100)),
    );

    mainloop.run();
    let capture_end_ms = u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX);

    // Finalize: close ffmpeg's stdin (EOF) and wait so the mp4 is flushed/moov-written.
    let (stdin, child, cursor_metadata, pixel_frames, frame_width, frame_height, capture_start_ms) = {
        let mut st = state.borrow_mut();
        (
            st.ff_stdin.take(),
            st.ff_child.take(),
            std::mem::take(&mut st.cursor_metadata),
            st.pixel_frames,
            st.width,
            st.height,
            st.capture_start_ms,
        )
    };
    drop(stdin); // EOF to ffmpeg
    if let Some(mut child) = child {
        let _ = child.wait();
    }
    // Gate on REAL pixel frames written, not dequeued buffers: a stream that only
    // delivered cursor-only buffers would pass a `frames`-based guard with an empty
    // mp4 (the oversized-buffer regression).
    if pixel_frames == 0 {
        return Err("no video frames captured from PipeWire node".to_string());
    }
    let capture_start_ms = capture_start_ms
        .ok_or_else(|| "no encoder-start timestamp from PipeWire frames".to_string())?;
    Ok(PipewireCursorCapture {
        metadata: cursor_metadata,
        frame_width,
        frame_height,
        capture_start_ms,
        capture_end_ms,
    })
}

#[cfg(test)]
mod tests {
    use super::{valid_chunk_data, write_depadded};

    // 2x2 BGRx packed frame: pass-through unchanged.
    #[test]
    fn packed_passthrough() {
        let (w, h, bpp) = (2usize, 2usize, 4usize);
        let row = w.checked_mul(bpp).unwrap();
        let len = row.checked_mul(h).unwrap();
        let data: Vec<u8> = (1..=u8::try_from(len).unwrap()).collect();
        let mut out = Vec::new();
        assert!(write_depadded(&mut out, &data, w, h, bpp, row, 0));
        assert_eq!(out, data);
    }

    // stride == 0 is treated as packed (some producers leave it unset).
    #[test]
    fn zero_stride_is_packed() {
        let (w, h, bpp) = (2usize, 2usize, 4usize);
        let len = w
            .checked_mul(bpp)
            .and_then(|value| value.checked_mul(h))
            .unwrap();
        let data: Vec<u8> = (1..=u8::try_from(len).unwrap()).collect();
        let mut out = Vec::new();
        assert!(write_depadded(&mut out, &data, w, h, bpp, 0, 0));
        assert_eq!(out, data);
    }

    // Padded: stride 12 > row_bytes 8 (4 bytes of pad per row) → rows de-padded.
    #[test]
    fn padded_depad() {
        let (w, h, bpp) = (2usize, 2usize, 4usize);
        let row = w.checked_mul(bpp).unwrap();
        let stride = 12usize;
        let mut data = vec![0u8; stride.checked_mul(h).unwrap()];
        for r in 0..h {
            for b in 0..row {
                let dst = r
                    .checked_mul(stride)
                    .and_then(|v| v.checked_add(b))
                    .unwrap();
                let value = r
                    .checked_mul(row)
                    .and_then(|v| v.checked_add(b))
                    .and_then(|v| v.checked_add(1))
                    .and_then(|v| u8::try_from(v).ok())
                    .unwrap();
                data[dst] = value;
            }
        }
        let mut out = Vec::new();
        assert!(write_depadded(&mut out, &data, w, h, bpp, stride, 0));
        // pad bytes (indices 8..12 of each row) must be dropped.
        assert_eq!(out, (1..=16).collect::<Vec<u8>>());
    }

    // Non-zero offset (data starts partway into the block).
    #[test]
    fn offset_depad() {
        let (w, h, bpp) = (2usize, 1usize, 4usize);
        let row = w.checked_mul(bpp).unwrap();
        let stride = 8usize;
        let offset = 4usize;
        let len = stride
            .checked_mul(h)
            .and_then(|value| value.checked_add(offset))
            .unwrap();
        let mut data = vec![0u8; len];
        for b in 0..row {
            data[offset + b] = (b + 1) as u8;
        }
        let mut out = Vec::new();
        assert!(write_depadded(&mut out, &data, w, h, bpp, stride, offset));
        assert_eq!(out, (1..=8).collect::<Vec<u8>>());
    }

    // Geometry doesn't fit in `data` → drop the frame (no panic, no OOB, no output).
    #[test]
    fn out_of_bounds_drops() {
        let (w, h, bpp) = (4usize, 4usize, 4usize); // needs 64 packed bytes
        let data = vec![0u8; 10]; // far too small
        let mut out = Vec::new();
        assert!(!write_depadded(&mut out, &data, w, h, bpp, 0, 0));
        assert!(out.is_empty());
    }

    // stride < row_bytes is invalid → reject (don't shear/overread).
    #[test]
    fn stride_too_small_drops() {
        let (w, h, bpp) = (4usize, 2usize, 4usize); // row_bytes 16
        let data = vec![0u8; 64];
        let mut out = Vec::new();
        assert!(!write_depadded(&mut out, &data, w, h, bpp, 8, 0));
        assert!(out.is_empty());
    }

    #[test]
    fn chunk_bounds_reject_empty_overflowing_or_truncated_ranges() {
        let data = [0u8; 16];
        assert!(valid_chunk_data(&data, 0, 0).is_none());
        assert!(valid_chunk_data(&data, usize::MAX, 2).is_none());
        assert!(valid_chunk_data(&data, 12, 8).is_none());
        assert_eq!(valid_chunk_data(&data, 4, 8).unwrap().len(), 12);
    }

    struct FailingWriter;

    impl std::io::Write for FailingWriter {
        fn write(&mut self, _buf: &[u8]) -> std::io::Result<usize> {
            Err(std::io::Error::other("closed"))
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn failed_encoder_write_does_not_count_as_a_frame() {
        let mut writer = FailingWriter;
        assert!(!write_depadded(&mut writer, &[0u8; 16], 2, 2, 4, 0, 0));
    }
}
