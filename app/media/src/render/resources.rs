//! Render memory: the estimate the render plan reports before a render starts
//! and the thread counts that fit it into this computer (items 82, 164, 165).
//!
//! Parts of the peak, each calibrated by measurement (peak RSS of the one
//! ffmpeg, `/usr/bin/time -l` on macOS with ffmpeg 9, on Linux with
//! ffmpeg 8; tables in the item-82 and item-164/165 records):
//! - encoder: libx264 keeps the slow preset's lookahead (about 180 bytes per
//!   output pixel) plus a reference set per frame thread (about 11 bytes per
//!   pixel per thread); the thread count is chosen in [`threads`].
//! - decoder: one per distinct source with a visible video segment (they are
//!   all open at once), about 8 + 3.4 x threads bytes per source pixel.
//!   Sources above DCI 4K decode with 8 threads (ffmpeg's automatic count is
//!   cores + 1, at most 16): on the 36-core Studio that saves 0.6 to 1.1 GB and
//!   costs no measurable time, because decoding runs 150 to 280 fps against an
//!   8K encode's 6 to 45 (8K60 HEVC 12 s: 16.4 s against 15.5 s; Artemis AV1
//!   with 12 encoder threads: 329.8 s against 338.8 s).
//! - held frames: source frames decoded early for a segment that plays later
//!   ([`held`]): shared decoder buffers when the chain passes the frame
//!   through, per-segment copies otherwise.
//! - overlays: each overlay track's full-frame alpha stream.
//! A segmented render (long timelines, overlays longer than one window) runs
//! one ffmpeg per window, so its peak is the largest window's, times the
//! windows that run at once.

mod held;
pub mod threads;

use cut_core::{Edl, Project, TrackKind};

pub use threads::{x264_auto_threads, ThreadChoice};

/// The encoder named by `-c:v` in a preset's video args.
pub fn video_encoder(video_args: &[String]) -> Option<&str> {
    video_args
        .windows(2)
        .find(|w| w[0] == "-c:v")
        .map(|w| w[1].as_str())
}

/// Frames above this many pixels (larger than DCI 4K) decode with fewer threads.
const CAP_ABOVE_PIXELS: u64 = 4096 * 2160;
/// Decoder threads for sources above DCI 4K (8K HEVC/AV1).
const DECODE_THREADS_BIG: u32 = 8;

/// Decoder threads for a `w`x`h` source, or None for ffmpeg's automatic count.
pub fn decoder_threads(w: u32, h: u32) -> Option<u32> {
    (u64::from(w) * u64::from(h) > CAP_ABOVE_PIXELS).then_some(DECODE_THREADS_BIG)
}

fn pinned_threads(video_args: &[String]) -> Option<u32> {
    video_args
        .windows(2)
        .find(|a| a[0] == "-threads")
        .and_then(|a| a[1].parse::<u32>().ok())
}

/// libx264 thread options for a render whose caller did not pin `-threads`
/// (export.range, bundles, reframes: render.final pins the preflight's own
/// choice). Automatic unless the encoder and one decoder at this size do not
/// fit the memory free now; then the ladder count that fits.
pub fn encoder_thread_args(video_args: &[String], w: u32, h: u32) -> Vec<String> {
    if video_encoder(video_args) != Some("libx264") || video_args.iter().any(|a| a == "-threads") {
        return Vec::new();
    }
    let cores = threads::logical_cores();
    let budget = threads::budget_mb(
        crate::host_memory::host_memory().map(|m| m.available_bytes / (1024 * 1024)),
        threads::memory_limit_mb(),
    );
    let px = u64::from(w) * u64::from(h);
    let choice = threads::choose(x264_auto_threads(cores), budget, |n| {
        mb(BASE_BYTES
            + px * encoder_bytes_per_pixel("libx264", u64::from(n))
            + px * decoder_bytes_per_pixel(decoder_threads(w, h), cores))
    });
    if !choice.reduced() {
        return Vec::new();
    }
    tracing::info!(
        threads = choice.chosen,
        automatic = choice.automatic,
        need_mb = choice.chosen_mb,
        "libx264 threads reduced to fit the memory free now"
    );
    vec!["-threads".into(), choice.chosen.to_string()]
}

/// Peak memory of one render, by part, in MB.
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize)]
pub struct MemoryEstimate {
    pub total_mb: u64,
    pub decode_mb: u64,
    pub encode_mb: u64,
    /// Source frames held for segments that come later on the timeline.
    pub held_frames_mb: u64,
    pub overlays_mb: u64,
    /// libx264 threads the render will use (None = not x264).
    pub encoder_threads: Option<u32>,
    /// Set for a segmented render: how many windows it renders; the parts are
    /// those of its largest window.
    pub windows: Option<usize>,
}

const BASE_BYTES: u64 = 150 * 1024 * 1024;
/// What one segmented window's ffmpeg costs beyond its parts (measured on the
/// Studio: about 200 MB at 4K beyond the 36-thread decoder; 400 keeps margin).
const WINDOW_OVERHEAD_MB: u64 = 400;

