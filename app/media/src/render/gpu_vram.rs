use super::{planned_video_tracks, RenderOptions};
use cut_core::{Edl, Project, TrackKind};

// === GPU VRAM bound ============================================================
//
// GPU frames live in VRAM, which has NO cgroup backstop (the render_command cgroup
// caps system RAM only) — NVDEC/NVENC fail HARD on a VRAM OOM. The GPU graph is
// single-pass (not windowed), so the bound is a per-render PEAK estimate compared
// to the device's VRAM ([`crate::hwencode::cuda_total_vram_bytes`]); over budget →
// fall back to software (never wedge the GPU). The estimate is a deliberate
// OVER-estimate: a false "fits" must never OOM the GPU, while a false "too big"
// only forgoes the fast path.

/// Bytes per pixel for a VRAM frame estimate. yuva420p is 4 planes; NV12 is 1.5,
/// but 4 keeps generous headroom (driver overhead, alignment, intermediate
/// surfaces) so the estimate over-counts rather than under-counts.
const VRAM_BYTES_PER_PX: u64 = 4;
/// NVDEC decode-surface pool per hardware-decoded BASE input (DPB + extra
/// surfaces). 4K HEVC can need ~16–20; 24 over-estimates safely.
const VRAM_DECODE_POOL_FRAMES: u64 = 24;
/// In-flight CUDA surfaces an overlay track adds (its hwupload'd full-frame
/// yuva420p stream + the overlay_cuda working set).
const VRAM_OVERLAY_FRAMES: u64 = 4;
/// NVENC working set (DPB + lookahead), in OUTPUT frames.
const VRAM_NVENC_FRAMES: u64 = 16;

/// Estimated PEAK VRAM (bytes) the GPU graph holds for this timeline: NVDEC pools
/// for the distinct base-track inputs (sized at SOURCE geometry — the pool holds
/// source-resolution frames before scale_cuda conforms them) + one full
/// output-frame surface set per overlay track + the NVENC working set. Used by the
/// render gate to fall back to software before a VRAM OOM. Over-estimates by
/// design (see the module note above).
pub(super) fn gpu_vram_estimate_bytes(project: &Project, edl: &Edl, opts: &RenderOptions) -> u64 {
    let (ow, oh) = opts.output_geometry(project, edl);
    let out_frame = (ow as u64) * (oh as u64) * VRAM_BYTES_PER_PX;

    // Base canvas = the FIRST video track with clips (NVDEC-decoded; stays on GPU).
    let video_tracks = planned_video_tracks(project);
    let base_plan = video_tracks.first().copied();
    let base_id = base_plan.map(|p| p.id);

    // Decode pools: one per DISTINCT base-track NVDEC input, at source geometry.
    let mut decode = 0u64;
    if let Some(bid) = base_id.filter(|_| base_plan.is_some_and(|p| p.visible)) {
        let mut seen: std::collections::BTreeSet<&str> = std::collections::BTreeSet::new();
        for seg in edl
            .track_segments(bid)
            .filter(|s| s.track_kind == TrackKind::Video)
        {
            let Some(asset_id) = seg.asset.as_deref() else {
                continue;
            };
            if !seen.insert(asset_id) {
                continue;
            }
            let (sw, sh) = project
                .assets
                .get(asset_id)
                .and_then(|a| a.probe.as_ref())
                .and_then(|p| Some((p.get("width")?.as_u64()?, p.get("height")?.as_u64()?)))
                .filter(|(w, h)| *w > 0 && *h > 0)
                .unwrap_or((ow as u64, oh as u64));
            // Saturating: an impossible probed size reads as too big (software).
            decode = decode.saturating_add(
                VRAM_DECODE_POOL_FRAMES
                    .saturating_mul(sw)
                    .saturating_mul(sh)
                    .saturating_mul(VRAM_BYTES_PER_PX),
            );
        }
    }

    // Overlay tracks = every video track with clips BEYOND the base; each is a
    // CPU-built full-frame yuva420p stream hwupload'd into overlay_cuda.
    let overlay_tracks = video_tracks.len().saturating_sub(1) as u64;
    let overlays = overlay_tracks * VRAM_OVERLAY_FRAMES * out_frame;

    decode
        .saturating_add(overlays)
        .saturating_add(VRAM_NVENC_FRAMES * out_frame)
}

/// VRAM budget (bytes) for a GPU render. `SHELLX_CUT_GPU_VRAM_BUDGET_MB` overrides
/// (tests + a manual hard cap); else 85% of the probed device VRAM (headroom for
/// the driver/display + fragmentation); else a conservative 4 GiB when the device
/// size can't be read (no nvidia-smi) — so a small GPU is never assumed huge.
fn gpu_vram_budget_bytes() -> u64 {
    if let Some(mb) = std::env::var("SHELLX_CUT_GPU_VRAM_BUDGET_MB")
        .ok()
        .and_then(|s| s.trim().parse::<u64>().ok())
        .filter(|n| *n > 0)
    {
        return mb * 1024 * 1024;
    }
    match crate::hwencode::cuda_total_vram_bytes() {
        Some(total) => total * 85 / 100,
        None => 4 * 1024 * 1024 * 1024,
    }
}

/// True when the GPU graph's estimated peak VRAM fits the budget. The render gate
/// ANDs this in so a heavy timeline (many 4K overlay layers / a small GPU) falls
/// back to software instead of OOMing the GPU.
pub(super) fn gpu_vram_fits(project: &Project, edl: &Edl, opts: &RenderOptions) -> bool {
    gpu_vram_estimate_bytes(project, edl, opts) <= gpu_vram_budget_bytes()
}
