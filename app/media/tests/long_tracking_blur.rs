//! Red-before-green integration tests for `long-tracking-blur-render`.
//! Exercises tracked blur redactions and keyframed overlays at 200 and 1000 points
//! through the real core edit + media render pipeline.

use cut_core::{
    edit::make_media_clip, edl_from_project, store::apply_edit_verb, Clip, Project,
    ProjectSettings, Track, TrackKind,
};
use cut_media::{render_final, PathFence, RenderOptions, RenderPreset};
use serde_json::json;
use std::path::{Path, PathBuf};

cut_core::isolate_test_process!();

fn create_private_dir(path: &Path) {
    std::fs::create_dir_all(path).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).unwrap();
    }
}

fn gen_checker_clip(dir: &Path, name: &str, dur_s: u32) -> PathBuf {
    let out = dir.join(name);
    let args: Vec<String> = [
        "-f",
        "lavfi",
        "-i",
        &format!(
            "color=c=black:s=320x240:r=30:d={dur_s},format=gray,\
             geq=lum='255*mod(floor(X/6)+floor(Y/6),2)',format=yuv420p"
        ),
        "-c:v",
        "libx264",
        "-preset",
        "ultrafast",
        "-pix_fmt",
        "yuv420p",
    ]
    .iter()
    .map(|s| s.to_string())
    .chain([out.display().to_string()])
    .collect();
    cut_media::ffmpeg::run_ffmpeg(&args).expect("checker clip generation");
    out
}

fn gen_solid_clip(dir: &Path, name: &str, color: &str, dur_s: u32) -> PathBuf {
    let out = dir.join(name);
    let args: Vec<String> = [
        "-f",
        "lavfi",
        "-i",
        &format!("color=c={color}:s=320x240:r=30:d={dur_s}"),
        "-c:v",
        "libx264",
        "-preset",
        "ultrafast",
        "-pix_fmt",
        "yuv420p",
    ]
    .iter()
    .map(|s| s.to_string())
    .chain([out.display().to_string()])
    .collect();
    cut_media::ffmpeg::run_ffmpeg(&args).expect("solid clip generation");
    out
}

fn frame_rgb(path: &Path, t_s: f64, w: u32, h: u32) -> Vec<u8> {
    let out = std::process::Command::new(cut_media::ffmpeg::ffmpeg_bin())
        .args(["-hide_banner", "-nostdin"])
        .args(if t_s > 0.0 {
            vec!["-ss".into(), format!("{t_s:.3}")]
        } else {
            vec![]
        })
        .arg("-i")
        .arg(path)
        .args(["-frames:v", "1", "-f", "rawvideo", "-pix_fmt", "rgb24", "-"])
        .output()
        .expect("ffmpeg raw frame");
    assert_eq!(out.stdout.len(), (w * h * 3) as usize, "one RGB frame");
    out.stdout
}

fn region_edge_energy(rgb: &[u8], w: u32, x0: u32, x1: u32, y0: u32, y1: u32) -> f64 {
    let mut sum = 0u64;
    let mut count = 0u64;
    for y in y0..y1 {
        for x in x0..x1 {
            let a = ((y * w + x) * 3) as usize;
            let b = ((y * w + x + 1) * 3) as usize;
            sum += rgb[a].abs_diff(rgb[b]) as u64;
            count += 1;
        }
    }
    sum as f64 / count as f64
}

fn add_video_asset(project: &mut Project, id: &str, path: &Path) {
    project.assets.insert(
        id.into(),
        cut_core::Asset {
            path: path.display().to_string(),
            hash: format!("sha256:{id}"),
            probe: Some(json!({"kind": "video", "width": 320, "height": 240, "fps": 30.0})),
            transcript: None,
            perception: None,
            proxy: None,
            filmstrip: None,
        },
    );
}

