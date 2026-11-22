//! Same ordinary-overlay export regression also runs against de709a602.
use cut_core::{edit, edl_from_project, Project, ProjectSettings};
use cut_media::{render_final, PathFence, RenderOptions, RenderPreset};
use serde_json::json;
use std::{path::Path, process::Command};
fn rgb(path: &Path) -> Vec<u8> {
    let out = Command::new("ffmpeg")
        .args(["-v", "error", "-threads", "1", "-ss", "1.5", "-i"])
        .arg(path)
        .args(["-frames:v", "1", "-f", "rawvideo", "-pix_fmt", "rgb24", "-"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    out.stdout
}
fn bounds(rgb: &[u8]) -> [usize; 4] {
    let mut b = [320, 180, 0, 0];
    for y in 0..180 {
        for x in 0..320 {
            let i = (y * 320 + x) * 3;
            if rgb[i] > 180 && rgb[i + 1] < 60 {
                b[0] = b[0].min(x);
                b[1] = b[1].min(y);
                b[2] = b[2].max(x);
                b[3] = b[3].max(y);
            }
        }
    }
    b
}
#[test]
fn ordinary_overlay_split_preserves_exported_hold_and_linear_picture() {
    let dir = tempfile::tempdir().unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    let mut project = Project::new(
        "ordinary split",
        ProjectSettings {
            width: 320,
            height: 180,
            ..Default::default()
        },
    );
    for (name, color) in [("screen", "green"), ("overlay", "red")] {
        let path = dir.path().join(format!("{name}.mp4"));
        assert!(Command::new("ffmpeg")
            .args([
                "-v",
                "error",
                "-f",
                "lavfi",
                "-i",
                &format!("color=c={color}:size=320x180:rate=30:duration=3"),
                "-pix_fmt",
                "yuv420p",
                "-y"
            ])
            .arg(&path)
            .status()
            .unwrap()
            .success());
        project.assets.insert(
            name.into(),
            serde_json::from_value(json!({"path":path,"hash":format!("sha256:{name}")})).unwrap(),
        );
    }
    project.tracks[0].clips = vec![serde_json::from_value(
        json!({"id":"base","asset":"screen","src_in_ms":0,"src_out_ms":3000}),
    )
    .unwrap()];
    let mut overlay = project.tracks[0].clone();
    overlay.id = "v2".into();
    overlay.clips=vec![serde_json::from_value(json!({"id":"overlay","asset":"overlay","src_in_ms":0,"src_out_ms":3000,"transform":{"x":0.7,"y":0.1,"scale":0.2,"opacity":1.0}})).unwrap()];
    project.tracks.push(overlay);
    for interp in ["hold", "linear"] {
        let mut original = project.clone();
        let cut_core::Clip::Media(m) = &mut original.tracks.last_mut().unwrap().clips[0] else {
            panic!()
        };
        m.keyframes=serde_json::from_value(json!([{"param":"pos_x","interp":interp,"points":[{"t_ms":0,"value":0.7},{"t_ms":900,"value":0.1},{"t_ms":2500,"value":0.5}]}])).unwrap();
        let mut split = original.clone();
        edit::split(&mut split, "v2", 1000).unwrap();
        let fence = PathFence::new(dir.path()).unwrap();
        let mut frames = Vec::new();
        for (name, p) in [("original", &original), ("split", &split)] {
            let out = render_final(
                p,
                &edl_from_project(p),
                &fence,
                Path::new(&format!("{name}-{interp}.mp4")),
                &RenderPreset::default(),
                RenderOptions::default(),
                None,
            )
            .unwrap();
            frames.push(rgb(&out.path));
        }
        let a = bounds(&frames[0]);
        let b = bounds(&frames[1]);
        for i in 0..4 {
            assert!(
                a[i].abs_diff(b[i]) <= 2,
                "{interp}: unsplit {a:?}, split {b:?}"
            );
        }
        let error: f64 = frames[0]
            .iter()
            .zip(&frames[1])
            .map(|(a, b)| a.abs_diff(*b) as f64)
            .sum::<f64>()
            / frames[0].len() as f64;
        assert!(error < 1.0, "{interp}: mean pixel difference {error}");
    }
}