/// Bytes per output pixel an encoder keeps, beside its frame threads.
fn encoder_bytes_per_pixel(encoder: &str, threads: u64) -> u64 {
    match encoder {
        "libx264" => 180 + 11 * threads,
        // Measured on the 36-core Studio: 318 B/px at 4K, 253 at 8K.
        "libx265" => 360,
        "libsvtav1" => 300,
        "libvpx-vp9" => 150,
        "prores_ks" => 200,
        // VideoToolbox keeps its surfaces outside the ffmpeg process (measured
        // 48 B/px at 4K, 35 at 8K on the Studio).
        e if e.ends_with("_videotoolbox") => 60,
        // NVENC / QSV / AMF keep a few surfaces in system memory.
        e if e.contains('_') => 180,
        _ => 200,
    }
}

/// Bytes per source pixel of one decoder with `threads` (None: ffmpeg's
/// automatic count, logical cores + 1, at most 16).
fn decoder_bytes_per_pixel(threads: Option<u32>, cores: u64) -> u64 {
    let threads = threads.map(u64::from).unwrap_or((cores + 1).min(16));
    8 + 34 * threads / 10
}

fn mb(bytes: u64) -> u64 {
    bytes.div_ceil(1024 * 1024)
}

fn probe_u64(asset: &cut_core::Asset, key: &str) -> Option<u64> {
    asset.probe.as_ref()?.get(key)?.as_u64()
}

/// Bytes of one decoded frame: 8-bit 4:2:0 is 1.5 bytes per pixel, deeper or
/// 4:4:4 formats about twice that.
fn frame_bytes(asset: &cut_core::Asset) -> u64 {
    let w = probe_u64(asset, "width").unwrap_or(1920);
    let h = probe_u64(asset, "height").unwrap_or(1080);
    let deep = held::source_pix_fmt(asset)
        .is_some_and(|f| f.contains("10") || f.contains("12") || f.contains("444"));
    // Saturating: the sides come from a probe; an impossible size reads as
    // too big, never wraps to a small number.
    let px3 = w.saturating_mul(h).saturating_mul(3);
    if deep {
        px3
    } else {
        px3 / 2
    }
}

/// Estimate the peak memory of rendering `edl` at `(w, h)` with `video_args`.
pub fn estimate_memory(
    project: &Project,
    edl: &Edl,
    w: u32,
    h: u32,
    video_args: &[String],
) -> MemoryEstimate {
    estimate_memory_for(project, edl, w, h, video_args, threads::logical_cores())
}

pub(crate) fn estimate_memory_for(
    project: &Project,
    edl: &Edl,
    w: u32,
    h: u32,
    video_args: &[String],
    cores: u64,
) -> MemoryEstimate {
    if !crate::render::should_segment(project, edl, w, h) {
        return one_pass(project, edl, w, h, video_args, cores, None);
    }
    let overlays = crate::render::overlay_track_count(project);
    let window_ms = crate::render::render_window_ms(w, h, overlays);
    let windows = crate::render::reverse_memory::split_windows(
        project,
        edl,
        crate::render::plan_windows(edl, window_ms, project.settings.fps),
        project.settings.fps,
    );
    let at_once = parallel_windows(window_ms, w, h, overlays, cores) as u64;
    // Each window's ffmpeg gets `-threads <cores / windows at once>` before its
    // first input (`render_command_governed`), so a source without its own
    // decoder count decodes with that many threads, and every window costs
    // more than the same cut in one pass: measured 1.84 GB against 1.06 GB for
    // a 4K cut with no seam (VideoToolbox, 36 threads against 16).
    let window_threads = (cores / at_once.max(1)).max(1) as u32;
    let mut worst = MemoryEstimate::default();
    for (w0, w1) in &windows {
        let e = one_pass(
            project,
            &edl.window(*w0, *w1),
            w,
            h,
            video_args,
            cores,
            Some(window_threads),
        );
        if e.total_mb > worst.total_mb {
            worst = e;
        }
    }
    worst.total_mb = (worst.total_mb + WINDOW_OVERHEAD_MB) * at_once;
    worst.windows = Some(windows.len());
    worst
}

/// Windows a segmented render runs at once: 1 unless opted in (mirrors
/// `plan_seg_resources`; its cgroup check is not repeated, which can only
/// make this larger).
fn parallel_windows(window_ms: u64, w: u32, h: u32, overlays: usize, cores: u64) -> usize {
    let cores = cores.max(1) as usize;
    if let Some(n) = std::env::var("SHELLX_CUT_RENDER_PARALLEL")
        .ok()
        .and_then(|s| s.trim().parse::<usize>().ok())
        .filter(|n| *n >= 1)
    {
        return n.min(cores);
    }
    let pct = std::env::var("SHELLX_CUT_RENDER_RAM_PCT")
        .ok()
        .and_then(|s| s.trim().parse::<f64>().ok())
        .filter(|n| (10.0..=90.0).contains(n));
    let ram_mb = crate::ffmpeg::total_ram_bytes() as f64 / (1024.0 * 1024.0);
    match pct {
        Some(pct) if cfg!(target_os = "linux") && ram_mb > 0.0 => {
            let per = crate::render::per_window_mem_mb(window_ms, w, h, overlays).max(256.0);
            ((ram_mb * pct / 100.0 / per).floor() as usize).clamp(1, cores.min(8))
        }
        _ => 1,
    }
}

