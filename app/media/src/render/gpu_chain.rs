//! Row 194 on the CUDA fast-track (`build_graph_gpu`): every segment shows the
//! same source frames at the same output frames as the software graph.
//!
//! The CUDA graph used to trim each clip by milliseconds and always convert
//! with `fps=`, with no frame lock: each clip came out as a whole number of its
//! own frames, so every cut moved the later picture late (the software path's
//! row 194 drift). It now takes the software graph's pieces: the same trim
//! (including the start of a clip that holds no source frame), the same
//! renumbering or `fps=` conversion, the speed-ramp lock and, before the
//! concat, the slot lock of [`super::frame_budget`]. Every one of these
//! filters only touches timestamps or clones frames, so they run on CUDA frames
//! (`tpad=stop_mode=clone` and `trim=end_frame` checked on NVDEC/hwupload
//! frames, FFmpeg N-127367).

use cut_core::{EdlSegment, Project};

/// The timing pieces of one segment chain, in the order the chain uses them.
pub(super) struct Timing {
    pub(super) trim_in: String,
    pub(super) trim_out: String,
    pub(super) setpts: String,
    /// The freeze hold (`""` for a clip that is not frozen).
    pub(super) freeze: String,
    /// The rate filter plus the speed-ramp lock: goes after the conform.
    pub(super) rate: String,
}

/// The software graph's timing for `seg` (source range `src_in..src_out`).
pub(super) fn timing(
    project: &Project,
    seg: &EdlSegment,
    fps: f64,
    src_in: u64,
    src_out: u64,
) -> Timing {
    let (trim_in, trim_out, setpts, freeze) = super::freeze_chain(
        seg.freeze.as_ref(),
        seg.speed,
        src_in,
        src_out,
        seg.timeline_out_ms - seg.timeline_in_ms,
        fps,
    );
    let vfps = seg
        .asset
        .as_deref()
        .and_then(|a| project.assets.get(a))
        .map(|a| super::fps_filter_for_asset(a, fps, seg.speed, seg.freeze.is_some()))
        .unwrap_or_else(|| format!("fps={}", super::fps_str(fps)));
    let (trim_in, trim_out, setpts, vfps) =
        super::window_part::apply(project, seg, fps, (trim_in, trim_out, setpts), vfps);
    Timing {
        trim_in,
        trim_out,
        setpts,
        freeze,
        rate: format!("{vfps}{}", super::ramp_timing::video_suffix(project, seg)),
    }
}

#[cfg(test)]
mod tests {
    use super::super::{build_graph, build_graph_gpu, RenderOptions};
    use cut_core::{edl_from_project, Project};
    use serde_json::json;

    /// Two 513 ms cuts of a 30 fps source in a 60 fps project: each clip's
    /// chain alone yields 32 frames, its slot 31 (frames 0..31, 31..62).
    fn project(src: &std::path::Path) -> Project {
        serde_json::from_value(json!({
            "schema": "shellx-cut/1", "name": "gpu-frames",
            "settings": {"width": 320, "height": 180, "fps": 60.0, "audio_rate": 48000},
            "assets": {"a1": {"path": src.display().to_string(), "hash": "sha256:gpu-frames",
                "probe": {"kind": "video", "width": 320, "height": 180, "fps": 30.0,
                          "has_audio": false,
                          "raw": {"streams": [{"codec_type": "video", "pix_fmt": "yuv420p",
                                               "avg_frame_rate": "30/1"}]}}}},
            "tracks": [{"id": "v1", "kind": "video", "clips": [
                {"id": "c1", "asset": "a1", "src_in_ms": 0, "src_out_ms": 513},
                {"id": "c2", "asset": "a1", "src_in_ms": 2500, "src_out_ms": 3013}]}],
        }))
        .expect("test project")
    }

    /// The trims and the frame locks of one graph, in order.
    fn timing_parts(filter: &str) -> Vec<String> {
        filter
            .split([',', ';', '[', ']'])
            .filter(|p| p.starts_with("trim=") || p.starts_with("fps=") || p.starts_with("tpad="))
            .map(str::to_string)
            .collect()
    }

    #[test]
    fn the_cuda_graph_uses_the_software_trims_rates_and_frame_locks() {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("src.mp4");
        std::fs::write(&src, b"").unwrap();
        let p = project(&src);
        let edl = edl_from_project(&p);
        let dir = tmp.path();
        let gpu = build_graph_gpu(&p, &edl, dir, RenderOptions::default())
            .expect("gpu graph")
            .filter;
        let sw = build_graph(
            &p,
            &edl,
            dir,
            true,
            true,
            true,
            RenderOptions::default(),
            None,
        )
        .expect("software graph")
        .filter;
        // Before the fix the CUDA chain had no lock: 2 x 32 frames instead of 62.
        assert_eq!(gpu.matches("trim=end_frame=31,").count(), 2, "{gpu}");
        let (g, s) = (timing_parts(&gpu), timing_parts(&sw));
        assert!(!g.is_empty());
        assert_eq!(g, s, "\nGPU: {gpu}\nsoftware: {sw}");
    }
}
