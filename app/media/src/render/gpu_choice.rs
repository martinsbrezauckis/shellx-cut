//! Why an export that asked for the GPU render path did not take it.
//!
//! `SHELLX_CUT_RENDER_GPU` asks for the CUDA render path; the gate in
//! `render_target` can still send the export down the standard path (no
//! working CUDA filters, a timeline the GPU path cannot draw, too little
//! graphics memory). The export was fine but nobody was told: the receipt only
//! lacked `pipeline: "gpu"`. This names the reason, in the gate's own order.

use super::{gpu_gate_refusal, gpu_opt_in, gpu_vram_fits, RenderOptions};
use cut_core::{Edl, Project};

/// True when this process asks for the GPU render path.
pub fn gpu_render_requested() -> bool {
    gpu_opt_in()
}

/// Why the GPU render path is not used for this export, in plain words, or
/// `None` when it was not asked for or is used.
pub fn gpu_render_refusal(project: &Project, edl: &Edl, opts: &RenderOptions) -> Option<String> {
    if !gpu_opt_in() {
        return None;
    }
    let lead = "The GPU render path was asked for (SHELLX_CUT_RENDER_GPU)";
    let tail = "so this export used the standard render path";
    if !crate::hwencode::gpu_filters_available() {
        return Some(format!(
            "{lead}, but its CUDA filters do not run on this computer, {tail}."
        ));
    }
    if let Some(why) = gpu_gate_refusal(project, edl, opts) {
        return Some(format!(
            "{lead}, but {why}, which the GPU render path cannot draw, {tail}."
        ));
    }
    if !gpu_vram_fits(project, edl, opts) {
        return Some(format!(
            "{lead}, but this timeline needs more graphics memory than this GPU has, {tail}."
        ));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::super::{gpu_gate_refusal, RenderOptions};
    use cut_core::{edl_from_project, Project};
    use serde_json::json;

    fn project(extra_track: serde_json::Value, reverse: bool) -> Project {
        let mut tracks = vec![json!({"id": "v1", "kind": "video", "clips": [
            {"id": "c1", "asset": "a1", "src_in_ms": 0, "src_out_ms": 1000, "reverse": reverse}]})];
        if !extra_track.is_null() {
            tracks.push(extra_track);
        }
        serde_json::from_value(json!({
            "schema": "shellx-cut/1", "name": "gpu-choice",
            "settings": {"width": 320, "height": 180, "fps": 30.0, "audio_rate": 48000},
            "assets": {"a1": {"path": "/m/a.mp4", "hash": "sha256:a",
                "probe": {"kind": "video", "width": 320, "height": 180, "fps": 30.0}}},
            "tracks": tracks,
        }))
        .expect("test project")
    }

    #[test]
    fn the_gate_names_what_the_gpu_render_path_cannot_draw() {
        let why =
            |p: &Project| gpu_gate_refusal(p, &edl_from_project(p), &RenderOptions::default());
        assert_eq!(why(&project(serde_json::Value::Null, false)), None);
        let reversed = why(&project(serde_json::Value::Null, true)).unwrap();
        assert!(reversed.contains("reverse"), "{reversed}");
        let captions = project(
            json!({"id": "cap1", "kind": "caption", "clips": [
                {"id": "t1", "text": "hello", "range_ms": [0, 500]}]}),
            false,
        );
        assert_eq!(why(&captions), Some("the timeline has captions"));
    }
}