fn run_tracked_blur_points(points_count: usize, dur_s: u32) {
    let root = tempfile::tempdir().unwrap();
    let media_dir = root.path().join("media");
    create_private_dir(&media_dir);
    let src = gen_checker_clip(&media_dir, "checker.mp4", dur_s);

    let mut project = Project::new(
        "tracked-blur-long",
        ProjectSettings {
            width: 320,
            height: 240,
            fps: 30.0,
            audio_rate: 48_000,
            color: cut_core::ColorConfig::default(),
        },
    );
    add_video_asset(&mut project, "a1", &src);
    let dur_ms = (dur_s as u64) * 1000;
    project.track_mut("v1").unwrap().clips =
        vec![Clip::Media(make_media_clip("c1", "a1", 0, dur_ms))];

    let track: Vec<_> = (0..points_count)
        .map(|i| {
            let frac = i as f64 / (points_count - 1) as f64;
            json!({
                "t_ms": (frac * dur_ms as f64).round() as u64,
                "cx": 0.20 + 0.60 * frac,
                "cy": 0.50,
            })
        })
        .collect();
    apply_edit_verb(
        &mut project,
        "edit.redact",
        &json!({
            "clip": "c1",
            "shape": "rect",
            "mode": "blur",
            "points": [[0.0, 0.0], [0.25, 0.35]],
            "track": track,
        }),
    )
    .expect("edit.redact accepts long tracking points");

    let edl = edl_from_project(&project);
    let fence = PathFence::new(&media_dir).unwrap();
    let out = render_final(
        &project,
        &edl,
        &fence,
        Path::new(&format!("tracked-{points_count}.mp4")),
        &RenderPreset::default(),
        RenderOptions::default(),
        None,
    )
    .expect("render tracked blur");

    // Early frame: box near left (x ≈ 70..74 px), right (x ≈ 246 px) still sharp.
    let early = frame_rgb(&out.path, 0.2, 320, 240);
    let early_left = region_edge_energy(&early, 320, 54, 86, 96, 144);
    let early_right = region_edge_energy(&early, 320, 226, 266, 96, 144);
    assert!(
        early_left * 2.5 < early_right,
        "early blur must cover left: left={early_left:.1} vs right={early_right:.1}"
    );

    // Late frame: box near right (x ≈ 246..252 px), left (x ≈ 70 px) now sharp.
    let late = frame_rgb(&out.path, dur_s as f64 - 0.2, 320, 240);
    let late_left = region_edge_energy(&late, 320, 54, 86, 96, 144);
    let late_right = region_edge_energy(&late, 320, 226, 266, 96, 144);
    assert!(
        late_right * 2.5 < late_left,
        "late blur must cover right: right={late_right:.1} vs left={late_left:.1}"
    );
}

fn run_keyframe_overlay_points(points_count: usize, dur_s: u32) {
    let root = tempfile::tempdir().unwrap();
    let media_dir = root.path().join("media");
    create_private_dir(&media_dir);
    let red = gen_solid_clip(&media_dir, "red.mp4", "red", dur_s);
    let green = gen_solid_clip(&media_dir, "green.mp4", "green", dur_s);

    let mut project = Project::new(
        "long-keyframes",
        ProjectSettings {
            width: 320,
            height: 240,
            fps: 30.0,
            audio_rate: 48_000,
            color: cut_core::ColorConfig::default(),
        },
    );
    add_video_asset(&mut project, "a1", &red);
    add_video_asset(&mut project, "a2", &green);
    let dur_ms = (dur_s as u64) * 1000;
    project.track_mut("v1").unwrap().clips =
        vec![Clip::Media(make_media_clip("c1", "a1", 0, dur_ms))];

    let mut ov = make_media_clip("c2", "a2", 0, dur_ms);
    ov.transform = Some(cut_core::ClipTransform {
        x: 0.0,
        y: 0.25,
        scale: 0.25,
        ..cut_core::ClipTransform::identity()
    });
    project.tracks.push(Track {
        id: "v2".into(),
        kind: TrackKind::Video,
        clips: vec![Clip::Media(ov)],
        gain_db: 0.0,
        gain_windows: vec![],
        blend_mode: None,
        visible: true,
        locked: false,
        muted: false,
        solo: false,
        pan: 0.0,
    });

    let pos_points: Vec<_> = (0..points_count)
        .map(|i| {
            let frac = i as f64 / (points_count - 1) as f64;
            json!({
                "t_ms": (frac * dur_ms as f64).round() as u64,
                "value": 0.05 + 0.65 * frac,
            })
        })
        .collect();
    apply_edit_verb(
        &mut project,
        "edit.keyframe",
        &json!({"clip": "c2", "param": "pos_x", "interp": "linear", "points": pos_points}),
    )
    .expect("edit.keyframe pos_x accepts long curve");

    let opacity_points: Vec<_> = (0..points_count)
        .map(|i| {
            let frac = i as f64 / (points_count - 1) as f64;
            json!({
                "t_ms": (frac * dur_ms as f64).round() as u64,
                "value": if i % 2 == 0 { 1.0 } else { 0.85 },
            })
        })
        .collect();
    apply_edit_verb(
        &mut project,
        "edit.keyframe",
        &json!({
            "clip": "c2",
            "param": "opacity",
            "interp": "ease_in_out_bounce",
            "points": opacity_points,
        }),
    )
    .expect("edit.keyframe eased opacity accepts long curve");

    let edl = edl_from_project(&project);
    let fence = PathFence::new(&media_dir).unwrap();
    let out = render_final(
        &project,
        &edl,
        &fence,
        Path::new(&format!("kf-{points_count}.mp4")),
        &RenderPreset::default(),
        RenderOptions::default(),
        None,
    )
    .expect("render long keyframes");

    // Pure green is G=128; the curve alternates 1.0 and 0.85 opacity, so the
    // overlay reads G of about 109 to 128 over red.
    // Green block starts near x=16..96 and ends near x=224..304 (at y=90).
    let early = frame_rgb(&out.path, 0.2, 320, 240);
    let late = frame_rgb(&out.path, dur_s as f64 - 0.2, 320, 240);
    let px_at = |buf: &[u8], x: u32, y: u32| {
        let i = ((y * 320 + x) * 3) as usize;
        (buf[i], buf[i + 1])
    };
    let (r_el, g_el) = px_at(&early, 48, 90);
    let (r_er, g_er) = px_at(&early, 260, 90);
    assert!(
        g_el > 90 && r_el < 80 && r_er > 140 && g_er < 80,
        "early green overlay left, red right: left=({r_el},{g_el}) right=({r_er},{g_er})"
    );
    let (r_ll, g_ll) = px_at(&late, 48, 90);
    let (r_lr, g_lr) = px_at(&late, 260, 90);
    assert!(
        r_ll > 140 && g_ll < 80 && g_lr > 90 && r_lr < 80,
        "late red base left, green overlay right: left=({r_ll},{g_ll}) right=({r_lr},{g_lr})"
    );
}