fn one_pass(
    project: &Project,
    edl: &Edl,
    w: u32,
    h: u32,
    video_args: &[String],
    cores: u64,
    // The decoder count of a source without its own (a segmented window's).
    window_decoder_threads: Option<u32>,
) -> MemoryEstimate {
    let out_px = u64::from(w) * u64::from(h);
    let encoder = video_encoder(video_args).unwrap_or("libx264");
    let enc_threads = if encoder == "libx264" {
        match pinned_threads(video_args) {
            Some(n) if n > 0 => u64::from(n),
            _ => u64::from(x264_auto_threads(cores)),
        }
    } else {
        0
    };
    let encode = out_px * encoder_bytes_per_pixel(encoder, enc_threads);

    let fps = project.settings.fps.max(1.0);
    let mut decoders = Vec::new();
    let mut held = 0u64;
    for (id, uses) in held::uses_by_source(project, edl, w, h) {
        let Some(asset) = project.assets.get(id) else {
            continue;
        };
        let (sw, sh) = (
            probe_u64(asset, "width").unwrap_or(1920),
            probe_u64(asset, "height").unwrap_or(1080),
        );
        let threads = decoder_threads(sw as u32, sh as u32).or(window_decoder_threads);
        decoders.push(
            sw.saturating_mul(sh)
                .saturating_mul(decoder_bytes_per_pixel(threads, cores)),
        );
        let source_fps = asset
            .probe
            .as_ref()
            .and_then(|p| p.get("fps"))
            .and_then(|v| v.as_f64())
            .filter(|f| *f > 0.0)
            .unwrap_or(30.0);
        held = held.saturating_add(held::peak_held_bytes(
            &uses,
            &held::Frames {
                // Decoder buffers are padded too: the reversed-parts 4K case
                // held 1.6% more than the plain frame size.
                source_bytes: frame_bytes(asset).saturating_mul(105) / 100,
                // Pool buffers are padded and a few source frames wait
                // before conversion: measured 10% above the plain size.
                output_bytes: out_px * 3 / 2 * 115 / 100,
                source_fps,
                output_fps: fps,
            },
        ));
    }
    // On one video track the sources play one after another: only one decoder
    // runs at a time and a waiting one keeps 20 to 29% of its memory (frames
    // decoded ahead, measured with five different 4K and 8K clips). Overlay
    // tracks decode at the same time, so every decoder counts in full.
    decoders.sort_unstable();
    let decode = if video_track_count(project) <= 1 {
        let active = decoders.pop().unwrap_or(0);
        let waiting = decoders.iter().fold(0u64, |a, b| a.saturating_add(*b));
        active.saturating_add(waiting.saturating_mul(35) / 100)
    } else {
        decoders.iter().fold(0u64, |a, b| a.saturating_add(*b))
    };
    let overlays = video_track_count(project).saturating_sub(1) as u64;
    let overlay_bytes = overlays * out_px * 64;
    let total = [encode, decode, held, overlay_bytes]
        .into_iter()
        .fold(BASE_BYTES, u64::saturating_add);
    MemoryEstimate {
        total_mb: mb(total),
        decode_mb: mb(decode),
        encode_mb: mb(encode),
        held_frames_mb: mb(held),
        overlays_mb: mb(overlay_bytes),
        encoder_threads: (encoder == "libx264").then_some(enc_threads as u32),
        windows: None,
    }
}

fn video_track_count(project: &Project) -> usize {
    project
        .tracks
        .iter()
        .filter(|t| t.kind == TrackKind::Video && !t.clips.is_empty())
        .count()
}

/// Typical output size in MB: the bitrate when one is set, else bits per pixel
/// per frame seen in the HQ tests (CRF/CQ rate control) for the codec.
pub fn estimate_output_mb(
    duration_ms: u64,
    w: u32,
    h: u32,
    fps: f64,
    video_args: &[String],
    audio_kbps: u64,
) -> u64 {
    let secs = duration_ms as f64 / 1000.0;
    let video_bits_per_sec = match video_args.windows(2).find(|a| a[0] == "-b:v") {
        Some(a) if a[1] != "0" => crate::render::parse_bitrate_kbps(&a[1])
            .map(|k| f64::from(k) * 1000.0)
            .unwrap_or(0.0),
        _ => {
            let bpp = match video_encoder(video_args).unwrap_or("libx264") {
                "prores_ks" => 2.5,
                e if e.starts_with("hevc") || e.starts_with("libx265") => 0.08,
                e if e.starts_with("av1") || e.starts_with("libsvtav1") => 0.07,
                _ => 0.13,
            };
            bpp * f64::from(w) * f64::from(h) * fps.max(1.0)
        }
    };
    let bytes = (video_bits_per_sec + audio_kbps as f64 * 1000.0) * secs / 8.0;
    mb(bytes as u64).max(1)
}

#[cfg(test)]
mod tests;
