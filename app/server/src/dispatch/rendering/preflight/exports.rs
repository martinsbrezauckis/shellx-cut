//! Stage admission for exports other than render.final.
use super::{host_preflight, Plan, Preflight};
use cut_core::{CutError, Edl, Project};
use cut_media::render::{resources::estimate_memory, RenderOptions, RenderPreset};
use serde_json::json;
use std::path::Path;

/// Range builds the full graph before trimming: account for the full graph.
pub(in crate::dispatch::rendering) fn export_preflight(
    project: &Project,
    edl: &Edl,
    out: &Path,
    preset: &mut RenderPreset,
    opts: &RenderOptions,
) -> Result<Preflight, CutError> {
    let (width, height) = opts.output_geometry(project, edl);
    let plan = Plan {
        project,
        edl,
        width,
        height,
        video_args: &preset.video_args,
        audio_kbps: 192,
        out,
    };
    let check = host_preflight(&plan, false, |args| {
        estimate_memory(project, edl, width, height, args)
    });
    admit_export(check, preset)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deliveries_use_the_explicit_geometry_and_pin_the_checked_encoder() {
        let p = Project::new("test", Default::default());
        let edl = Edl {
            duration_ms: 1_000,
            ..Default::default()
        };
        let mut preset = RenderPreset::default();
        let opts = RenderOptions {
            resolution: cut_media::render::Resolution::Explicit {
                width: 320,
                height: 180,
            },
            ..Default::default()
        };
        let check =
            export_preflight(&p, &edl, Path::new("/unused.mp4"), &mut preset, &opts).unwrap();
        let expected = estimate_memory(&p, &edl, 320, 180, &preset.video_args);
        assert_eq!(check.json["memory"]["estimate_mb"], expected.total_mb);
        assert_eq!(
            preset
                .video_args
                .iter()
                .filter(|s| *s == "-threads")
                .count(),
            1
        );
    }

    #[test]
    fn post_passes_are_checked_separately_and_gif_accounts_for_held_frames() {
        let p = Project::new("test", Default::default());
        let out = Path::new("/unused.mp4");
        let mut preset = RenderPreset {
            name: "gif".into(),
            video_args: vec!["-c:v".into(), "gif".into()],
            audio_args: vec![],
        };
        let plain = post_pass_preflight(&p, 1000, out, &mut preset, 480, 270, None).unwrap();
        let gif = post_pass_preflight(&p, 1000, out, &mut preset, 480, 270, Some(12)).unwrap();
        let held = (480_u64 * 270 * 4 * 12).div_ceil(1024 * 1024) + 32;
        assert_eq!(
            gif.json["memory"]["estimate_mb"].as_u64().unwrap(),
            plain.json["memory"]["estimate_mb"].as_u64().unwrap() + held
        );
        // A crop encode that cannot fit is refused independently of its source render.
        super::super::TEST_TOTAL_MB.with(|m| m.set(Some(1)));
        let refused = post_pass_preflight(
            &p,
            1000,
            out,
            &mut RenderPreset::default(),
            1920,
            1080,
            None,
        );
        super::super::TEST_TOTAL_MB.with(|m| m.set(None));
        assert_eq!(refused.err().unwrap().code, "out_of_memory");
    }
}

fn admit_export(mut check: Preflight, preset: &mut RenderPreset) -> Result<Preflight, CutError> {
    if let Some(error) = check.refusal.take() {
        return Err(error);
    }
    if let Some(n) = check.adapt_threads {
        preset.video_args.extend(["-threads".into(), n.to_string()]);
    }
    Ok(check)
}

/// A sequential post-pass reads one baked movie, not the original layers.
/// GIF's palette branch holds the scaled frames until the palette arrives.
pub(in crate::dispatch::rendering) fn post_pass_preflight(
    project: &Project,
    duration_ms: u64,
    out: &Path,
    preset: &mut RenderPreset,
    width: u32,
    height: u32,
    gif_fps: Option<u32>,
) -> Result<Preflight, CutError> {
    let mut baked = Project::new("export post-pass", project.settings.clone());
    baked.assets.insert("baked".into(), serde_json::from_value(json!({
        "path": "baked.mp4", "hash": "runtime", "probe": {"kind": "video", "width": project.settings.width, "height": project.settings.height, "fps": project.settings.fps}
    })).expect("baked source"));
    baked.tracks[0].clips.push(
        serde_json::from_value(json!({
            "id": "baked", "asset": "baked", "src_in_ms": 0, "src_out_ms": duration_ms
        }))
        .expect("baked clip"),
    );
    let edl = cut_core::edl_from_project(&baked);
    let plan = Plan {
        project: &baked,
        edl: &edl,
        width,
        height,
        video_args: &preset.video_args,
        audio_kbps: 192,
        out,
    };
    let check = host_preflight(&plan, false, |args| {
        let mut memory = estimate_memory(&baked, &edl, width, height, args);
        if let Some(fps) = gif_fps {
            let frames = duration_ms.saturating_mul(u64::from(fps)).div_ceil(1000);
            let held = u64::from(width)
                .saturating_mul(u64::from(height))
                .saturating_mul(4)
                .saturating_mul(frames)
                .div_ceil(1024 * 1024);
            memory.held_frames_mb = memory.held_frames_mb.saturating_add(held + 32);
            memory.total_mb = memory.total_mb.saturating_add(held + 32);
        }
        memory
    });
    admit_export(check, preset)
}