#[test]
fn tracked_blur_200_points_renders_and_moves_blur() {
    run_tracked_blur_points(200, 4);
}

#[test]
fn tracked_blur_1000_points_renders_and_moves_blur() {
    run_tracked_blur_points(1000, 10);
}

#[test]
fn keyframe_overlay_200_and_1000_points_apply_and_render() {
    run_keyframe_overlay_points(200, 4);
    run_keyframe_overlay_points(1000, 10);
}

fn audio_rms(path: &Path, from_s: f64, len_s: f64) -> f64 {
    let out = std::process::Command::new(cut_media::ffmpeg::ffmpeg_bin())
        .args(["-hide_banner", "-nostdin", "-loglevel", "error"])
        .args(["-ss", &format!("{from_s:.3}"), "-t", &format!("{len_s:.3}")])
        .arg("-i")
        .arg(path)
        .args(["-vn", "-ac", "1", "-ar", "48000", "-f", "s16le", "-"])
        .output()
        .expect("decode audio");
    let samples: Vec<f64> = out
        .stdout
        .as_chunks::<2>()
        .0
        .iter()
        .map(|b| i16::from_le_bytes(*b) as f64)
        .collect();
    assert!(
        !samples.is_empty(),
        "no audio decoded from {}",
        path.display()
    );
    (samples.iter().map(|s| s * s).sum::<f64>() / samples.len() as f64).sqrt()
}

/// A long volume curve (above the released expression's 64 points) is read
/// from a command file and still fades the sound.
#[test]
fn volume_200_points_apply_and_fade_the_sound() {
    let root = tempfile::tempdir().unwrap();
    let media_dir = root.path().join("media");
    create_private_dir(&media_dir);
    let src = media_dir.join("tone.mp4");
    let args: Vec<String> = [
        "-f",
        "lavfi",
        "-i",
        "color=c=black:s=320x240:r=30:d=4",
        "-f",
        "lavfi",
        "-i",
        "sine=frequency=440:sample_rate=48000:duration=4",
        "-c:v",
        "libx264",
        "-preset",
        "ultrafast",
        "-pix_fmt",
        "yuv420p",
        "-c:a",
        "aac",
        "-shortest",
    ]
    .iter()
    .map(|s| s.to_string())
    .chain([src.display().to_string()])
    .collect();
    cut_media::ffmpeg::run_ffmpeg(&args).expect("tone clip generation");

    let mut project = Project::new(
        "long-volume",
        ProjectSettings {
            width: 320,
            height: 240,
            fps: 30.0,
            audio_rate: 48_000,
            color: cut_core::ColorConfig::default(),
        },
    );
    add_video_asset(&mut project, "a1", &src);
    project.track_mut("v1").unwrap().clips =
        vec![Clip::Media(make_media_clip("c1", "a1", 0, 4000))];
    project.track_mut("a1t").unwrap().clips =
        vec![Clip::Media(make_media_clip("c1a", "a1", 0, 4000))];
    let points: Vec<_> = (0..200)
        .map(|i| {
            let frac = i as f64 / 199.0;
            json!({"t_ms": (frac * 4000.0).round() as u64, "value": 1.0 - 0.95 * frac})
        })
        .collect();
    apply_edit_verb(
        &mut project,
        "edit.keyframe",
        &json!({"clip": "c1a", "param": "volume", "interp": "linear", "points": points}),
    )
    .expect("edit.keyframe volume accepts a long curve");

    let edl = edl_from_project(&project);
    let fence = PathFence::new(&media_dir).unwrap();
    let out = render_final(
        &project,
        &edl,
        &fence,
        Path::new("volume-200.mp4"),
        &RenderPreset::default(),
        RenderOptions::default(),
        None,
    )
    .expect("render long volume curve");
    let early = audio_rms(&out.path, 0.2, 0.4);
    let late = audio_rms(&out.path, 3.4, 0.4);
    assert!(
        early > 1000.0 && late < early * 0.2,
        "the curve must fade the sound: early RMS {early:.0}, late RMS {late:.0}"
    );
}
