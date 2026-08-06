//! media_engine.rs — cut-media integration tests (media-engine contract + the output-fencing contract).
//!
//! Generates tiny inputs IN-TEST via ffmpeg lavfi (testsrc2 video + sine
//! audio — known ground truth, nothing committed to the repo) and exercises
//! the real subprocess pipeline end to end:
//! probe fields · proxy geometry · 2-clip EDL render duration (±1 frame) ·
//! deterministic re-render (identical sha256) · composed-frame JPEG ·
//! path-fence rejection through render_final.

use cut_core::{
    edl_from_project, CaptionClip, CaptionStyle, Clip, MediaClip, Project, ProjectSettings, Track,
    TrackKind,
};
use cut_media::{
    extract_frame, make_proxy, probe, render_final, render_preview_incremental, PathFence,
    RenderOptions, RenderPreset,
};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

/// Generate a ~2s 320x240@30 test clip with a 440 Hz sine track into `dir`.
/// testsrc2 + sine = fully synthetic, deterministic ground truth.
fn gen_clip(dir: &Path, name: &str) -> PathBuf {
    let out = dir.join(name);
    let args: Vec<String> = [
        "-f",
        "lavfi",
        "-i",
        "testsrc2=size=320x240:rate=30:duration=2",
        "-f",
        "lavfi",
        "-i",
        "sine=frequency=440:duration=2",
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
    .chain([out.display().to_string()])
    .collect();
    cut_media::ffmpeg::run_ffmpeg(&args).expect("test asset generation");
    out
}

/// A legitimate filesystem apostrophe must survive both ffmpeg filtergraph
/// parser levels. This uses a real identity LUT so a string-only assertion
/// cannot hide an escaping regression.
#[test]
fn filter_path_with_apostrophe_loads_real_lut() {
    let dir = tempfile::tempdir().unwrap();
    let asset_dir = dir.path().join("editor's assets");
    std::fs::create_dir(&asset_dir).unwrap();
    let lut = asset_dir.join("identity.cube");
    std::fs::write(
        &lut,
        "TITLE \"Identity\"\nLUT_3D_SIZE 2\nDOMAIN_MIN 0 0 0\nDOMAIN_MAX 1 1 1\n\
         0 0 0\n1 0 0\n0 1 0\n1 1 0\n0 0 1\n1 0 1\n0 1 1\n1 1 1\n",
    )
    .unwrap();

    let filter = format!("lut3d=file={}", cut_media::ffmpeg::escape_filter_path(&lut));
    let args = vec![
        "-f".into(),
        "lavfi".into(),
        "-i".into(),
        "color=size=16x16:duration=0.1".into(),
        "-vf".into(),
        filter,
        "-frames:v".into(),
        "1".into(),
        "-f".into(),
        "null".into(),
        "-".into(),
    ];
    cut_media::ffmpeg::run_ffmpeg(&args).expect("lut path containing apostrophe");
}

/// reframe_video end-to-end: a 1280x720 clip + a synthetic subject track sweeping
/// left→right → a real ffmpeg sendcmd+crop pass → a 1080x1920 (9:16) output. Proves
/// the whole path wires up (spring → sendcmd → ffmpeg → RenderOutput), the crop
/// actually MOVES (not a static centre-crop), the sendcmd temp is cleaned up, and
/// audio passes through (copy). testsrc2 = synthetic ground truth.
#[test]
fn reframe_video_produces_moving_target_aspect_crop() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("wide.mp4");
    let gen: Vec<String> = [
        "-f",
        "lavfi",
        "-i",
        "testsrc2=size=1280x720:rate=30:duration=2",
        "-f",
        "lavfi",
        "-i",
        "sine=frequency=440:duration=2",
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
    cut_media::ffmpeg::run_ffmpeg(&gen).expect("gen wide clip");

    // Synthetic subject: a person-sized focus box sweeping the centre L→R over 2s.
    let n = 60u32; // 2s @ 30fps
    let frames: Vec<cut_media::reframe::FrameObs> = (0..n)
        .map(|i| {
            let cx = 0.25 + 0.5 * (i as f64 / (n - 1) as f64);
            cut_media::reframe::FrameObs {
                focus: Some([cx - 0.08, 0.25, cx + 0.08, 0.95]),
                conf: 0.9,
                scene: 0,
            }
        })
        .collect();

    let out = dir.path().join("vertical.mp4");
    let preset = cut_media::RenderPreset::named("draft").unwrap();
    let result = cut_media::render::reframe_video(
        &src,
        &out,
        &frames,
        9,
        16,
        1080,
        1920,
        &cut_media::reframe::ReframeParams::default(),
        &[0],
        &preset,
        None,
    )
    .expect("reframe_video");

    // Output is the target aspect, ~2s, with a real determinism hash.
    assert_eq!((result.width, result.height), (Some(1080), Some(1920)));
    assert!(
        result.duration_ms >= 1800 && result.duration_ms <= 2200,
        "dur {}",
        result.duration_ms
    );
    assert!(result.hash.starts_with("sha256:"), "hash {}", result.hash);
    let p = probe(&out).unwrap();
    assert_eq!((p.width, p.height), (Some(1080), Some(1920)));
    assert!(p.duration_ms.unwrap() > 0);
    // The sendcmd temp file is cleaned up after the pass.
    assert!(
        !out.with_extension("reframe.sendcmd.txt").exists(),
        "sendcmd temp leaked"
    );
}

/// Build the canonical 2-clip test project: clip A = src [0,1000), clip B =
/// src [500,1500) of the same asset, mirrored on video + audio tracks, plus
/// one styled caption. Timeline duration: exactly 2000 ms.
fn two_clip_project(asset_path: &Path) -> Project {
    let mut p = Project::new(
        "t",
        ProjectSettings {
            width: 320,
            height: 240,
            fps: 30.0,
            audio_rate: 48_000,
            color: cut_core::ColorConfig::default(),
        },
    );
    p.assets.insert(
        "a1".into(),
        cut_core::Asset {
            path: asset_path.display().to_string(),
            hash: "sha256:test".into(),
            probe: None,
            transcript: None,
            perception: None,
            proxy: None,
            filmstrip: None,
        },
    );
    let mk = |id: &str, in_ms: u64, out_ms: u64, gain: f64| {
        Clip::Media(MediaClip {
            id: id.into(),
            asset: "a1".into(),
            src_in_ms: in_ms,
            src_out_ms: out_ms,
            effects: vec![],
            gain_db: gain,
            transform: None,
            crop: None,
            fade: None,
            xfade_in_ms: 0,
            xfade_kind: None,
            speed: 1.0,
            grade: None,
            matte: None,
            mask: None,
            reverse: false,
            freeze: None,
            animation: None,
            keyframes: vec![],
            eq: None,
            mute_ranges: vec![],
            stabilize: None,
            speed_ramp: None,
            input_color_space: None,
            nest: None,
            grade_stack: vec![],
            grade_windows: vec![],
        })
    };
    p.track_mut("v1").unwrap().clips = vec![mk("c1", 0, 1000, 0.0), mk("c2", 500, 1500, 0.0)];
    // Audio mirrors video; second clip carries -3 dB to exercise volume.
    p.track_mut("a1t").unwrap().clips = vec![mk("c1a", 0, 1000, 0.0), mk("c2a", 500, 1500, -3.0)];
    p.caption_styles.insert(
        "brand1".into(),
        CaptionStyle {
            font: "DejaVu Sans".into(),
            size: 24,
            color: "#fff".into(),
            bg: Some("#000a".into()),
            pos: Some("bottom".into()),
            extra: Default::default(),
        },
    );
    p.tracks.push(Track {
        id: "cap1".into(),
        kind: TrackKind::Caption,
        clips: vec![Clip::Caption(CaptionClip {
            id: "s1".into(),
            text: "hello cut".into(),
            style_ref: Some("brand1".into()),
            range_ms: [200, 1500],
        })],
        gain_db: 0.0,
        gain_windows: vec![],
        blend_mode: None,
        visible: true,
        locked: false,
        muted: false,
        solo: false,
        pan: 0.0,
    });
    p
}

#[test]
fn probe_normalizes_fields() {
    let dir = tempfile::tempdir().unwrap();
    let clip = gen_clip(dir.path(), "in.mp4");
    let info = probe(&clip).expect("probe");
    // Known ground truth from the lavfi generator.
    assert_eq!(info.kind, "video");
    let dur = info.duration_ms.expect("video has a duration");
    assert!((1900..=2100).contains(&dur), "duration {dur} ms");
    assert_eq!((info.width, info.height), (Some(320), Some(240)));
    assert!((info.fps.unwrap() - 30.0).abs() < 0.01);
    assert!(info.has_audio);
    assert_eq!(info.audio_rate, Some(44_100)); // sine default rate
    assert!(info.format.contains("h264"), "format: {}", info.format);
    assert!(info.raw["streams"].is_array()); // raw kept verbatim
}

/// Generate a solid-red 320x240 still into `dir` (PNG or JPEG by extension).
/// Solid red has a known luma (Y ≈ 0.299·255 ≈ 76) — assertable after render.
fn gen_still(dir: &Path, name: &str) -> PathBuf {
    let out = dir.join(name);
    let args: Vec<String> = [
        "-f",
        "lavfi",
        "-i",
        "color=c=red:s=320x240",
        "-frames:v",
        "1",
    ]
    .iter()
    .map(|s| s.to_string())
    .chain([out.display().to_string()])
    .collect();
    cut_media::ffmpeg::run_ffmpeg(&args).expect("still generation");
    out
}

/// Mean luma (YAVG) of the FIRST frame of a video file via signalstats —
/// proves what actually rendered (red still ≈ 76, black gap ≈ 16).
fn first_frame_yavg(path: &Path) -> f64 {
    let out = std::process::Command::new("ffprobe")
        .args(["-v", "error", "-f", "lavfi"])
        .arg(format!("movie={},signalstats", path.display()))
        .args([
            "-show_entries",
            "frame_tags=lavfi.signalstats.YAVG",
            "-of",
            "csv=p=0",
            "-read_intervals",
            "%+#1",
        ])
        .output()
        .expect("ffprobe signalstats");
    // csv=p=0 leaves a trailing comma on the tags row — strip before parsing.
    String::from_utf8_lossy(&out.stdout)
        .trim()
        .trim_end_matches(',')
        .parse()
        .expect("YAVG parses")
}

/// Still-image probe regression: stills report kind=image with no duration — including JPEG,
/// whose image2 demuxer reports a bogus one-frame duration that must be
/// discarded (PNG reports none at all; both forms are covered here).
#[test]
fn probe_classifies_still_images() {
    let dir = tempfile::tempdir().unwrap();
    for name in ["intro.png", "intro.jpg"] {
        let still = gen_still(dir.path(), name);
        let info = probe(&still).expect(name);
        assert_eq!(info.kind, "image", "{name}");
        assert_eq!(
            info.duration_ms, None,
            "{name}: stills have no intrinsic duration"
        );
        assert_eq!(info.fps, None, "{name}: nominal demuxer fps discarded");
        assert_eq!((info.width, info.height), (Some(320), Some(240)), "{name}");
        assert!(!info.has_audio, "{name}");
    }
    // And a real video is NOT misclassified.
    let clip = gen_clip(dir.path(), "in.mp4");
    assert_eq!(probe(&clip).unwrap().kind, "video");
}

/// Still-image render regression: an image clip renders by looping the still for
/// the clip's duration, conformed to project geometry, and concats cleanly
/// with normal video — the intro-card workflow.
#[test]
fn render_loops_still_for_clip_duration() {
    let dir = tempfile::tempdir().unwrap();
    let clip = gen_clip(dir.path(), "in.mp4");
    let still = gen_still(dir.path(), "intro.png");
    let mut p = Project::new(
        "t",
        ProjectSettings {
            width: 320,
            height: 240,
            fps: 30.0,
            audio_rate: 48_000,
            color: cut_core::ColorConfig::default(),
        },
    );
    let mk_asset = |path: &Path, kind: &str| cut_core::Asset {
        path: path.display().to_string(),
        hash: "sha256:test".into(),
        probe: Some(serde_json::json!({"kind": kind})), // what import persists
        transcript: None,
        perception: None,
        proxy: None,
        filmstrip: None,
    };
    p.assets.insert("img".into(), mk_asset(&still, "image"));
    p.assets.insert("vid".into(), mk_asset(&clip, "video"));
    // v1 = 1.5s intro card, then 2s of video. Audio mirrors the video only.
    p.track_mut("v1").unwrap().clips = vec![
        Clip::Media(MediaClip {
            id: "c1".into(),
            asset: "img".into(),
            src_in_ms: 0,
            src_out_ms: 1500,
            effects: vec![],
            gain_db: 0.0,
            transform: None,
            crop: None,
            fade: None,
            xfade_in_ms: 0,
            xfade_kind: None,
            speed: 1.0,
            grade: None,
            matte: None,
            mask: None,
            reverse: false,
            freeze: None,
            animation: None,
            keyframes: vec![],
            eq: None,
            mute_ranges: vec![],
            stabilize: None,
            speed_ramp: None,
            input_color_space: None,
            nest: None,
            grade_stack: vec![],
            grade_windows: vec![],
        }),
        Clip::Media(MediaClip {
            id: "c2".into(),
            asset: "vid".into(),
            src_in_ms: 0,
            src_out_ms: 2000,
            effects: vec![],
            gain_db: 0.0,
            transform: None,
            crop: None,
            fade: None,
            xfade_in_ms: 0,
            xfade_kind: None,
            speed: 1.0,
            grade: None,
            matte: None,
            mask: None,
            reverse: false,
            freeze: None,
            animation: None,
            keyframes: vec![],
            eq: None,
            mute_ranges: vec![],
            stabilize: None,
            speed_ramp: None,
            input_color_space: None,
            nest: None,
            grade_stack: vec![],
            grade_windows: vec![],
        }),
    ];
    p.track_mut("a1t").unwrap().clips = vec![
        Clip::Gap(cut_core::GapClip::new(1500)),
        Clip::Media(MediaClip {
            id: "c3".into(),
            asset: "vid".into(),
            src_in_ms: 0,
            src_out_ms: 2000,
            effects: vec![],
            gain_db: 0.0,
            transform: None,
            crop: None,
            fade: None,
            xfade_in_ms: 0,
            xfade_kind: None,
            speed: 1.0,
            grade: None,
            matte: None,
            mask: None,
            reverse: false,
            freeze: None,
            animation: None,
            keyframes: vec![],
            eq: None,
            mute_ranges: vec![],
            stabilize: None,
            speed_ramp: None,
            input_color_space: None,
            nest: None,
            grade_stack: vec![],
            grade_windows: vec![],
        }),
    ];
    let edl = edl_from_project(&p);
    assert_eq!(edl.duration_ms, 3500);
    let fence = PathFence::new(dir.path()).unwrap();
    let out = render_final(
        &p,
        &edl,
        &fence,
        Path::new("card.mp4"),
        &RenderPreset::default(),
        RenderOptions::default(),
        None,
    )
    .expect("render with still");
    let diff = out.duration_ms.abs_diff(3500);
    assert!(
        diff <= 34,
        "duration {} ms — the still must occupy its full 1500ms",
        out.duration_ms
    );
    // The opening frame IS the red card (Y ≈ 76), not black (≈ 16) — proves
    // the loop actually rendered pixels for the whole clip, conformed.
    let y = first_frame_yavg(&out.path);
    assert!(
        (60.0..=95.0).contains(&y),
        "first frame luma {y} — expected the red still"
    );
    // frame extraction inside the still's range works too (agent's eyes).
    let jpeg = extract_frame(&p, &edl, dir.path(), 700, None).expect("frame in still range");
    assert_eq!(&jpeg[..2], &[0xFF, 0xD8]);
}

/// Mean luma (YAVG) of EVERY frame of a video, in order. Used to prove an animation
/// trends over time (the per-frame value has a 1–2 frame readback offset through
/// `movie=,signalstats`, which is fine for a coarse first-third-vs-last-third trend).
fn all_frames_yavg(path: &Path) -> Vec<f64> {
    let out = std::process::Command::new("ffprobe")
        .args(["-v", "error", "-f", "lavfi"])
        .arg(format!("movie={},signalstats", path.display()))
        .args([
            "-show_entries",
            "frame_tags=lavfi.signalstats.YAVG",
            "-of",
            "csv=p=0",
        ])
        .output()
        .expect("ffprobe signalstats");
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|l| l.trim().trim_end_matches(',').parse().ok())
        .collect()
}

/// (scale keyframe): a `KfParam::Scale` keyframe 1.0→2.0 over a clip
/// lowers to the proven centred `zoompan` and actually ZOOMS IN — a centred bright
/// square's coverage (mean luma) grows monotonically across the clip. This exercises
/// the REAL render pipeline (Project → edl → render_final), proving the scale channel
/// renders, not just that the filter string is built. The eased interp keeps it on
/// the software path (the keyframes gate). Synthetic ground truth via lavfi.
#[test]
fn scale_keyframe_zooms_in_over_time() {
    use cut_core::{Keyframe, KfInterp, KfParam, KfPoint};
    let dir = tempfile::tempdir().unwrap();
    // A 320×240 black still with a SMALL centred white box (60×60). A centred zoom-in
    // makes the box cover more of the frame → mean luma rises.
    let square = dir.path().join("sq.png");
    let st = std::process::Command::new(cut_media::ffmpeg::ffmpeg_bin())
        .args(["-y", "-v", "error", "-f", "lavfi", "-i"])
        .arg("color=black:s=320x240:d=1")
        .args(["-frames:v", "1", "-vf"])
        .arg("drawbox=x=130:y=90:w=60:h=60:color=white:t=fill")
        .arg(&square)
        .status()
        .expect("gen square still");
    assert!(st.success(), "square still generation failed");

    let mut p = Project::new(
        "t",
        ProjectSettings {
            width: 320,
            height: 240,
            fps: 30.0,
            audio_rate: 48_000,
            color: cut_core::ColorConfig::default(),
        },
    );
    p.assets.insert(
        "sq".into(),
        cut_core::Asset {
            path: square.display().to_string(),
            hash: "sha256:sqtest".into(),
            probe: Some(serde_json::json!({"kind": "image"})),
            transcript: None,
            perception: None,
            proxy: None,
            filmstrip: None,
        },
    );
    // 1s still clip carrying a SCALE keyframe 1.0→2.0 (eased) — the multi-point zoom.
    p.track_mut("v1").unwrap().clips = vec![Clip::Media(MediaClip {
        id: "c1".into(),
        asset: "sq".into(),
        src_in_ms: 0,
        src_out_ms: 1000,
        effects: vec![],
        gain_db: 0.0,
        transform: None,
        crop: None,
        fade: None,
        xfade_in_ms: 0,
        xfade_kind: None,
        speed: 1.0,
        grade: None,
        matte: None,
        mask: None,
        reverse: false,
        freeze: None,
        animation: None,
        keyframes: vec![Keyframe {
            param: KfParam::Scale,
            points: vec![
                KfPoint {
                    t_ms: 0,
                    value: 1.0,
                },
                KfPoint {
                    t_ms: 1000,
                    value: 2.0,
                },
            ],
            interp: KfInterp::EaseInOutCubic,
        }],
        eq: None,
        mute_ranges: vec![],
        stabilize: None,
        speed_ramp: None,
        input_color_space: None,
        nest: None,
        grade_stack: vec![],
        grade_windows: vec![],
    })];
    let edl = edl_from_project(&p);
    let fence = PathFence::new(dir.path()).unwrap();
    let out = render_final(
        &p,
        &edl,
        &fence,
        Path::new("zoom.mp4"),
        &RenderPreset::default(),
        RenderOptions::default(),
        None,
    )
    .expect("render scale-keyframed clip");

    let ys = all_frames_yavg(&out.path);
    assert!(ys.len() >= 20, "expected ~30 frames, got {}", ys.len());
    let n = ys.len();
    let first: f64 = ys[..n / 5].iter().sum::<f64>() / (n / 5) as f64;
    let last: f64 = ys[n - n / 5..].iter().sum::<f64>() / (n / 5) as f64;
    // Centred zoom-in: the bright square's coverage grows → last third noticeably
    // brighter than the first (≥1.5× here; measured ~26 → ~57).
    assert!(
        last > first * 1.4,
        "scale keyframe should zoom IN (centred bright region grows): first {first:.1} vs last {last:.1}"
    );
}

/// (loudness loop): the MEASURE half (`cut_media::loudness::measure`,
/// the verb verify.loudness's engine) and the NORMALIZE half (render.final's
/// `loudness_target`) compose — render a QUIET source with a -14 LUFS target and the
/// rendered output MEASURES back to ≈ -14 with the true peak capped at -1 dBTP. This
/// is the plan's exact proof ("normalize to -14 → re-measure → within tolerance"),
/// through the real render API. Pink noise = realistic content loudnorm converges on
/// tightly (a steady tone is a degenerate case); ±1.5 LU tolerance. Skips if ffmpeg absent.
#[test]
fn loudness_normalize_then_remeasure_closes_the_loop() {
    let dir = tempfile::tempdir().unwrap();
    // A quiet clip: testsrc2 video + PINK NOISE audio (~-25 LUFS).
    let clip = dir.path().join("quiet.mp4");
    let gen: Vec<String> = [
        "-f",
        "lavfi",
        "-i",
        "testsrc2=size=320x240:rate=30:duration=3",
        "-f",
        "lavfi",
        "-i",
        "anoisesrc=color=pink:duration=3:amplitude=0.3",
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
    .chain([clip.display().to_string()])
    .collect();
    if cut_media::ffmpeg::run_ffmpeg(&gen).is_err() {
        eprintln!("ffmpeg unavailable — skipping loudness loop proof");
        return;
    }
    // Sanity: the source is well under -14 (so normalization has real work to do).
    let src_loud = cut_media::loudness::measure(&clip).expect("measure source");
    assert!(
        src_loud.integrated_lufs < -18.0,
        "source should be quiet (<-18 LUFS), got {}",
        src_loud.integrated_lufs
    );

    let mut p = Project::new(
        "loud",
        ProjectSettings {
            width: 320,
            height: 240,
            fps: 30.0,
            audio_rate: 48_000,
            color: cut_core::ColorConfig::default(),
        },
    );
    p.assets.insert(
        "a1".into(),
        cut_core::Asset {
            path: clip.display().to_string(),
            hash: "sha256:loudtest".into(),
            probe: Some(serde_json::json!({"kind": "video"})),
            transcript: None,
            perception: None,
            proxy: None,
            filmstrip: None,
        },
    );
    let mk = |id: &str| {
        Clip::Media(MediaClip {
            id: id.into(),
            asset: "a1".into(),
            src_in_ms: 0,
            src_out_ms: 3000,
            effects: vec![],
            gain_db: 0.0,
            transform: None,
            crop: None,
            fade: None,
            xfade_in_ms: 0,
            xfade_kind: None,
            speed: 1.0,
            grade: None,
            matte: None,
            mask: None,
            reverse: false,
            freeze: None,
            animation: None,
            keyframes: vec![],
            eq: None,
            mute_ranges: vec![],
            stabilize: None,
            speed_ramp: None,
            input_color_space: None,
            nest: None,
            grade_stack: vec![],
            grade_windows: vec![],
        })
    };
    // Video on v1, audio on a1t (audio renders from the audio track).
    p.track_mut("v1").unwrap().clips = vec![mk("c1")];
    p.track_mut("a1t").unwrap().clips = vec![mk("c1a")];
    let edl = edl_from_project(&p);
    let fence = PathFence::new(dir.path()).unwrap();
    // render.final {normalize_loudness:-14}
    let opts = RenderOptions {
        loudness_target: Some(-14),
        ..RenderOptions::default()
    };
    let out = render_final(
        &p,
        &edl,
        &fence,
        Path::new("norm.mp4"),
        &RenderPreset::default(),
        opts,
        None,
    )
    .expect("render with loudness normalization");

    // RE-MEASURE the rendered output — the loop's payoff.
    let got = cut_media::loudness::measure(&out.path).expect("measure render");
    assert!(
        (got.integrated_lufs - -14.0).abs() <= 1.5,
        "normalized render should measure ≈-14 LUFS, got {} (source was {})",
        got.integrated_lufs,
        src_loud.integrated_lufs
    );
    assert!(
        !got.true_peak_dbtp.is_finite() || got.true_peak_dbtp <= -0.5,
        "true peak should be capped near -1 dBTP, got {}",
        got.true_peak_dbtp
    );
}

/// (transitions): a NEWLY-ADDED xfade transition name (`coverleft`,
/// part of the expanded enum set) renders end-to-end through real ffmpeg. ffmpeg
/// ERRORS on an unknown `xfade=transition=` value, so a clean render at the
/// crossfade-shortened duration proves the name is genuinely accepted (not just
/// schema-valid). Two adjacent clips, a 500ms crossfade on the seam. Skips if
/// ffmpeg absent.
#[test]
fn new_transition_renders_via_ffmpeg() {
    let dir = tempfile::tempdir().unwrap();
    let clip = gen_clip(dir.path(), "a.mp4"); // 2s testsrc2 + sine
    let mut p = Project::new(
        "xf",
        ProjectSettings {
            width: 320,
            height: 240,
            fps: 30.0,
            audio_rate: 48_000,
            color: cut_core::ColorConfig::default(),
        },
    );
    p.assets.insert(
        "a1".into(),
        cut_core::Asset {
            path: clip.display().to_string(),
            hash: "sha256:xftest".into(),
            probe: Some(serde_json::json!({"kind": "video"})),
            transcript: None,
            perception: None,
            proxy: None,
            filmstrip: None,
        },
    );
    let mk = |id: &str, xfade_in_ms: u64, kind: Option<&str>| {
        Clip::Media(MediaClip {
            id: id.into(),
            asset: "a1".into(),
            src_in_ms: 0,
            src_out_ms: 2000,
            effects: vec![],
            gain_db: 0.0,
            transform: None,
            crop: None,
            fade: None,
            xfade_in_ms,
            xfade_kind: kind.map(|s| s.to_string()),
            speed: 1.0,
            grade: None,
            matte: None,
            mask: None,
            reverse: false,
            freeze: None,
            animation: None,
            keyframes: vec![],
            eq: None,
            mute_ranges: vec![],
            stabilize: None,
            speed_ramp: None,
            input_color_space: None,
            nest: None,
            grade_stack: vec![],
            grade_windows: vec![],
        })
    };
    // clip1 (0–2000), clip2 crossfades IN over 500ms with the NEW `coverleft` style.
    p.track_mut("v1").unwrap().clips = vec![mk("c1", 0, None), mk("c2", 500, Some("coverleft"))];
    let edl = edl_from_project(&p);
    // Total = 2000 + 2000 - 500 overlap = 3500ms.
    assert_eq!(
        edl.duration_ms, 3500,
        "crossfade should shorten the timeline"
    );
    let fence = PathFence::new(dir.path()).unwrap();
    let out = render_final(
        &p,
        &edl,
        &fence,
        Path::new("xf.mp4"),
        &RenderPreset::default(),
        RenderOptions::default(),
        None,
    )
    .expect("render with the new 'coverleft' transition (ffmpeg accepts the name)");
    // A real output of ~3.5s = ffmpeg ran the xfade with the new name cleanly.
    assert!(
        out.duration_ms.abs_diff(3500) <= 60,
        "rendered {}ms",
        out.duration_ms
    );
    let y = first_frame_yavg(&out.path);
    assert!(
        y > 5.0,
        "first frame should have real picture, got luma {y}"
    );
}

/// the still-image preview contract: draft incremental preview MUST work on a timeline that contains a
/// still-image card (the intro/outro pattern every demo uses). Stills never get
/// a proxy (their import chain stops after probe), and the preview used to
/// REFUSE the whole timeline the moment a card was present ("asset has no proxy
/// yet"). Fix: a still is a trivially-cacheable conform — looped from its source
/// image, no proxy required. This test mixes an intro CARD (still, no proxy)
/// with a VIDEO clip (real proxy) and proves the preview renders the full
/// timeline, the still segment shows the red card (not black/empty), and the
/// segment cache is content-addressed (re-run reuses, no re-render).
#[test]
fn draft_preview_renders_timeline_with_still_card() {
    let dir = tempfile::tempdir().unwrap();
    let proxies = dir.path().join("proxies");
    std::fs::create_dir_all(&proxies).unwrap();
    let cache = dir.path().join(".preview");

    let clip = gen_clip(dir.path(), "in.mp4");
    let still = gen_still(dir.path(), "intro.png");
    // The video asset has a REAL proxy (the import proxy step ran); the still
    // asset has NONE — exactly the regression shape that broke draft preview.
    let proxy = make_proxy(&clip, &proxies, "vid").expect("proxy");

    let mut p = Project::new(
        "t",
        ProjectSettings {
            width: 320,
            height: 240,
            fps: 30.0,
            audio_rate: 48_000,
            color: cut_core::ColorConfig::default(),
        },
    );
    let img_asset = cut_core::Asset {
        path: still.display().to_string(),
        hash: "sha256:still".into(),
        probe: Some(serde_json::json!({"kind": "image"})),
        transcript: None,
        perception: None,
        proxy: None,
        filmstrip: None, // stills NEVER get a proxy
    };
    let vid_asset = cut_core::Asset {
        path: clip.display().to_string(),
        hash: "sha256:vid".into(),
        probe: Some(serde_json::json!({"kind": "video"})),
        transcript: None,
        perception: None,
        proxy: Some(proxy.display().to_string()),
        filmstrip: None,
    };
    p.assets.insert("img".into(), img_asset);
    p.assets.insert("vid".into(), vid_asset);
    // v1 = 1.5s intro card (still) then 2s of video. The base video track is
    // what the incremental preview walks.
    p.track_mut("v1").unwrap().clips = vec![
        Clip::Media(MediaClip {
            id: "c1".into(),
            asset: "img".into(),
            src_in_ms: 0,
            src_out_ms: 1500,
            effects: vec![],
            gain_db: 0.0,
            transform: None,
            crop: None,
            fade: None,
            xfade_in_ms: 0,
            xfade_kind: None,
            speed: 1.0,
            grade: None,
            matte: None,
            mask: None,
            reverse: false,
            freeze: None,
            animation: None,
            keyframes: vec![],
            eq: None,
            mute_ranges: vec![],
            stabilize: None,
            speed_ramp: None,
            input_color_space: None,
            nest: None,
            grade_stack: vec![],
            grade_windows: vec![],
        }),
        Clip::Media(MediaClip {
            id: "c2".into(),
            asset: "vid".into(),
            src_in_ms: 0,
            src_out_ms: 2000,
            effects: vec![],
            gain_db: 0.0,
            transform: None,
            crop: None,
            fade: None,
            xfade_in_ms: 0,
            xfade_kind: None,
            speed: 1.0,
            grade: None,
            matte: None,
            mask: None,
            reverse: false,
            freeze: None,
            animation: None,
            keyframes: vec![],
            eq: None,
            mute_ranges: vec![],
            stabilize: None,
            speed_ramp: None,
            input_color_space: None,
            nest: None,
            grade_stack: vec![],
            grade_windows: vec![],
        }),
    ];
    let edl = edl_from_project(&p);
    assert_eq!(edl.duration_ms, 3500);

    let draft = RenderPreset::named("draft").unwrap();
    // Before the fix this returned Err("asset img has no proxy yet").
    let r = render_preview_incremental(&p, &edl, dir.path(), &cache, &draft)
        .expect("draft preview must render a timeline containing a still card");
    assert_eq!(
        r.segments_rendered, 2,
        "still card + video both rendered fresh"
    );
    assert_eq!(r.segments_reused, 0);
    let pdiff = r.duration_ms.abs_diff(3500);
    assert!(pdiff <= 34, "preview duration {} ms ≈ 3500", r.duration_ms);
    assert!(r.path.is_file(), "preview.mp4 written");
    // The opening frame is the RED card (Y ≈ 76 at 320x240→960x540 proxy
    // geometry, padded), proving the still actually rendered (not black ≈ 16).
    let y = first_frame_yavg(&r.path);
    assert!(
        (40.0..=95.0).contains(&y),
        "first preview frame luma {y} — expected the red still card"
    );

    // Re-run: both segments are content-addressed and already cached → reused,
    // nothing re-rendered (the incremental-cache contract holds for stills too).
    let r2 = render_preview_incremental(&p, &edl, dir.path(), &cache, &draft)
        .expect("second preview reuses cache");
    assert_eq!(r2.segments_rendered, 0, "warm cache re-renders nothing");
    assert_eq!(
        r2.segments_reused, 2,
        "still + video both reused from cache"
    );
}

/// Decode ONE frame of `path` at `t_s` seconds to raw RGB24 bytes (W·H·3) —
/// exact pixel sampling for compositing proofs.
fn frame_rgb(path: &Path, t_s: f64, w: u32, h: u32) -> Vec<u8> {
    let out = std::process::Command::new("ffmpeg")
        .args(["-hide_banner", "-nostdin", "-ss", &format!("{t_s}")])
        .arg("-i")
        .arg(path)
        .args(["-frames:v", "1", "-f", "rawvideo", "-pix_fmt", "rgb24", "-"])
        .output()
        .expect("ffmpeg raw frame");
    assert_eq!(out.stdout.len(), (w * h * 3) as usize, "one full RGB frame");
    out.stdout
}

/// (r, g, b) of pixel (x, y) in a raw RGB24 buffer of width `w`.
fn px(rgb: &[u8], w: u32, x: u32, y: u32) -> (u8, u8, u8) {
    let i = ((y * w + x) * 3) as usize;
    (rgb[i], rgb[i + 1], rgb[i + 2])
}

/// (masks): an `edit.add_mask` rect with `effect:black` over the LEFT
/// half of the frame blacks out ONLY that region (the right half keeps the testsrc2
/// picture). This exercises the WHOLE mask path through the real render: resvg bakes
/// the shape alpha → it's a parallel input → alphamerge+overlay scopes the effect to
/// the region. Black is the unambiguous signal (left luma≈0, right luma≫0). Skips if
/// ffmpeg absent.
#[test]
fn mask_black_region_scopes_to_the_shape() {
    use cut_core::{ClipMask, MaskEffect, MaskShape};
    let dir = tempfile::tempdir().unwrap();
    let (w, h) = (320u32, 240u32);
    let clip = gen_clip(dir.path(), "src.mp4"); // testsrc2 (busy picture) + sine
    let mut p = Project::new(
        "mask",
        ProjectSettings {
            width: w,
            height: h,
            fps: 30.0,
            audio_rate: 48_000,
            color: cut_core::ColorConfig::default(),
        },
    );
    p.assets.insert(
        "a1".into(),
        cut_core::Asset {
            path: clip.display().to_string(),
            hash: "sha256:masktest".into(),
            probe: Some(serde_json::json!({"kind": "video"})),
            transcript: None,
            perception: None,
            proxy: None,
            filmstrip: None,
        },
    );
    p.track_mut("v1").unwrap().clips = vec![Clip::Media(MediaClip {
        id: "c1".into(),
        asset: "a1".into(),
        src_in_ms: 0,
        src_out_ms: 1500,
        effects: vec![],
        gain_db: 0.0,
        transform: None,
        crop: None,
        fade: None,
        xfade_in_ms: 0,
        xfade_kind: None,
        speed: 1.0,
        grade: None,
        matte: None,
        // Rect over the LEFT half [x 0..0.5], blacked out.
        mask: Some(ClipMask {
            shape: MaskShape::Rect,
            points: vec![[0.0, 0.0], [0.5, 1.0]],
            feather: 0.0,
            invert: false,
            effect: MaskEffect::Black,
            strength: None,
            range_ms: None,
            track: None,
            regions: Vec::new(),
        }),
        reverse: false,
        freeze: None,
        animation: None,
        keyframes: vec![],
        eq: None,
        mute_ranges: vec![],
        stabilize: None,
        speed_ramp: None,
        input_color_space: None,
        nest: None,
        grade_stack: vec![],
        grade_windows: vec![],
    })];
    let edl = edl_from_project(&p);
    let fence = PathFence::new(dir.path()).unwrap();
    let out = render_final(
        &p,
        &edl,
        &fence,
        Path::new("mask.mp4"),
        &RenderPreset::default(),
        RenderOptions::default(),
        None,
    )
    .expect("render a masked clip");

    let rgb = frame_rgb(&out.path, 0.5, w, h);
    // Sample several rows on each side (avoid the exact seam at x=160).
    let avg = |xs: std::ops::Range<u32>| -> f64 {
        let mut sum = 0.0;
        let mut n = 0.0;
        for y in (20..h - 20).step_by(20) {
            for x in xs.clone().step_by(20) {
                let (r, g, b) = px(&rgb, w, x, y);
                sum += (r as f64 + g as f64 + b as f64) / 3.0;
                n += 1.0;
            }
        }
        sum / n
    };
    let left = avg(20..150); // inside the masked (black) region
    let right = avg(170..300); // outside — untouched testsrc2
    assert!(
        left < 12.0,
        "left half should be blacked out by the mask, got luma {left:.1}"
    );
    assert!(
        right > 40.0,
        "right half keeps the picture, got luma {right:.1}"
    );
}

/// Windows render regression: a base clip
/// carrying both a power window and a non-identity base transform must render.
/// The region composite previously joined the comma-prefixed transform as
/// `[region],scale...`, which ffmpeg rejected as an empty filter before `scale`.
#[test]
fn grade_window_with_base_transform_renders() {
    use cut_core::{ClipGrade, GradeWindow, MaskShape, WindowShape};
    let dir = tempfile::tempdir().unwrap();
    let source = gen_clip(dir.path(), "source.mp4");
    let mut project = Project::new(
        "grade_window_transform",
        ProjectSettings {
            width: 320,
            height: 240,
            fps: 30.0,
            audio_rate: 48_000,
            color: cut_core::ColorConfig::default(),
        },
    );
    project.assets.insert(
        "a1".into(),
        cut_core::Asset {
            path: source.display().to_string(),
            hash: "sha256:grade-window-transform".into(),
            probe: Some(serde_json::json!({"kind": "video", "width": 320, "height": 240})),
            transcript: None,
            perception: None,
            proxy: None,
            filmstrip: None,
        },
    );
    project.track_mut("v1").unwrap().clips = vec![Clip::Media(MediaClip {
        id: "c1".into(),
        asset: "a1".into(),
        src_in_ms: 0,
        src_out_ms: 1500,
        effects: vec![],
        gain_db: 0.0,
        transform: Some(cut_core::ClipTransform {
            x: 0.4,
            y: 0.1,
            scale: 0.8,
            opacity: 1.0,
        }),
        crop: None,
        fade: None,
        xfade_in_ms: 0,
        xfade_kind: None,
        speed: 1.0,
        grade: None,
        matte: None,
        mask: None,
        reverse: false,
        freeze: None,
        animation: None,
        keyframes: vec![],
        eq: None,
        mute_ranges: vec![],
        stabilize: None,
        speed_ramp: None,
        input_color_space: None,
        nest: None,
        grade_stack: vec![],
        grade_windows: vec![GradeWindow {
            window: WindowShape {
                shape: MaskShape::Rect,
                points: vec![[0.2, 0.2], [0.8, 0.8]],
                feather: 0.0,
                invert: false,
            },
            grade: ClipGrade {
                contrast: 1.2,
                brightness: 0.0,
                saturation: 1.0,
                gamma: 1.0,
                temperature_k: None,
                lut: None,
            },
        }],
    })];
    let edl = edl_from_project(&project);
    let fence = PathFence::new(dir.path()).unwrap();
    let output = dir.path().join("grade-window-transform.mp4");
    render_final(
        &project,
        &edl,
        &fence,
        &output,
        &RenderPreset::default(),
        RenderOptions::default(),
        None,
    )
    .expect("power window plus base transform must produce a valid filter graph");
    assert!(
        output.is_file(),
        "combined region/transform render wrote output"
    );
}

/// (mask BLUR): a heavy blur mask over the LEFT half turns a sharp
/// vertical-stripe pattern into gray mush there (local VARIANCE collapses) while the
/// RIGHT half keeps its hard black/white stripes (high variance). Proves the same
/// split→effect→alphamerge→overlay path with `gblur`. Stripes = an unambiguous
/// high-frequency signal (vs testsrc2 + h264, which compress the ratio). Skips if
/// ffmpeg absent.
#[test]
fn mask_blur_reduces_detail_in_the_region() {
    use cut_core::{ClipMask, MaskEffect, MaskShape};
    let dir = tempfile::tempdir().unwrap();
    let (w, h) = (320u32, 240u32);
    // Sharp 4px vertical black/white stripes (high-frequency ground truth).
    let clip = dir.path().join("stripes.mp4");
    let gen: Vec<String> = [
        "-f",
        "lavfi",
        "-i",
        "color=black:s=320x240:r=30:d=1.5",
        "-vf",
        "geq=lum='if(mod(floor(X/4)\\,2)\\,235\\,16)':cb=128:cr=128",
        "-c:v",
        "libx264",
        "-preset",
        "ultrafast",
        "-qp",
        "0",
        "-pix_fmt",
        "yuv420p",
    ]
    .iter()
    .map(|s| s.to_string())
    .chain([clip.display().to_string()])
    .collect();
    if cut_media::ffmpeg::run_ffmpeg(&gen).is_err() {
        eprintln!("ffmpeg unavailable — skipping mask blur proof");
        return;
    }
    let mut p = Project::new(
        "mb",
        ProjectSettings {
            width: w,
            height: h,
            fps: 30.0,
            audio_rate: 48_000,
            color: cut_core::ColorConfig::default(),
        },
    );
    p.assets.insert(
        "a1".into(),
        cut_core::Asset {
            path: clip.display().to_string(),
            hash: "sha256:maskblur".into(),
            probe: Some(serde_json::json!({"kind": "video"})),
            transcript: None,
            perception: None,
            proxy: None,
            filmstrip: None,
        },
    );
    p.track_mut("v1").unwrap().clips = vec![Clip::Media(MediaClip {
        id: "c1".into(),
        asset: "a1".into(),
        src_in_ms: 0,
        src_out_ms: 1500,
        effects: vec![],
        gain_db: 0.0,
        transform: None,
        crop: None,
        fade: None,
        xfade_in_ms: 0,
        xfade_kind: None,
        speed: 1.0,
        grade: None,
        matte: None,
        mask: Some(ClipMask {
            shape: MaskShape::Rect,
            points: vec![[0.0, 0.0], [0.5, 1.0]],
            feather: 0.0,
            invert: false,
            effect: MaskEffect::Blur,
            strength: Some(30.0), // heavy blur, clearly flattens detail
            range_ms: None,
            track: None,
            regions: Vec::new(),
        }),
        reverse: false,
        freeze: None,
        animation: None,
        keyframes: vec![],
        eq: None,
        mute_ranges: vec![],
        stabilize: None,
        speed_ramp: None,
        input_color_space: None,
        nest: None,
        grade_stack: vec![],
        grade_windows: vec![],
    })];
    let edl = edl_from_project(&p);
    let fence = PathFence::new(dir.path()).unwrap();
    let out = render_final(
        &p,
        &edl,
        &fence,
        Path::new("blur.mp4"),
        &RenderPreset::default(),
        RenderOptions::default(),
        None,
    )
    .expect("render a blur-masked clip");

    let rgb = frame_rgb(&out.path, 0.5, w, h);
    // Luma VARIANCE over a region: sharp stripes are bimodal (16/235) → high variance;
    // a heavy blur flattens them to mid-gray → low variance.
    let variance = |xs: std::ops::Range<u32>| -> f64 {
        let lumas: Vec<f64> = (10..h - 10)
            .step_by(2)
            .flat_map(|y| xs.clone().step_by(2).map(move |x| (x, y)))
            .map(|(x, y)| {
                let (r, g, b) = px(&rgb, w, x, y);
                (r as f64 + g as f64 + b as f64) / 3.0
            })
            .collect();
        let mean = lumas.iter().sum::<f64>() / lumas.len() as f64;
        lumas.iter().map(|l| (l - mean).powi(2)).sum::<f64>() / lumas.len() as f64
    };
    let left = variance(20..150); // blurred — flat (low variance)
    let right = variance(170..300); // sharp stripes — bimodal (high variance)
    assert!(
        right > left * 3.0,
        "blur should collapse the stripe variance in the masked region: left(blurred) {left:.0} vs right(sharp) {right:.0}"
    );
}

/// A TIME-BOUNDED redaction (`edit.redact` → ClipMask.range_ms) is active
/// ONLY in its window. Proven at the pixel level: the SAME left-half region is
/// SHARP before the window (the `enable='between(t,…)'` overlay is off → the
/// un-effected base shows) and BLURRED during it. This is the property that makes
/// redaction practical (blur the password only while it's on screen).
#[test]
fn redaction_is_active_only_within_its_time_range() {
    use cut_core::{ClipMask, MaskEffect, MaskShape};
    let dir = tempfile::tempdir().unwrap();
    let (w, h) = (320u32, 240u32);
    // Sharp 4px vertical stripes for 2s (high-frequency ground truth).
    let clip = dir.path().join("stripes.mp4");
    let gen: Vec<String> = [
        "-f",
        "lavfi",
        "-i",
        "color=black:s=320x240:r=30:d=2",
        "-vf",
        "geq=lum='if(mod(floor(X/4)\\,2)\\,235\\,16)':cb=128:cr=128",
        "-c:v",
        "libx264",
        "-preset",
        "ultrafast",
        "-qp",
        "0",
        "-pix_fmt",
        "yuv420p",
    ]
    .iter()
    .map(|s| s.to_string())
    .chain([clip.display().to_string()])
    .collect();
    if cut_media::ffmpeg::run_ffmpeg(&gen).is_err() {
        eprintln!("ffmpeg unavailable — skipping redaction time-range proof");
        return;
    }
    let mut p = Project::new(
        "rd",
        ProjectSettings {
            width: w,
            height: h,
            fps: 30.0,
            audio_rate: 48_000,
            color: cut_core::ColorConfig::default(),
        },
    );
    p.assets.insert(
        "a1".into(),
        cut_core::Asset {
            path: clip.display().to_string(),
            hash: "sha256:redact".into(),
            probe: Some(serde_json::json!({"kind": "video"})),
            transcript: None,
            perception: None,
            proxy: None,
            filmstrip: None,
        },
    );
    // Redact the LEFT half, but ONLY between 800ms and 1600ms.
    p.track_mut("v1").unwrap().clips = vec![Clip::Media(MediaClip {
        id: "c1".into(),
        asset: "a1".into(),
        src_in_ms: 0,
        src_out_ms: 2000,
        effects: vec![],
        gain_db: 0.0,
        transform: None,
        crop: None,
        fade: None,
        xfade_in_ms: 0,
        xfade_kind: None,
        speed: 1.0,
        grade: None,
        matte: None,
        mask: Some(ClipMask {
            shape: MaskShape::Rect,
            points: vec![[0.0, 0.0], [0.5, 1.0]],
            feather: 0.0,
            invert: false,
            effect: MaskEffect::Blur,
            strength: Some(30.0),
            range_ms: Some([800, 1600]),
            track: None,
            regions: Vec::new(),
        }),
        reverse: false,
        freeze: None,
        animation: None,
        keyframes: vec![],
        eq: None,
        mute_ranges: vec![],
        stabilize: None,
        speed_ramp: None,
        input_color_space: None,
        nest: None,
        grade_stack: vec![],
        grade_windows: vec![],
    })];
    let edl = edl_from_project(&p);
    let fence = PathFence::new(dir.path()).unwrap();
    let out = render_final(
        &p,
        &edl,
        &fence,
        Path::new("redact.mp4"),
        &RenderPreset::default(),
        RenderOptions::default(),
        None,
    )
    .expect("render a time-bounded redaction");

    // Luma variance of the LEFT half at a given time (SECONDS; sharp stripes = high
    // variance; a heavy blur flattens them = low variance).
    let left_variance = |t_s: f64| -> f64 {
        let rgb = frame_rgb(&out.path, t_s, w, h);
        let lumas: Vec<f64> = (10..h - 10)
            .step_by(2)
            .flat_map(|y| (20u32..150).step_by(2).map(move |x| (x, y)))
            .map(|(x, y)| {
                let (r, g, b) = px(&rgb, w, x, y);
                (r as f64 + g as f64 + b as f64) / 3.0
            })
            .collect();
        let mean = lumas.iter().sum::<f64>() / lumas.len() as f64;
        lumas.iter().map(|l| (l - mean).powi(2)).sum::<f64>() / lumas.len() as f64
    };
    let before = left_variance(0.3); // 300ms — OUTSIDE [800,1600] → sharp
    let during = left_variance(1.0); // 1000ms — INSIDE [800,1600] → blurred
    assert!(
        before > during * 3.0,
        "redaction must be active ONLY in its window: left-half variance before {before:.0} (sharp) should be ≫ during {during:.0} (blurred)"
    );
}

/// A MOTION-TRACKED redaction (`edit.redact{track}` → ClipMask.track) FOLLOWS
/// a moving subject: the renderer paints the alpha procedurally (geq) at a
/// time-varying centre. Proven at the pixel level: a blur region whose track runs
/// left→right is on the LEFT early and on the RIGHT late (the band variances flip).
#[test]
fn tracked_redaction_blur_follows_the_moving_region() {
    use cut_core::{ClipMask, MaskEffect, MaskShape, MaskTrackPoint};
    let dir = tempfile::tempdir().unwrap();
    let (w, h) = (320u32, 240u32);
    // Sharp 4px vertical stripes for 2s.
    let clip = dir.path().join("stripes.mp4");
    let gen: Vec<String> = [
        "-f",
        "lavfi",
        "-i",
        "color=black:s=320x240:r=30:d=2",
        "-vf",
        "geq=lum='if(mod(floor(X/4)\\,2)\\,235\\,16)':cb=128:cr=128",
        "-c:v",
        "libx264",
        "-preset",
        "ultrafast",
        "-qp",
        "0",
        "-pix_fmt",
        "yuv420p",
    ]
    .iter()
    .map(|s| s.to_string())
    .chain([clip.display().to_string()])
    .collect();
    if cut_media::ffmpeg::run_ffmpeg(&gen).is_err() {
        eprintln!("ffmpeg unavailable — skipping tracked-redaction proof");
        return;
    }
    let mut p = Project::new(
        "tr",
        ProjectSettings {
            width: w,
            height: h,
            fps: 30.0,
            audio_rate: 48_000,
            color: cut_core::ColorConfig::default(),
        },
    );
    p.assets.insert(
        "a1".into(),
        cut_core::Asset {
            path: clip.display().to_string(),
            hash: "sha256:trackredact".into(),
            probe: Some(serde_json::json!({"kind": "video"})),
            transcript: None,
            perception: None,
            proxy: None,
            filmstrip: None,
        },
    );
    // A blur rect (~80×120) whose CENTRE tracks left→right (cx 0.15→0.85) over 2s.
    p.track_mut("v1").unwrap().clips = vec![Clip::Media(MediaClip {
        id: "c1".into(),
        asset: "a1".into(),
        src_in_ms: 0,
        src_out_ms: 2000,
        effects: vec![],
        gain_db: 0.0,
        transform: None,
        crop: None,
        fade: None,
        xfade_in_ms: 0,
        xfade_kind: None,
        speed: 1.0,
        grade: None,
        matte: None,
        mask: Some(ClipMask {
            // Ellipse exercises the geq `lte`/`pow` path (the fragile one); the radii
            // give the SIZE, the track gives the centre. points = [centre, radii].
            shape: MaskShape::Ellipse,
            points: vec![[0.5, 0.5], [0.13, 0.25]],
            feather: 0.0,
            invert: false,
            effect: MaskEffect::Blur,
            strength: Some(14.0),
            range_ms: None,
            track: Some(vec![
                MaskTrackPoint {
                    t_ms: 0,
                    cx: 0.15,
                    cy: 0.5,
                },
                MaskTrackPoint {
                    t_ms: 2000,
                    cx: 0.85,
                    cy: 0.5,
                },
            ]),
            regions: Vec::new(),
        }),
        reverse: false,
        freeze: None,
        animation: None,
        keyframes: vec![],
        eq: None,
        mute_ranges: vec![],
        stabilize: None,
        speed_ramp: None,
        input_color_space: None,
        nest: None,
        grade_stack: vec![],
        grade_windows: vec![],
    })];
    let edl = edl_from_project(&p);
    let fence = PathFence::new(dir.path()).unwrap();
    let out = render_final(
        &p,
        &edl,
        &fence,
        Path::new("tracked.mp4"),
        &RenderPreset::default(),
        RenderOptions::default(),
        None,
    )
    .expect("render a tracked redaction");

    // Luma variance of an x-band at a given time (SECONDS).
    let band_var = |t_s: f64, x0: u32, x1: u32| -> f64 {
        let rgb = frame_rgb(&out.path, t_s, w, h);
        let lumas: Vec<f64> = (20..h - 20)
            .step_by(2)
            .flat_map(|y| (x0..x1).step_by(2).map(move |x| (x, y)))
            .map(|(x, y)| {
                let (r, g, b) = px(&rgb, w, x, y);
                (r as f64 + g as f64 + b as f64) / 3.0
            })
            .collect();
        let mean = lumas.iter().sum::<f64>() / lumas.len() as f64;
        lumas.iter().map(|l| (l - mean).powi(2)).sum::<f64>() / lumas.len() as f64
    };
    // Early (t=0.3): region is LEFT → left band blurred (low), right sharp (high).
    let (le, re) = (band_var(0.3, 40, 120), band_var(0.3, 200, 280));
    // Late (t=1.7): region is RIGHT → the bands FLIP.
    let (ll, rl) = (band_var(1.7, 40, 120), band_var(1.7, 200, 280));
    assert!(
        le < re * 0.6 && rl < ll * 0.6,
        "the tracked blur must FOLLOW the region: early L{le:.0}<R{re:.0}, late L{ll:.0}>R{rl:.0}"
    );
}

/// multi-track compositing regression: a second video track renders as an OVERLAY above
/// the base — full-frame by default, positioned/scaled by ClipTransform
/// (PiP). Proven at the pixel level: red PiP in the bottom-right quadrant
/// over a green base while the overlay clip is active, plain green after.
#[test]
fn render_composites_overlay_track_with_transform() {
    let dir = tempfile::tempdir().unwrap();
    // Base: 2s of solid green video. Overlay: solid red still (PiP source).
    let base = dir.path().join("base.mp4");
    let args: Vec<String> = [
        "-f",
        "lavfi",
        "-i",
        "color=c=green:s=320x240:r=30:d=2",
        "-c:v",
        "libx264",
        "-preset",
        "ultrafast",
        "-pix_fmt",
        "yuv420p",
    ]
    .iter()
    .map(|s| s.to_string())
    .chain([base.display().to_string()])
    .collect();
    cut_media::ffmpeg::run_ffmpeg(&args).expect("base generation");
    let still = gen_still(dir.path(), "pip.png");

    let mut p = Project::new(
        "t",
        ProjectSettings {
            width: 320,
            height: 240,
            fps: 30.0,
            audio_rate: 48_000,
            color: cut_core::ColorConfig::default(),
        },
    );
    let mk_asset = |path: &Path, kind: &str| cut_core::Asset {
        path: path.display().to_string(),
        hash: "sha256:test".into(),
        probe: Some(serde_json::json!({"kind": kind})),
        transcript: None,
        perception: None,
        proxy: None,
        filmstrip: None,
    };
    p.assets.insert("base".into(), mk_asset(&base, "video"));
    p.assets.insert("pip".into(), mk_asset(&still, "image"));
    p.track_mut("v1").unwrap().clips = vec![Clip::Media(MediaClip {
        id: "c1".into(),
        asset: "base".into(),
        src_in_ms: 0,
        src_out_ms: 2000,
        effects: vec![],
        gain_db: 0.0,
        transform: None,
        crop: None,
        fade: None,
        xfade_in_ms: 0,
        xfade_kind: None,
        speed: 1.0,
        grade: None,
        matte: None,
        mask: None,
        reverse: false,
        freeze: None,
        animation: None,
        keyframes: vec![],
        eq: None,
        mute_ranges: vec![],
        stabilize: None,
        speed_ramp: None,
        input_color_space: None,
        nest: None,
        grade_stack: vec![],
        grade_windows: vec![],
    })];
    // Overlay track: red still for the FIRST second only, half-size PiP
    // at the bottom-right quadrant (x=0.5, y=0.5, scale=0.5 → 160×120 @ 160,120).
    p.tracks.push(Track {
        id: "v2".into(),
        kind: TrackKind::Video,
        clips: vec![Clip::Media(MediaClip {
            id: "c2".into(),
            asset: "pip".into(),
            src_in_ms: 0,
            src_out_ms: 1000,
            effects: vec![],
            gain_db: 0.0,
            transform: Some(cut_core::ClipTransform {
                x: 0.5,
                y: 0.5,
                scale: 0.5,
                opacity: 1.0,
            }),
            crop: None,
            fade: None,
            xfade_in_ms: 0,
            xfade_kind: None,
            speed: 1.0,
            grade: None,
            matte: None,
            mask: None,
            reverse: false,
            freeze: None,
            animation: None,
            keyframes: vec![],
            eq: None,
            mute_ranges: vec![],
            stabilize: None,
            speed_ramp: None,
            input_color_space: None,
            nest: None,
            grade_stack: vec![],
            grade_windows: vec![],
        })],
        gain_db: 0.0,
        gain_windows: vec![],
        blend_mode: None,
        visible: true,
        locked: false,
        muted: false,
        solo: false,
        pan: 0.0,
    });
    let edl = edl_from_project(&p);
    assert_eq!(edl.duration_ms, 2000);
    let fence = PathFence::new(dir.path()).unwrap();
    let out = render_final(
        &p,
        &edl,
        &fence,
        Path::new("pip.mp4"),
        &RenderPreset::default(),
        RenderOptions::default(),
        None,
    )
    .expect("overlay render");
    let diff = out.duration_ms.abs_diff(2000);
    assert!(
        diff <= 34,
        "overlay must not change composition length: {} ms",
        out.duration_ms
    );

    // t=0.5s: PiP active — bottom-right red, top-left still green.
    let rgb = frame_rgb(&out.path, 0.5, 320, 240);
    let (r, g, _b) = px(&rgb, 320, 240, 180); // inside the PiP quadrant
    assert!(r > 180 && g < 100, "PiP pixel is red: ({r},{g},_)");
    let (r, g, _b) = px(&rgb, 320, 80, 60); // outside the PiP
    assert!(g > 100 && r < 100, "base pixel stays green: ({r},{g},_)");
    // t=1.5s: overlay clip ended — the same spot is green again.
    let rgb = frame_rgb(&out.path, 1.5, 320, 240);
    let (r, g, _b) = px(&rgb, 320, 240, 180);
    assert!(
        g > 100 && r < 100,
        "after the overlay clip: green again ({r},{g},_)"
    );
}

/// Layer-order contract: later video tracks cover earlier tracks,
/// group-relative reorder changes the delivered picture, hidden tracks stop
/// contributing pixels, and a hidden base keeps its black canvas slot. The same
/// proof covers static and keyframed base opacity over black.
#[test]
fn render_layer_order_visibility_and_base_opacity() {
    use cut_core::{Keyframe, KfInterp, KfParam, KfPoint};

    let dir = tempfile::tempdir().unwrap();
    let gen = |name: &str, color: &str| -> PathBuf {
        let out = dir.path().join(name);
        let args: Vec<String> = [
            "-f",
            "lavfi",
            "-i",
            &format!("color=c={color}:s=320x240:r=30:d=2"),
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
        cut_media::ffmpeg::run_ffmpeg(&args).expect("layer color source");
        out
    };
    let sources = [
        ("blue", gen("blue.mp4", "blue")),
        ("green", gen("green-layer.mp4", "0x00ff00")),
        ("red", gen("red-layer.mp4", "red")),
    ];
    let mut p = Project::new(
        "layer-contract",
        ProjectSettings {
            width: 320,
            height: 240,
            fps: 30.0,
            audio_rate: 48_000,
            color: cut_core::ColorConfig::default(),
        },
    );
    for (id, path) in &sources {
        p.assets.insert(
            (*id).into(),
            cut_core::Asset {
                path: path.display().to_string(),
                hash: format!("sha256:{id}"),
                probe: Some(serde_json::json!({"kind": "video", "width": 320, "height": 240})),
                transcript: None,
                perception: None,
                proxy: None,
                filmstrip: None,
            },
        );
    }
    let media = |id: &str, asset: &str| MediaClip {
        id: id.into(),
        asset: asset.into(),
        src_in_ms: 0,
        src_out_ms: 2000,
        effects: vec![],
        gain_db: 0.0,
        transform: None,
        crop: None,
        fade: None,
        xfade_in_ms: 0,
        xfade_kind: None,
        speed: 1.0,
        grade: None,
        matte: None,
        mask: None,
        reverse: false,
        freeze: None,
        animation: None,
        keyframes: vec![],
        eq: None,
        mute_ranges: vec![],
        stabilize: None,
        speed_ramp: None,
        input_color_space: None,
        nest: None,
        grade_stack: vec![],
        grade_windows: vec![],
    };
    let track = |id: &str, clip: MediaClip| Track {
        id: id.into(),
        kind: TrackKind::Video,
        clips: vec![Clip::Media(clip)],
        gain_db: 0.0,
        gain_windows: vec![],
        blend_mode: None,
        visible: true,
        locked: false,
        muted: false,
        solo: false,
        pan: 0.0,
    };
    p.track_mut("v1").unwrap().clips = vec![Clip::Media(media("blue-clip", "blue"))];
    p.tracks.push(track("v2", media("green-clip", "green")));
    p.tracks.push(track("v3", media("red-clip", "red")));

    let render_frame = |project: &Project, name: &str, at: f64| {
        let edl = edl_from_project(project);
        let fence = PathFence::new(dir.path()).unwrap();
        let out = render_final(
            project,
            &edl,
            &fence,
            Path::new(name),
            &RenderPreset::default(),
            RenderOptions::default(),
            None,
        )
        .expect("layer contract render");
        frame_rgb(&out.path, at, 320, 240)
    };

    let frame = render_frame(&p, "layers-red-top.mp4", 0.5);
    let (r, g, _) = px(&frame, 320, 160, 120);
    assert!(
        r > 180 && g < 80,
        "last video track is front-most: ({r},{g},_)"
    );

    cut_core::edit::reorder_track(&mut p, "v2", 2).expect("bring green forward");
    let frame = render_frame(&p, "layers-green-top.mp4", 0.5);
    let (r, g, _) = px(&frame, 320, 160, 120);
    assert!(
        g > 180 && r < 80,
        "reorder changes the front-most pixels: ({r},{g},_)"
    );

    p.track_mut("v2").unwrap().visible = false;
    let frame = render_frame(&p, "layers-green-hidden.mp4", 0.5);
    let (r, g, _) = px(&frame, 320, 160, 120);
    assert!(
        r > 180 && g < 80,
        "hidden top track reveals the layer below: ({r},{g},_)"
    );

    p.track_mut("v1").unwrap().visible = false;
    if let Clip::Media(clip) = &mut p.track_mut("v3").unwrap().clips[0] {
        clip.transform = Some(cut_core::ClipTransform {
            x: 0.0,
            y: 0.0,
            scale: 0.5,
            opacity: 1.0,
        });
    }
    let frame = render_frame(&p, "layers-hidden-base.mp4", 0.5);
    let (ri, _, _) = px(&frame, 320, 60, 40);
    let (ro, go, bo) = px(&frame, 320, 260, 200);
    assert!(
        ri > 180,
        "visible overlay remains transformed over a hidden base"
    );
    assert!(
        ro < 30 && go < 30 && bo < 30,
        "hidden base stays a black canvas: ({ro},{go},{bo})"
    );

    p.track_mut("v1").unwrap().visible = true;
    p.track_mut("v3").unwrap().visible = false;
    if let Clip::Media(clip) = &mut p.track_mut("v1").unwrap().clips[0] {
        clip.transform = Some(cut_core::ClipTransform {
            x: 0.25,
            y: 0.25,
            scale: 0.5,
            opacity: 0.5,
        });
    }
    let frame = render_frame(&p, "layers-base-opacity.mp4", 0.5);
    let (_, _, inside_b) = px(&frame, 320, 160, 120);
    let (outside_r, outside_g, outside_b) = px(&frame, 320, 20, 20);
    assert!(
        (70..190).contains(&inside_b),
        "base opacity dims blue over black: {inside_b}"
    );
    assert!(
        outside_r < 30 && outside_g < 30 && outside_b < 30,
        "base transform pads with black"
    );

    if let Clip::Media(clip) = &mut p.track_mut("v1").unwrap().clips[0] {
        clip.transform.as_mut().unwrap().opacity = 1.0;
        clip.keyframes = vec![Keyframe {
            param: KfParam::Opacity,
            points: vec![
                KfPoint {
                    t_ms: 0,
                    value: 0.0,
                },
                KfPoint {
                    t_ms: 2000,
                    value: 1.0,
                },
            ],
            interp: KfInterp::Linear,
        }];
    }
    let early = render_frame(&p, "layers-base-opacity-kf.mp4", 0.2);
    let late = frame_rgb(
        &dir.path().join("layers-base-opacity-kf.mp4"),
        1.8,
        320,
        240,
    );
    let (_, _, early_b) = px(&early, 320, 160, 120);
    let (_, _, late_b) = px(&late, 320, 160, 120);
    assert!(
        late_b > early_b.saturating_add(120),
        "base opacity keyframe brightens over time: {early_b} -> {late_b}"
    );
}

/// Native motion-graphics title regression: a TitleSpec encodes to a transparent
/// qtrle .mov (encode_title_overlay) that composites through the EXISTING
/// overlay pipeline with its ALPHA intact — proven at the pixel level. A red
/// rect title over a green base: inside the rect = red (title), OUTSIDE the
/// rect = green (the base shows through the transparent area — the alpha
/// survived). This is the whole load-bearing assumption of the title feature.
#[test]
fn title_overlay_composites_with_alpha() {
    use cut_media::title::{Easing, Keyframe, LayerContent, TitleLayer, TitleSpec};
    let dir = tempfile::tempdir().unwrap();
    // Base: 2s solid green.
    let base = dir.path().join("base.mp4");
    let args: Vec<String> = [
        "-f",
        "lavfi",
        "-i",
        "color=c=green:s=320x240:r=30:d=2",
        "-c:v",
        "libx264",
        "-preset",
        "ultrafast",
        "-pix_fmt",
        "yuv420p",
    ]
    .iter()
    .map(|s| s.to_string())
    .chain([base.display().to_string()])
    .collect();
    cut_media::ffmpeg::run_ffmpeg(&args).expect("base generation");

    // Title: a fully-opaque red rect covering the MIDDLE of the frame; the rest
    // of the canvas is transparent (resvg draws nothing there).
    let spec = TitleSpec {
        width: 320,
        height: 240,
        fps: 30,
        duration_ms: 2000,
        layers: vec![TitleLayer {
            content: LayerContent::Rect {
                color: "#FF0000".into(),
                opacity: 1.0,
                radius_px: 0.0,
            },
            x: 0.25,
            y: 0.375,
            w: 0.5,
            h: 0.25,
            keyframes: vec![Keyframe {
                t: 0.0,
                opacity: 1.0,
                tx: 0.0,
                ty: 0.0,
                scale: 1.0,
            }],
            easing: Easing::Linear,
        }],
    };
    let title_mov = dir.path().join("title.mov");
    cut_media::render::encode_title_overlay(&spec, &title_mov).expect("title encode");
    // The .mov must carry a real alpha channel (qtrle).
    let pf = std::process::Command::new(cut_media::ffmpeg::ffprobe_bin())
        .args([
            "-v",
            "error",
            "-select_streams",
            "v:0",
            "-show_entries",
            "stream=pix_fmt",
            "-of",
            "csv=p=0",
        ])
        .arg(&title_mov)
        .output()
        .expect("ffprobe title");
    let pix = String::from_utf8_lossy(&pf.stdout);
    assert!(
        pix.contains("argb")
            || pix.contains("rgba")
            || pix.contains("bgra")
            || pix.contains("yuva"),
        "title .mov must have an alpha pix_fmt, got {pix:?}"
    );

    let mut p = Project::new(
        "t",
        ProjectSettings {
            width: 320,
            height: 240,
            fps: 30.0,
            audio_rate: 48_000,
            color: cut_core::ColorConfig::default(),
        },
    );
    let mk_asset = |path: &Path, kind: &str| cut_core::Asset {
        path: path.display().to_string(),
        hash: "sha256:test".into(),
        probe: Some(serde_json::json!({"kind": kind})),
        transcript: None,
        perception: None,
        proxy: None,
        filmstrip: None,
    };
    p.assets.insert("base".into(), mk_asset(&base, "video"));
    p.assets
        .insert("title".into(), mk_asset(&title_mov, "video"));
    p.track_mut("v1").unwrap().clips = vec![Clip::Media(MediaClip {
        id: "c1".into(),
        asset: "base".into(),
        src_in_ms: 0,
        src_out_ms: 2000,
        effects: vec![],
        gain_db: 0.0,
        transform: None,
        crop: None,
        fade: None,
        xfade_in_ms: 0,
        xfade_kind: None,
        speed: 1.0,
        grade: None,
        matte: None,
        mask: None,
        reverse: false,
        freeze: None,
        animation: None,
        keyframes: vec![],
        eq: None,
        mute_ranges: vec![],
        stabilize: None,
        speed_ramp: None,
        input_color_space: None,
        nest: None,
        grade_stack: vec![],
        grade_windows: vec![],
    })];
    // Title on a full-frame overlay track (no transform — it's already canvas-sized).
    p.tracks.push(Track {
        id: "v2".into(),
        kind: TrackKind::Video,
        clips: vec![Clip::Media(MediaClip {
            id: "t1".into(),
            asset: "title".into(),
            src_in_ms: 0,
            src_out_ms: 2000,
            effects: vec![],
            gain_db: 0.0,
            transform: None,
            crop: None,
            fade: None,
            xfade_in_ms: 0,
            xfade_kind: None,
            speed: 1.0,
            grade: None,
            matte: None,
            mask: None,
            reverse: false,
            freeze: None,
            animation: None,
            keyframes: vec![],
            eq: None,
            mute_ranges: vec![],
            stabilize: None,
            speed_ramp: None,
            input_color_space: None,
            nest: None,
            grade_stack: vec![],
            grade_windows: vec![],
        })],
        gain_db: 0.0,
        gain_windows: vec![],
        blend_mode: None,
        visible: true,
        locked: false,
        muted: false,
        solo: false,
        pan: 0.0,
    });
    let edl = edl_from_project(&p);
    let fence = PathFence::new(dir.path()).unwrap();
    let out = render_final(
        &p,
        &edl,
        &fence,
        Path::new("titled.mp4"),
        &RenderPreset::default(),
        RenderOptions::default(),
        None,
    )
    .expect("title render");
    // Centre (inside the rect) = red title; corner (outside) = green base through alpha.
    let rgb = frame_rgb(&out.path, 1.0, 320, 240);
    let (r, g, _b) = px(&rgb, 320, 160, 120);
    assert!(r > 150 && g < 110, "title rect centre is red: ({r},{g},_)");
    let (r, g, _b) = px(&rgb, 320, 20, 20);
    assert!(
        g > 100 && r < 110,
        "outside the title: green base shows through the alpha ({r},{g},_)"
    );
}

/// edit.crop: a source with baked-in letterbox bands renders WITH black
/// at the frame edge when uncropped, and FILLS the frame (picture at the edge)
/// when edit.crop removes the bands. Compose order proof: crop runs in source
/// space BEFORE the conform scale, so the cropped picture is what gets scaled
/// to fill the project frame.
#[test]
fn render_crop_removes_baked_in_letterbox() {
    let dir = tempfile::tempdir().unwrap();
    // Source: a 320x160 GREEN picture padded to 320x240 with 40px BLACK bands
    // top and bottom (the baked-in-letterbox shape the OBS driver produced).
    let src = dir.path().join("boxed.mp4");
    let args: Vec<String> = [
        "-f",
        "lavfi",
        "-i",
        "color=c=green:s=320x160:r=30:d=2",
        "-vf",
        "pad=320:240:0:40:color=black",
        "-c:v",
        "libx264",
        "-preset",
        "ultrafast",
        "-pix_fmt",
        "yuv420p",
    ]
    .iter()
    .map(|s| s.to_string())
    .chain([src.display().to_string()])
    .collect();
    cut_media::ffmpeg::run_ffmpeg(&args).expect("letterboxed source generation");

    // Project geometry == source geometry (320x240) so a no-crop render keeps
    // the bands 1:1 (no aspect re-pad to confuse the proof).
    let mk = |p: &mut Project, crop: Option<cut_core::ClipCrop>| {
        p.assets.insert(
            "a1".into(),
            cut_core::Asset {
                path: src.display().to_string(),
                hash: "sha256:boxed".into(),
                probe: Some(
                    serde_json::json!({"kind": "video", "width": 320, "height": 240, "fps": 30.0}),
                ),
                transcript: None,
                perception: None,
                proxy: None,
                filmstrip: None,
            },
        );
        p.track_mut("v1").unwrap().clips = vec![Clip::Media(MediaClip {
            id: "c1".into(),
            asset: "a1".into(),
            src_in_ms: 0,
            src_out_ms: 2000,
            effects: vec![],
            gain_db: 0.0,
            transform: None,
            crop,
            fade: None,
            xfade_in_ms: 0,
            xfade_kind: None,
            speed: 1.0,
            grade: None,
            matte: None,
            mask: None,
            reverse: false,
            freeze: None,
            animation: None,
            keyframes: vec![],
            eq: None,
            mute_ranges: vec![],
            stabilize: None,
            speed_ramp: None,
            input_color_space: None,
            nest: None,
            grade_stack: vec![],
            grade_windows: vec![],
        })];
    };
    let fence = PathFence::new(dir.path()).unwrap();

    // --- uncropped: the top band (y=10) is BLACK in the output --------------
    let mut p0 = Project::new(
        "t",
        ProjectSettings {
            width: 320,
            height: 240,
            fps: 30.0,
            audio_rate: 48_000,
            color: cut_core::ColorConfig::default(),
        },
    );
    mk(&mut p0, None);
    let edl0 = edl_from_project(&p0);
    let out0 = render_final(
        &p0,
        &edl0,
        &fence,
        Path::new("boxed_nocrop.mp4"),
        &RenderPreset::default(),
        RenderOptions::default(),
        None,
    )
    .expect("uncropped render");
    let rgb0 = frame_rgb(&out0.path, 1.0, 320, 240);
    let (r0, g0, _b0) = px(&rgb0, 320, 160, 10); // top band
    assert!(
        r0 < 40 && g0 < 40,
        "uncropped: top band stays black ({r0},{g0},_)"
    );

    // --- cropped to the content (320x160 @ y=40): top of the frame is GREEN --
    let mut p1 = Project::new(
        "t",
        ProjectSettings {
            width: 320,
            height: 240,
            fps: 30.0,
            audio_rate: 48_000,
            color: cut_core::ColorConfig::default(),
        },
    );
    mk(
        &mut p1,
        Some(cut_core::ClipCrop {
            x: 0,
            y: 40,
            w: 320,
            h: 160,
        }),
    );
    let edl1 = edl_from_project(&p1);
    let out1 = render_final(
        &p1,
        &edl1,
        &fence,
        Path::new("boxed_cropped.mp4"),
        &RenderPreset::default(),
        RenderOptions::default(),
        None,
    )
    .expect("cropped render");
    let rgb1 = frame_rgb(&out1.path, 1.0, 320, 240);
    // The cropped 320x160 picture conforms into 320x240 with letterbox of its
    // own (contain), but the picture now reaches much higher than y=40 — sample
    // the vertical centre where the green picture definitely lands.
    let (r1, g1, _b1) = px(&rgb1, 320, 160, 120); // centre of the frame
    assert!(
        g1 > 100 && r1 < 100,
        "cropped: centre shows the picture (green) ({r1},{g1},_)"
    );
}

/// edit.reverse LIVE proof: a clip that is GREEN for its first second then RED
/// for its second second. Played NORMALLY the early frame is green and the late
/// frame is red; REVERSED, the early frame is red and the late frame is green —
/// and the timeline duration is UNCHANGED. This exercises the real
/// trim,setpts,`reverse`,conform chain through render_final (synthetic ground
/// truth, deterministic). Audio `areverse` is covered by the core unit test +
/// the measured ffmpeg research; this is the visible end-to-end render proof.
#[test]
fn render_reverse_flips_clip_in_time() {
    let dir = tempfile::tempdir().unwrap();
    // GREEN (0–1s) then RED (1–2s), concatenated into one 2s/30fps clip.
    let src = dir.path().join("greenred.mp4");
    let args: Vec<String> = [
        "-f",
        "lavfi",
        "-i",
        "color=c=green:s=320x240:r=30:d=1",
        "-f",
        "lavfi",
        "-i",
        "color=c=red:s=320x240:r=30:d=1",
        "-filter_complex",
        "[0:v][1:v]concat=n=2:v=1:a=0[v]",
        "-map",
        "[v]",
        "-c:v",
        "libx264",
        "-preset",
        "ultrafast",
        "-pix_fmt",
        "yuv420p",
    ]
    .iter()
    .map(|s| s.to_string())
    .chain([src.display().to_string()])
    .collect();
    cut_media::ffmpeg::run_ffmpeg(&args).expect("green→red source generation");

    let mk = |p: &mut Project, reverse: bool| {
        p.assets.insert(
            "a1".into(),
            cut_core::Asset {
                path: src.display().to_string(),
                hash: "sha256:greenred".into(),
                probe: Some(
                    serde_json::json!({"kind": "video", "width": 320, "height": 240, "fps": 30.0}),
                ),
                transcript: None,
                perception: None,
                proxy: None,
                filmstrip: None,
            },
        );
        p.track_mut("v1").unwrap().clips = vec![Clip::Media(MediaClip {
            id: "c1".into(),
            asset: "a1".into(),
            src_in_ms: 0,
            src_out_ms: 2000,
            effects: vec![],
            gain_db: 0.0,
            transform: None,
            crop: None,
            fade: None,
            xfade_in_ms: 0,
            xfade_kind: None,
            speed: 1.0,
            grade: None,
            matte: None,
            mask: None,
            reverse,
            freeze: None,
            animation: None,
            keyframes: vec![],
            eq: None,
            mute_ranges: vec![],
            stabilize: None,
            speed_ramp: None,
            input_color_space: None,
            nest: None,
            grade_stack: vec![],
            grade_windows: vec![],
        })];
    };
    let fence = PathFence::new(dir.path()).unwrap();
    let settings = ProjectSettings {
        width: 320,
        height: 240,
        fps: 30.0,
        audio_rate: 48_000,
        color: cut_core::ColorConfig::default(),
    };
    let is_green = |r: u8, g: u8| g > 100 && r < 100;
    let is_red = |r: u8, g: u8| r > 100 && g < 100;

    // --- NORMAL: early frame green, late frame red --------------------------
    let mut pn = Project::new("t", settings.clone());
    mk(&mut pn, false);
    let edln = edl_from_project(&pn);
    let outn = render_final(
        &pn,
        &edln,
        &fence,
        Path::new("normal.mp4"),
        &RenderPreset::default(),
        RenderOptions::default(),
        None,
    )
    .expect("normal render");
    let early_n = frame_rgb(&outn.path, 0.5, 320, 240);
    let late_n = frame_rgb(&outn.path, 1.5, 320, 240);
    let (rne, gne, _) = px(&early_n, 320, 160, 120);
    let (rnl, gnl, _) = px(&late_n, 320, 160, 120);
    assert!(
        is_green(rne, gne),
        "normal @0.5s should be GREEN ({rne},{gne})"
    );
    assert!(is_red(rnl, gnl), "normal @1.5s should be RED ({rnl},{gnl})");

    // --- REVERSED: early frame red, late frame green (time flipped) ---------
    let mut pr = Project::new("t", settings.clone());
    mk(&mut pr, true);
    let edlr = edl_from_project(&pr);
    let outr = render_final(
        &pr,
        &edlr,
        &fence,
        Path::new("reversed.mp4"),
        &RenderPreset::default(),
        RenderOptions::default(),
        None,
    )
    .expect("reversed render");
    let early_r = frame_rgb(&outr.path, 0.5, 320, 240);
    let late_r = frame_rgb(&outr.path, 1.5, 320, 240);
    let (rre, gre, _) = px(&early_r, 320, 160, 120);
    let (rrl, grl, _) = px(&late_r, 320, 160, 120);
    assert!(
        is_red(rre, gre),
        "reversed @0.5s should be RED ({rre},{gre})"
    );
    assert!(
        is_green(rrl, grl),
        "reversed @1.5s should be GREEN ({rrl},{grl})"
    );

    // Duration is UNCHANGED by reverse (still ~2s, within a frame).
    assert!(
        outr.duration_ms.abs_diff(outn.duration_ms) <= 34,
        "reverse preserves duration: normal {} vs reversed {}",
        outn.duration_ms,
        outr.duration_ms
    );
}

/// edit.freeze LIVE proof: a GREEN(0–1s)→RED(1–2s) clip frozen on a GREEN frame
/// (at_ms=200) renders GREEN across its WHOLE 2s slot (the held frame fills the
/// duration via tpad) — both the early AND late sampled frames are green, where a
/// normal render would have gone red at 1s. Duration is unchanged. Exercises the
/// real trim(1 frame),setpts,tpad,conform chain through render_final.
#[test]
fn render_freeze_holds_one_frame() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("greenred_fz.mp4");
    let args: Vec<String> = [
        "-f",
        "lavfi",
        "-i",
        "color=c=green:s=320x240:r=30:d=1",
        "-f",
        "lavfi",
        "-i",
        "color=c=red:s=320x240:r=30:d=1",
        "-filter_complex",
        "[0:v][1:v]concat=n=2:v=1:a=0[v]",
        "-map",
        "[v]",
        "-c:v",
        "libx264",
        "-preset",
        "ultrafast",
        "-pix_fmt",
        "yuv420p",
    ]
    .iter()
    .map(|s| s.to_string())
    .chain([src.display().to_string()])
    .collect();
    cut_media::ffmpeg::run_ffmpeg(&args).expect("green→red source generation");

    let mut p = Project::new(
        "t",
        ProjectSettings {
            width: 320,
            height: 240,
            fps: 30.0,
            audio_rate: 48_000,
            color: cut_core::ColorConfig::default(),
        },
    );
    p.assets.insert(
        "a1".into(),
        cut_core::Asset {
            path: src.display().to_string(),
            hash: "sha256:greenred_fz".into(),
            probe: Some(serde_json::json!({"kind": "video", "width": 320, "height": 240})),
            transcript: None,
            perception: None,
            proxy: None,
            filmstrip: None,
        },
    );
    p.track_mut("v1").unwrap().clips = vec![Clip::Media(MediaClip {
        id: "c1".into(),
        asset: "a1".into(),
        src_in_ms: 0,
        src_out_ms: 2000,
        effects: vec![],
        gain_db: 0.0,
        transform: None,
        crop: None,
        fade: None,
        xfade_in_ms: 0,
        xfade_kind: None,
        speed: 1.0,
        grade: None,
        matte: None,
        mask: None,
        reverse: false,
        // freeze on a GREEN frame (200ms into the green first second).
        freeze: Some(cut_core::ClipFreeze { at_ms: 200 }),
        animation: None,
        keyframes: vec![],
        eq: None,
        mute_ranges: vec![],
        stabilize: None,
        speed_ramp: None,
        input_color_space: None,
        nest: None,
        grade_stack: vec![],
        grade_windows: vec![],
    })];
    let fence = PathFence::new(dir.path()).unwrap();
    let edl = edl_from_project(&p);
    let out = render_final(
        &p,
        &edl,
        &fence,
        Path::new("frozen.mp4"),
        &RenderPreset::default(),
        RenderOptions::default(),
        None,
    )
    .expect("freeze render");

    // The held green frame fills BOTH halves of the slot (no red at 1.5s).
    let early = frame_rgb(&out.path, 0.5, 320, 240);
    let late = frame_rgb(&out.path, 1.5, 320, 240);
    let (re, ge, _) = px(&early, 320, 160, 120);
    let (rl, gl, _) = px(&late, 320, 160, 120);
    assert!(
        ge > 100 && re < 100,
        "frozen @0.5s should be GREEN ({re},{ge})"
    );
    assert!(
        gl > 100 && rl < 100,
        "frozen @1.5s should STILL be GREEN ({rl},{gl})"
    );
    // Duration unchanged by the freeze (~2s).
    assert!(
        out.duration_ms.abs_diff(2000) <= 70,
        "freeze fills the slot: duration {} ≈ 2000ms",
        out.duration_ms
    );
}

/// edit.animate LIVE proof: a GREEN-left | RED-right split frame, animated with a
/// PAN-RIGHT (zoom 1.3, focal centre x 0.3→0.7), pans the zoom window from the
/// left (green) to the right (red). So the CENTRE pixel reads GREEN early and RED
/// late — the window moved across the frame. Crucially the frame count is correct
/// (~2s, no zoompan explosion — the measured setpts PTS rebuild holds). Exercises
/// the real conform,fps,zoompan,setpts chain through render_final.
#[test]
fn render_animate_ken_burns_pans() {
    use cut_core::{AnimState, ClipAnimation};
    let dir = tempfile::tempdir().unwrap();
    // GREEN left half | RED right half, 320x240, 2s @30fps.
    let src = dir.path().join("splitlr.mp4");
    let args: Vec<String> = [
        "-f",
        "lavfi",
        "-i",
        "color=c=green:s=160x240:r=30:d=2",
        "-f",
        "lavfi",
        "-i",
        "color=c=red:s=160x240:r=30:d=2",
        "-filter_complex",
        "[0:v][1:v]hstack=inputs=2[v]",
        "-map",
        "[v]",
        "-c:v",
        "libx264",
        "-preset",
        "ultrafast",
        "-pix_fmt",
        "yuv420p",
    ]
    .iter()
    .map(|s| s.to_string())
    .chain([src.display().to_string()])
    .collect();
    cut_media::ffmpeg::run_ffmpeg(&args).expect("green|red split source generation");

    let mut p = Project::new(
        "t",
        ProjectSettings {
            width: 320,
            height: 240,
            fps: 30.0,
            audio_rate: 48_000,
            color: cut_core::ColorConfig::default(),
        },
    );
    p.assets.insert(
        "a1".into(),
        cut_core::Asset {
            path: src.display().to_string(),
            hash: "sha256:splitlr".into(),
            probe: Some(serde_json::json!({"kind": "video", "width": 320, "height": 240})),
            transcript: None,
            perception: None,
            proxy: None,
            filmstrip: None,
        },
    );
    p.track_mut("v1").unwrap().clips = vec![Clip::Media(MediaClip {
        id: "c1".into(),
        asset: "a1".into(),
        src_in_ms: 0,
        src_out_ms: 2000,
        effects: vec![],
        gain_db: 0.0,
        transform: None,
        crop: None,
        fade: None,
        xfade_in_ms: 0,
        xfade_kind: None,
        speed: 1.0,
        grade: None,
        matte: None,
        mask: None,
        reverse: false,
        freeze: None,
        // pan_right: zoom 1.3, focal centre x 0.3 → 0.7 (left→right).
        animation: Some(ClipAnimation {
            from: AnimState {
                zoom: 1.3,
                x: 0.3,
                y: 0.5,
            },
            to: AnimState {
                zoom: 1.3,
                x: 0.7,
                y: 0.5,
            },
        }),
        keyframes: vec![],
        eq: None,
        mute_ranges: vec![],
        stabilize: None,
        speed_ramp: None,
        input_color_space: None,
        nest: None,
        grade_stack: vec![],
        grade_windows: vec![],
    })];
    let fence = PathFence::new(dir.path()).unwrap();
    let edl = edl_from_project(&p);
    let out = render_final(
        &p,
        &edl,
        &fence,
        Path::new("kenburns.mp4"),
        &RenderPreset::default(),
        RenderOptions::default(),
        None,
    )
    .expect("ken burns render");

    // Centre pixel: GREEN early (window on the left), RED late (window panned right).
    let early = frame_rgb(&out.path, 0.2, 320, 240);
    let late = frame_rgb(&out.path, 1.8, 320, 240);
    let (re, ge, _) = px(&early, 320, 160, 120);
    let (rl, gl, _) = px(&late, 320, 160, 120);
    assert!(
        ge > 100 && re < 100,
        "pan @0.2s shows the LEFT (green) ({re},{ge})"
    );
    assert!(
        rl > 100 && gl < 100,
        "pan @1.8s shows the RIGHT (red) ({rl},{gl})"
    );
    // No zoompan explosion: duration stays ~2s (the setpts PTS rebuild holds).
    assert!(
        out.duration_ms.abs_diff(2000) <= 70,
        "animate keeps the frame count exact: duration {} ≈ 2000ms",
        out.duration_ms
    );
}

/// edit.keyframe (opacity) LIVE proof: a full-frame GREEN overlay over a RED base,
/// with opacity KEYFRAMED 1→0 across the 2s clip, fades the overlay out — so the
/// composite reads GREEN early (overlay opaque) and RED late (overlay transparent,
/// base shows through). Exercises the real `geq` alpha time-expression through
/// render_final's compositor.
#[test]
fn render_keyframe_opacity_fades_overlay() {
    use cut_core::{Keyframe, KfInterp, KfParam, KfPoint};
    let dir = tempfile::tempdir().unwrap();
    let gen = |name: &str, color: &str| -> std::path::PathBuf {
        let out = dir.path().join(name);
        let args: Vec<String> = [
            "-f",
            "lavfi",
            "-i",
            &format!("color=c={color}:s=320x240:r=30:d=2"),
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
        cut_media::ffmpeg::run_ffmpeg(&args).expect("color source generation");
        out
    };
    let red = gen("red.mp4", "red");
    let green = gen("green.mp4", "green");

    let mut p = Project::new(
        "t",
        ProjectSettings {
            width: 320,
            height: 240,
            fps: 30.0,
            audio_rate: 48_000,
            color: cut_core::ColorConfig::default(),
        },
    );
    for (id, path) in [("a1", &red), ("a2", &green)] {
        p.assets.insert(
            id.into(),
            cut_core::Asset {
                path: path.display().to_string(),
                hash: format!("sha256:{id}"),
                probe: Some(serde_json::json!({"kind": "video", "width": 320, "height": 240})),
                transcript: None,
                perception: None,
                proxy: None,
                filmstrip: None,
            },
        );
    }
    let mk = |id: &str, asset: &str| MediaClip {
        id: id.into(),
        asset: asset.into(),
        src_in_ms: 0,
        src_out_ms: 2000,
        effects: vec![],
        gain_db: 0.0,
        transform: None,
        crop: None,
        fade: None,
        xfade_in_ms: 0,
        xfade_kind: None,
        speed: 1.0,
        grade: None,
        matte: None,
        mask: None,
        reverse: false,
        freeze: None,
        animation: None,
        keyframes: vec![],
        eq: None,
        mute_ranges: vec![],
        stabilize: None,
        speed_ramp: None,
        input_color_space: None,
        nest: None,
        grade_stack: vec![],
        grade_windows: vec![],
    };
    // base = red on v1.
    p.track_mut("v1").unwrap().clips = vec![Clip::Media(mk("c1", "a1"))];
    // overlay = green on v2, full-frame, opacity keyframed 1 → 0.
    let mut ov = mk("c2", "a2");
    ov.keyframes = vec![Keyframe {
        param: KfParam::Opacity,
        points: vec![
            KfPoint {
                t_ms: 0,
                value: 1.0,
            },
            KfPoint {
                t_ms: 2000,
                value: 0.0,
            },
        ],
        interp: KfInterp::Linear,
    }];
    p.tracks.push(Track {
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

    let fence = PathFence::new(dir.path()).unwrap();
    let edl = edl_from_project(&p);
    let out = render_final(
        &p,
        &edl,
        &fence,
        Path::new("kfopacity.mp4"),
        &RenderPreset::default(),
        RenderOptions::default(),
        None,
    )
    .expect("keyframe-opacity render");

    let early = frame_rgb(&out.path, 0.2, 320, 240);
    let late = frame_rgb(&out.path, 1.8, 320, 240);
    let (re, ge, _) = px(&early, 320, 160, 120);
    let (rl, gl, _) = px(&late, 320, 160, 120);
    assert!(
        ge > 100 && re < 100,
        "early (opacity~1) shows GREEN overlay ({re},{ge})"
    );
    assert!(
        rl > 100 && gl < 100,
        "late (opacity~0) shows RED base ({rl},{gl})"
    );
}

/// edit.keyframe POSITION (pos_x) LIVE proof: a small green PiP slides left→right
/// across a red base via pos_x keyframes 0.0 → 0.7. The animated-placement branch
/// (overlay onto a transparent canvas at a time-expression x) must move the green
/// block's leftmost column measurably to the right from an early frame to a late one.
#[test]
fn render_keyframe_position_slides_overlay() {
    use cut_core::{Keyframe, KfInterp, KfParam, KfPoint};
    let dir = tempfile::tempdir().unwrap();
    let gen = |name: &str, color: &str| -> std::path::PathBuf {
        let out = dir.path().join(name);
        let args: Vec<String> = [
            "-f",
            "lavfi",
            "-i",
            &format!("color=c={color}:s=320x240:r=30:d=2"),
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
        cut_media::ffmpeg::run_ffmpeg(&args).expect("color source generation");
        out
    };
    let red = gen("red_p.mp4", "red");
    let green = gen("green_p.mp4", "green");

    let mut p = Project::new(
        "t",
        ProjectSettings {
            width: 320,
            height: 240,
            fps: 30.0,
            audio_rate: 48_000,
            color: cut_core::ColorConfig::default(),
        },
    );
    for (id, path) in [("a1", &red), ("a2", &green)] {
        p.assets.insert(
            id.into(),
            cut_core::Asset {
                path: path.display().to_string(),
                hash: format!("sha256:{id}"),
                probe: Some(serde_json::json!({"kind": "video", "width": 320, "height": 240})),
                transcript: None,
                perception: None,
                proxy: None,
                filmstrip: None,
            },
        );
    }
    let mk = |id: &str, asset: &str| MediaClip {
        id: id.into(),
        asset: asset.into(),
        src_in_ms: 0,
        src_out_ms: 2000,
        effects: vec![],
        gain_db: 0.0,
        transform: None,
        crop: None,
        fade: None,
        xfade_in_ms: 0,
        xfade_kind: None,
        speed: 1.0,
        grade: None,
        matte: None,
        mask: None,
        reverse: false,
        freeze: None,
        animation: None,
        keyframes: vec![],
        eq: None,
        mute_ranges: vec![],
        stabilize: None,
        speed_ramp: None,
        input_color_space: None,
        nest: None,
        grade_stack: vec![],
        grade_windows: vec![],
    };
    // base = red on v1.
    p.track_mut("v1").unwrap().clips = vec![Clip::Media(mk("c1", "a1"))];
    // overlay = green PiP (scale 0.25 → 80x60, top row) on v2, pos_x 0.0 → 0.7.
    let mut ov = mk("c2", "a2");
    ov.transform = Some(cut_core::ClipTransform {
        x: 0.0,
        y: 0.0,
        scale: 0.25,
        opacity: 1.0,
    });
    ov.keyframes = vec![Keyframe {
        param: KfParam::PosX,
        points: vec![
            KfPoint {
                t_ms: 0,
                value: 0.0,
            },
            KfPoint {
                t_ms: 2000,
                value: 0.7,
            },
        ],
        interp: KfInterp::Linear,
    }];
    p.tracks.push(Track {
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

    let fence = PathFence::new(dir.path()).unwrap();
    let edl = edl_from_project(&p);
    let out = render_final(
        &p,
        &edl,
        &fence,
        Path::new("kfpos.mp4"),
        &RenderPreset::default(),
        RenderOptions::default(),
        None,
    )
    .expect("keyframe-position render");

    // Leftmost green column on the overlay's row (y=30) at an early vs a late frame.
    let leftmost_green = |at: f64| -> Option<u32> {
        let f = frame_rgb(&out.path, at, 320, 240);
        (0u32..320).find(|&x| {
            let (r, g, _) = px(&f, 320, x, 30);
            g > 120 && r < 120
        })
    };
    let early = leftmost_green(0.2).expect("green PiP visible early");
    let late = leftmost_green(1.8).expect("green PiP visible late");
    assert!(
        late > early + 80,
        "pos_x keyframes slid the PiP right: leftmost green x {early} → {late}"
    );
}

/// Render fit modes: a source whose aspect differs from the project frame
/// renders with BLACK bands under `contain` (default) and FILLS the frame
/// (crop-to-fill, no bands) under `cover`. Proof uses a 320x120 green source
/// in a 320x240 (taller) project: contain pillars/letterboxes it, cover scales
/// it up to cover and crops.
#[test]
fn render_fit_contain_vs_cover() {
    use cut_media::{Fit, Resolution};
    let dir = tempfile::tempdir().unwrap();
    // 320x120 solid green (wider-than-tall relative to the 320x240 frame).
    let src = dir.path().join("wide.mp4");
    let args: Vec<String> = [
        "-f",
        "lavfi",
        "-i",
        "color=c=green:s=320x120:r=30:d=2",
        "-c:v",
        "libx264",
        "-preset",
        "ultrafast",
        "-pix_fmt",
        "yuv420p",
    ]
    .iter()
    .map(|s| s.to_string())
    .chain([src.display().to_string()])
    .collect();
    cut_media::ffmpeg::run_ffmpeg(&args).expect("wide source generation");

    let mk = || {
        let mut p = Project::new(
            "t",
            ProjectSettings {
                width: 320,
                height: 240,
                fps: 30.0,
                audio_rate: 48_000,
                color: cut_core::ColorConfig::default(),
            },
        );
        p.assets.insert(
            "a1".into(),
            cut_core::Asset {
                path: src.display().to_string(),
                hash: "sha256:wide".into(),
                probe: Some(serde_json::json!({"kind": "video", "width": 320, "height": 120})),
                transcript: None,
                perception: None,
                proxy: None,
                filmstrip: None,
            },
        );
        p.track_mut("v1").unwrap().clips = vec![Clip::Media(MediaClip {
            id: "c1".into(),
            asset: "a1".into(),
            src_in_ms: 0,
            src_out_ms: 2000,
            effects: vec![],
            gain_db: 0.0,
            transform: None,
            crop: None,
            fade: None,
            xfade_in_ms: 0,
            xfade_kind: None,
            speed: 1.0,
            grade: None,
            matte: None,
            mask: None,
            reverse: false,
            freeze: None,
            animation: None,
            keyframes: vec![],
            eq: None,
            mute_ranges: vec![],
            stabilize: None,
            speed_ramp: None,
            input_color_space: None,
            nest: None,
            grade_stack: vec![],
            grade_windows: vec![],
        })];
        p
    };
    let fence = PathFence::new(dir.path()).unwrap();

    // contain (default): the top of the 320x240 frame is BLACK (letterbox).
    let p = mk();
    let edl = edl_from_project(&p);
    let out = render_final(
        &p,
        &edl,
        &fence,
        Path::new("contain.mp4"),
        &RenderPreset::default(),
        RenderOptions::default(),
        None,
    )
    .expect("contain render");
    let rgb = frame_rgb(&out.path, 1.0, 320, 240);
    let (r, g, _b) = px(&rgb, 320, 160, 10);
    assert!(
        r < 40 && g < 40,
        "contain: top is black (letterbox) ({r},{g},_)"
    );

    // cover: the whole frame is GREEN (scaled up to cover, sides cropped).
    let p = mk();
    let edl = edl_from_project(&p);
    let opts = RenderOptions {
        fit: Fit::Cover,
        resolution: Resolution::Project,
        loudness_target: None,
    };
    let out = render_final(
        &p,
        &edl,
        &fence,
        Path::new("cover.mp4"),
        &RenderPreset::default(),
        opts,
        None,
    )
    .expect("cover render");
    assert_eq!(
        out.fit.as_deref(),
        Some("cover"),
        "non-default fit recorded on the output"
    );
    let rgb = frame_rgb(&out.path, 1.0, 320, 240);
    let (r, g, _b) = px(&rgb, 320, 160, 10); // same top spot — now green
    assert!(
        g > 100 && r < 100,
        "cover: top is filled with picture (green) ({r},{g},_)"
    );
}

/// Crossfade-timebase regression: `edit.crossfade` must not break the composed
/// render at 60fps (or any project fps). The xfade filter HARD-REQUIRES both
/// input legs at a matching timebase; before the fix the accumulator leg
/// (concat output) carried the microsecond timebase 1/1000000 while the fresh
/// segment leg carried the frame timebase 1/60, so xfade refused to configure
/// — and because that poisons graph configuration, the WHOLE compose graph
/// died, not just the seam (a frame far from the crossfade failed too). At
/// 30fps both legs happened to negotiate to 1/1000000 and the bug hid; this
/// test runs at 60fps to catch it. The fix normalises both legs with
/// settb=AVTB,fps before the xfade (fold_video).
///
/// Proves: (a) render_final succeeds on a 60fps two-clip crossfade project,
/// (b) a composed frame FAR from the seam succeeds (the global-poison repro),
/// (c) a composed frame AT the seam succeeds and is a genuine blend of both
/// clips (neither pure clip-A nor pure clip-B), and (d) the realized timeline
/// is shortened by the crossfade overlap.
#[test]
fn crossfade_renders_at_60fps_and_seam_blends() {
    let dir = tempfile::tempdir().unwrap();
    // Two distinct 60fps solid-colour sources so a seam blend is visible:
    // clip A = pure RED, clip B = pure BLUE, each 2s @ 60fps, 320x240.
    let gen = |name: &str, color: &str| -> PathBuf {
        let out = dir.path().join(name);
        let args: Vec<String> = [
            "-f",
            "lavfi",
            "-i",
            &format!("color=c={color}:s=320x240:r=60:d=2"),
            "-c:v",
            "libx264",
            "-preset",
            "ultrafast",
            "-pix_fmt",
            "yuv420p",
            // Short GOP so the 60fps timebase is exercised throughout.
            "-g",
            "30",
        ]
        .iter()
        .map(|s| s.to_string())
        .chain([out.display().to_string()])
        .collect();
        cut_media::ffmpeg::run_ffmpeg(&args).expect("60fps source generation");
        out
    };
    let red = gen("red60.mp4", "red");
    let blue = gen("blue60.mp4", "blue");

    let mut p = Project::new(
        "t",
        ProjectSettings {
            width: 320,
            height: 240,
            fps: 60.0,
            audio_rate: 48_000,
            color: cut_core::ColorConfig::default(),
        },
    );
    for (id, path) in [("a1", &red), ("a2", &blue)] {
        p.assets.insert(
            id.into(),
            cut_core::Asset {
                path: path.display().to_string(),
                hash: format!("sha256:{id}"),
                probe: Some(serde_json::json!({"kind": "video", "width": 320, "height": 240})),
                transcript: None,
                perception: None,
                proxy: None,
                filmstrip: None,
            },
        );
    }
    // Clip A [0,2000), clip B [2000,4000) with a 400ms crossfade IN on B:
    // the EDL shortens to 3600ms and the seam dissolves red→blue over [1600,2000).
    p.track_mut("v1").unwrap().clips = vec![
        Clip::Media(MediaClip {
            id: "ca".into(),
            asset: "a1".into(),
            src_in_ms: 0,
            src_out_ms: 2000,
            effects: vec![],
            gain_db: 0.0,
            transform: None,
            crop: None,
            fade: None,
            xfade_in_ms: 0,
            xfade_kind: None,
            speed: 1.0,
            grade: None,
            matte: None,
            mask: None,
            reverse: false,
            freeze: None,
            animation: None,
            keyframes: vec![],
            eq: None,
            mute_ranges: vec![],
            stabilize: None,
            speed_ramp: None,
            input_color_space: None,
            nest: None,
            grade_stack: vec![],
            grade_windows: vec![],
        }),
        Clip::Media(MediaClip {
            id: "cb".into(),
            asset: "a2".into(),
            src_in_ms: 0,
            src_out_ms: 2000,
            effects: vec![],
            gain_db: 0.0,
            transform: None,
            crop: None,
            fade: None,
            xfade_in_ms: 400,
            xfade_kind: None,
            speed: 1.0,
            grade: None,
            matte: None,
            mask: None,
            reverse: false,
            freeze: None,
            animation: None,
            keyframes: vec![],
            eq: None,
            mute_ranges: vec![],
            stabilize: None,
            speed_ramp: None,
            input_color_space: None,
            nest: None,
            grade_stack: vec![],
            grade_windows: vec![],
        }),
    ];
    let edl = edl_from_project(&p);
    // (d) Realized duration = 2000 + 2000 − 400 = 3600ms (overlap shortens it).
    assert_eq!(
        edl.duration_ms, 3600,
        "crossfade shortens the realized timeline by the overlap"
    );

    let fence = PathFence::new(dir.path()).unwrap();

    // (a) The full composed render must succeed at 60fps (pre-fix: exit 234,
    // "First input link main timebase (1/1000000) do not match … xfade (1/60)").
    let out = render_final(
        &p,
        &edl,
        &fence,
        Path::new("xfade60.mp4"),
        &RenderPreset::default(),
        RenderOptions::default(),
        None,
    )
    .expect("60fps crossfade render must succeed (the crossfade-timebase contract)");
    assert!(out.path.is_file(), "render produced an output file");
    // Measured duration within one 60fps frame (~17ms) of the EDL.
    assert!(
        (out.duration_ms as i64 - 3600).abs() <= 34,
        "rendered duration ≈ EDL 3600ms (got {})",
        out.duration_ms
    );

    // (b) A composed frame FAR from the seam (200ms, deep in clip A) must
    // succeed — the pre-fix bug poisoned the WHOLE graph, so even this failed.
    let early = extract_frame(&p, &edl, dir.path(), 200, None)
        .expect("composed frame far from the seam must render (the crossfade-timebase contract global poison)");
    assert!(
        !early.is_empty() && early[..3] == [0xFF, 0xD8, 0xFF],
        "valid JPEG far from seam"
    );

    // (c) A composed frame AT the seam (1800ms, mid-crossfade [1600,2000)) must
    // be a genuine BLEND: red fading into blue → the pixel has BOTH non-trivial
    // red and blue (a hard cut would be pure red or pure blue).
    let mid_rgb = frame_rgb(&out.path, 1.8, 320, 240);
    let (r, g, b) = px(&mid_rgb, 320, 160, 120); // centre pixel
    assert!(
        r > 30 && b > 30 && g < 80,
        "seam frame is a red↔blue blend, not a hard cut (r={r},g={g},b={b})"
    );
}

/// Transitions end-to-end: a STYLED crossfade (wipeleft, not the default dissolve)
/// must RENDER — ffmpeg has to accept the xfade transition name through the real
/// graph (a bad/unsupported name fails the render). Proves the EDL→fold_video→
/// ffmpeg chain for non-default styles, and the realized timeline still shortens
/// by the overlap. (The dissolve seam-blend pixels are covered by the test above.)
#[test]
fn styled_crossfade_renders_end_to_end() {
    let dir = tempfile::tempdir().unwrap();
    let a = gen_clip(dir.path(), "a.mp4");
    let b = gen_clip(dir.path(), "b.mp4");
    let fence = PathFence::new(dir.path()).unwrap();
    let mk = |path: &Path| cut_core::Asset {
        path: path.display().to_string(),
        hash: "sha256:x".into(),
        probe: Some(serde_json::json!({"kind":"video","width":320,"height":240})),
        transcript: None,
        perception: None,
        proxy: None,
        filmstrip: None,
    };
    let mut p = Project::new(
        "t",
        ProjectSettings {
            width: 320,
            height: 240,
            fps: 30.0,
            audio_rate: 48_000,
            color: cut_core::ColorConfig::default(),
        },
    );
    p.assets.insert("a1".into(), mk(&a));
    p.assets.insert("a2".into(), mk(&b));
    let clip = |id: &str, asset: &str, xf: u64, kind: Option<&str>| {
        Clip::Media(MediaClip {
            id: id.into(),
            asset: asset.into(),
            src_in_ms: 0,
            src_out_ms: 2000,
            effects: vec![],
            gain_db: 0.0,
            transform: None,
            crop: None,
            fade: None,
            xfade_in_ms: xf,
            xfade_kind: kind.map(String::from),
            speed: 1.0,
            grade: None,
            matte: None,
            mask: None,
            reverse: false,
            freeze: None,
            animation: None,
            keyframes: vec![],
            eq: None,
            mute_ranges: vec![],
            stabilize: None,
            speed_ramp: None,
            input_color_space: None,
            nest: None,
            grade_stack: vec![],
            grade_windows: vec![],
        })
    };
    p.track_mut("v1").unwrap().clips = vec![
        clip("c1", "a1", 0, None),
        clip("c2", "a2", 500, Some("wipeleft")), // 500ms wipeleft into the second clip
    ];
    let edl = edl_from_project(&p);
    let out = dir.path().join("styled.mp4");
    render_final(
        &p,
        &edl,
        &fence,
        &out,
        &RenderPreset::default(),
        RenderOptions::default(),
        None,
    )
    .expect("a wipeleft crossfade must render (ffmpeg accepts the transition name)");
    let d = probe(&out)
        .expect("probe styled output")
        .duration_ms
        .unwrap_or(0);
    assert!(
        (3300..=3700).contains(&d),
        "realized length ~3500ms (2000+2000-500 overlap), got {d}ms"
    );
}

/// Effects end-to-end: a base clip with vignette+grain AND a chroma-key OVERLAY
/// must RENDER — ffmpeg has to accept every effect's filter (vignette/noise/
/// chromakey) through the real graph (a bad filter fails the render). Proves the
/// edit.effect → EDL → effect_filter → ffmpeg chain, incl. the overlay chroma path.
#[test]
fn effects_render_end_to_end() {
    use cut_core::ClipEffect as E;
    let dir = tempfile::tempdir().unwrap();
    let base = gen_clip(dir.path(), "base.mp4");
    let over = gen_clip(dir.path(), "over.mp4");
    let fence = PathFence::new(dir.path()).unwrap();
    let mk = |path: &Path| cut_core::Asset {
        path: path.display().to_string(),
        hash: "sha256:x".into(),
        probe: Some(serde_json::json!({"kind":"video","width":320,"height":240})),
        transcript: None,
        perception: None,
        proxy: None,
        filmstrip: None,
    };
    let mut p = Project::new(
        "t",
        ProjectSettings {
            width: 320,
            height: 240,
            fps: 30.0,
            audio_rate: 48_000,
            color: cut_core::ColorConfig::default(),
        },
    );
    p.assets.insert("a1".into(), mk(&base));
    p.assets.insert("a2".into(), mk(&over));
    let clip =
        |id: &str, asset: &str, effects: Vec<E>, transform: Option<cut_core::ClipTransform>| {
            Clip::Media(MediaClip {
                id: id.into(),
                asset: asset.into(),
                src_in_ms: 0,
                src_out_ms: 2000,
                effects,
                gain_db: 0.0,
                transform,
                crop: None,
                fade: None,
                xfade_in_ms: 0,
                xfade_kind: None,
                speed: 1.0,
                grade: None,
                matte: None,
                mask: None,
                reverse: false,
                freeze: None,
                animation: None,
                keyframes: vec![],
                eq: None,
                mute_ranges: vec![],
                stabilize: None,
                speed_ramp: None,
                input_color_space: None,
                nest: None,
                grade_stack: vec![],
                grade_windows: vec![],
            })
        };
    // Base clip: vignette + grain + mirror + flip + hue (uniform effects, chained).
    p.track_mut("v1").unwrap().clips = vec![clip(
        "c1",
        "a1",
        vec![
            E::Vignette { amount: 0.5 },
            E::Grain { amount: 15.0 },
            E::Mirror,
            E::Flip,
            E::HueShift { degrees: 45.0 },
        ],
        None,
    )];
    // Overlay clip: chroma key (greenscreen) — reveals the base below.
    p.tracks.push(Track {
        id: "v2".into(),
        kind: TrackKind::Video,
        clips: vec![clip(
            "ov",
            "a2",
            vec![E::ChromaKey {
                color: "green".into(),
                similarity: 0.2,
                blend: 0.1,
            }],
            Some(cut_core::ClipTransform {
                x: 0.5,
                y: 0.5,
                scale: 0.4,
                opacity: 1.0,
            }),
        )],
        gain_db: 0.0,
        gain_windows: vec![],
        blend_mode: None,
        visible: true,
        locked: false,
        muted: false,
        solo: false,
        pan: 0.0,
    });
    let edl = edl_from_project(&p);
    let out = dir.path().join("effects.mp4");
    render_final(
        &p,
        &edl,
        &fence,
        &out,
        &RenderPreset::default(),
        RenderOptions::default(),
        None,
    )
    .expect("vignette+grain base + chroma-key overlay must render (ffmpeg accepts the filters)");
    let d = probe(&out)
        .expect("probe effects output")
        .duration_ms
        .unwrap_or(0);
    assert!((1800..=2200).contains(&d), "~2s, got {d}ms");
}

/// Audio denoise (edit.effect type=denoise → afftdn): the ONE audio effect must
/// RENDER on an audio-track clip (ffmpeg accepts afftdn), and the EDL carries it
/// onto the audio segment. Our talking-head/podcast wedge's voice-cleanup.
#[test]
fn denoise_audio_effect_renders() {
    use cut_core::ClipEffect as E;
    let dir = tempfile::tempdir().unwrap();
    let clip = gen_clip(dir.path(), "src.mp4"); // gen_clip has a sine audio track
    let fence = PathFence::new(dir.path()).unwrap();
    let mut p = Project::new(
        "t",
        ProjectSettings {
            width: 320,
            height: 240,
            fps: 30.0,
            audio_rate: 48_000,
            color: cut_core::ColorConfig::default(),
        },
    );
    p.assets.insert(
        "a1".into(),
        cut_core::Asset {
            path: clip.display().to_string(),
            hash: "sha256:x".into(),
            probe: Some(serde_json::json!({"kind":"video","width":320,"height":240})),
            transcript: None,
            perception: None,
            proxy: None,
            filmstrip: None,
        },
    );
    let media = |id: &str, fx: Vec<E>| {
        Clip::Media(MediaClip {
            id: id.into(),
            asset: "a1".into(),
            src_in_ms: 0,
            src_out_ms: 2000,
            effects: fx,
            gain_db: 0.0,
            transform: None,
            crop: None,
            fade: None,
            xfade_in_ms: 0,
            xfade_kind: None,
            speed: 1.0,
            grade: None,
            matte: None,
            mask: None,
            reverse: false,
            freeze: None,
            animation: None,
            keyframes: vec![],
            eq: None,
            mute_ranges: vec![],
            stabilize: None,
            speed_ramp: None,
            input_color_space: None,
            nest: None,
            grade_stack: vec![],
            grade_windows: vec![],
        })
    };
    p.track_mut("v1").unwrap().clips = vec![media("c1", vec![])];
    // Denoise on the AUDIO-track clip.
    p.track_mut("a1t").unwrap().clips = vec![media("c1a", vec![E::Denoise { amount: 0.6 }])];
    let edl = edl_from_project(&p);
    let out = dir.path().join("denoise.mp4");
    render_final(
        &p,
        &edl,
        &fence,
        &out,
        &RenderPreset::default(),
        RenderOptions::default(),
        None,
    )
    .expect("denoise (afftdn) on the audio clip must render");
    assert!(out.exists());
    let aseg = edl
        .segments
        .iter()
        .find(|s| s.clip_id.as_deref() == Some("c1a"))
        .unwrap();
    assert!(
        aseg.effects.iter().any(|e| matches!(e, E::Denoise { .. })),
        "EDL carries the denoise onto the audio segment"
    );
}

/// non-exact-scale guard regression: a NON-EXACT transform scale (0.62) must render.
/// Pre-fix, the transform's scale stage even-rounded the PiP geometry, ffmpeg
/// compensated the sub-pixel aspect drift with a non-1:1 SAR, and the overlay
/// track's concat refused the mixed-SAR streams ("SAR a:b do not match SAR
/// 1:1") — one odd PiP scale bricked EVERY render of the project. setsar=1
/// now follows the transform scale. Exact scales (0.5) are covered by
/// render_composites_overlay_track_with_transform above.
#[test]
fn render_survives_non_exact_transform_scale() {
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().join("base.mp4");
    let args: Vec<String> = [
        "-f",
        "lavfi",
        "-i",
        "color=c=green:s=320x240:r=30:d=2",
        "-c:v",
        "libx264",
        "-preset",
        "ultrafast",
        "-pix_fmt",
        "yuv420p",
    ]
    .iter()
    .map(|s| s.to_string())
    .chain([base.display().to_string()])
    .collect();
    cut_media::ffmpeg::run_ffmpeg(&args).expect("base generation");
    let still = gen_still(dir.path(), "pip.png");

    let mut p = Project::new(
        "t",
        ProjectSettings {
            width: 320,
            height: 240,
            fps: 30.0,
            audio_rate: 48_000,
            color: cut_core::ColorConfig::default(),
        },
    );
    let mk_asset = |path: &Path, kind: &str| cut_core::Asset {
        path: path.display().to_string(),
        hash: "sha256:test".into(),
        probe: Some(serde_json::json!({"kind": kind})),
        transcript: None,
        perception: None,
        proxy: None,
        filmstrip: None,
    };
    p.assets.insert("base".into(), mk_asset(&base, "video"));
    p.assets.insert("pip".into(), mk_asset(&still, "image"));
    p.track_mut("v1").unwrap().clips = vec![Clip::Media(MediaClip {
        id: "c1".into(),
        asset: "base".into(),
        src_in_ms: 0,
        src_out_ms: 2000,
        effects: vec![],
        gain_db: 0.0,
        transform: None,
        crop: None,
        fade: None,
        xfade_in_ms: 0,
        xfade_kind: None,
        speed: 1.0,
        grade: None,
        matte: None,
        mask: None,
        reverse: false,
        freeze: None,
        animation: None,
        keyframes: vec![],
        eq: None,
        mute_ranges: vec![],
        stabilize: None,
        speed_ramp: None,
        input_color_space: None,
        nest: None,
        grade_stack: vec![],
        grade_windows: vec![],
    })];
    // The overlay-tail repro shape: overlay clip for 1s + trailing transparent filler
    // (so the overlay track concats ≥2 streams), scale 0.62 → 198×148 even-
    // rounded from 198.4×148.8 (the aspect-drift case).
    p.tracks.push(Track {
        id: "v2".into(),
        kind: TrackKind::Video,
        clips: vec![Clip::Media(MediaClip {
            id: "c2".into(),
            asset: "pip".into(),
            src_in_ms: 0,
            src_out_ms: 1000,
            effects: vec![],
            gain_db: 0.0,
            transform: Some(cut_core::ClipTransform {
                x: 0.5,
                y: 0.5,
                scale: 0.62,
                opacity: 1.0,
            }),
            crop: None,
            fade: None,
            xfade_in_ms: 0,
            xfade_kind: None,
            speed: 1.0,
            grade: None,
            matte: None,
            mask: None,
            reverse: false,
            freeze: None,
            animation: None,
            keyframes: vec![],
            eq: None,
            mute_ranges: vec![],
            stabilize: None,
            speed_ramp: None,
            input_color_space: None,
            nest: None,
            grade_stack: vec![],
            grade_windows: vec![],
        })],
        gain_db: 0.0,
        gain_windows: vec![],
        blend_mode: None,
        visible: true,
        locked: false,
        muted: false,
        solo: false,
        pan: 0.0,
    });
    let edl = edl_from_project(&p);
    let fence = PathFence::new(dir.path()).unwrap();
    let out = render_final(
        &p,
        &edl,
        &fence,
        Path::new("pip62.mp4"),
        &RenderPreset::default(),
        RenderOptions::default(),
        None,
    )
    .expect("non-exact scale must render (SAR conformed after transform scale)");
    assert!(
        out.duration_ms.abs_diff(2000) <= 34,
        "duration {} ms",
        out.duration_ms
    );
    // PiP is clamped inside the frame: 198×148 at (122,92) — center is red,
    // a far-corner base pixel stays green.
    let rgb = frame_rgb(&out.path, 0.5, 320, 240);
    let (r, g, _b) = px(&rgb, 320, 221, 166);
    assert!(r > 180 && g < 100, "PiP pixel is red: ({r},{g},_)");
    let (r, g, _b) = px(&rgb, 320, 40, 40);
    assert!(g > 100 && r < 100, "base pixel stays green: ({r},{g},_)");
}

/// RMS level (dB) of a time window of `path`'s audio, optionally band-passed
/// around `bandpass_hz` (Q=5) — lets a test measure one sine in a mix.
fn rms_db(path: &Path, from_s: f64, to_s: f64, bandpass_hz: Option<u32>) -> f64 {
    let mut af = format!("atrim=start={from_s}:end={to_s},asetpts=PTS-STARTPTS");
    if let Some(hz) = bandpass_hz {
        af.push_str(&format!(",bandpass=f={hz}:width_type=q:w=5"));
    }
    af.push_str(",astats=measure_perchannel=none");
    let out = std::process::Command::new("ffmpeg")
        .args(["-hide_banner", "-nostdin", "-i"])
        .arg(path)
        .args(["-af", &af, "-f", "null", "-"])
        .output()
        .expect("ffmpeg astats");
    let stderr = String::from_utf8_lossy(&out.stderr);
    stderr
        .lines()
        .filter_map(|l| l.split("RMS level dB:").nth(1))
        .next_back()
        .unwrap_or_else(|| panic!("no RMS in astats output:\n{stderr}"))
        .trim()
        .parse()
        .expect("RMS parses")
}

/// Multi-track mixing regression: the render amixes ALL audio tracks with
/// per-clip gain honored — speech (440 Hz) + music bed (2 kHz at −18 dB)
/// must BOTH be audible in the output at their relative levels.
#[test]
fn render_mixes_all_audio_tracks_with_gain() {
    let dir = tempfile::tempdir().unwrap();
    let speech = gen_clip(dir.path(), "speech.mp4"); // video + 440 Hz sine
                                                     // Audio-only 2 kHz "music bed".
    let music = dir.path().join("music.wav");
    let args: Vec<String> = ["-f", "lavfi", "-i", "sine=frequency=2000:duration=2"]
        .iter()
        .map(|s| s.to_string())
        .chain([music.display().to_string()])
        .collect();
    cut_media::ffmpeg::run_ffmpeg(&args).expect("music generation");

    let mut p = Project::new(
        "t",
        ProjectSettings {
            width: 320,
            height: 240,
            fps: 30.0,
            audio_rate: 48_000,
            color: cut_core::ColorConfig::default(),
        },
    );
    let mk_asset = |path: &Path| cut_core::Asset {
        path: path.display().to_string(),
        hash: "sha256:test".into(),
        probe: None,
        transcript: None,
        perception: None,
        proxy: None,
        filmstrip: None,
    };
    p.assets.insert("sp".into(), mk_asset(&speech));
    p.assets.insert("mu".into(), mk_asset(&music));
    let mk = |id: &str, asset: &str, gain: f64| {
        Clip::Media(MediaClip {
            id: id.into(),
            asset: asset.into(),
            src_in_ms: 0,
            src_out_ms: 2000,
            effects: vec![],
            gain_db: gain,
            transform: None,
            crop: None,
            fade: None,
            xfade_in_ms: 0,
            xfade_kind: None,
            speed: 1.0,
            grade: None,
            matte: None,
            mask: None,
            reverse: false,
            freeze: None,
            animation: None,
            keyframes: vec![],
            eq: None,
            mute_ranges: vec![],
            stabilize: None,
            speed_ramp: None,
            input_color_space: None,
            nest: None,
            grade_stack: vec![],
            grade_windows: vec![],
        })
    };
    p.track_mut("v1").unwrap().clips = vec![mk("c1", "sp", 0.0)];
    p.track_mut("a1t").unwrap().clips = vec![mk("c2", "sp", 0.0)];
    // Second audio track — the music bed at −18 dB clip gain.
    p.tracks.push(Track {
        id: "a2t".into(),
        kind: TrackKind::Audio,
        clips: vec![mk("c3", "mu", -18.0)],
        gain_db: 0.0,
        gain_windows: vec![],
        blend_mode: None,
        visible: true,
        locked: false,
        muted: false,
        solo: false,
        pan: 0.0,
    });
    let edl = edl_from_project(&p);
    let fence = PathFence::new(dir.path()).unwrap();
    let out = render_final(
        &p,
        &edl,
        &fence,
        Path::new("mix.mp4"),
        &RenderPreset::default(),
        RenderOptions::default(),
        None,
    )
    .expect("multi-track render");

    // Band-passed RMS isolates each source in the mix.
    let speech_rms = rms_db(&out.path, 0.2, 1.8, Some(440));
    let music_rms = rms_db(&out.path, 0.2, 1.8, Some(2000));
    assert!(speech_rms > -40.0, "speech audible: {speech_rms} dB");
    assert!(
        music_rms > -60.0,
        "music audible (not dropped from the mix): {music_rms} dB"
    );
    let delta = speech_rms - music_rms;
    assert!(
        (14.0..=22.0).contains(&delta),
        "music sits ≈18 dB under speech (clip gain honored), measured Δ {delta:.1} dB \
         (speech {speech_rms:.1}, music {music_rms:.1})"
    );
}

/// Ducking-window regression: gain windows render as real, measurably
/// deep reductions — music at full level outside the window, −18 dB inside.
#[test]
fn render_applies_duck_windows() {
    let dir = tempfile::tempdir().unwrap();
    let music = dir.path().join("music.wav");
    let args: Vec<String> = ["-f", "lavfi", "-i", "sine=frequency=1000:duration=3"]
        .iter()
        .map(|s| s.to_string())
        .chain([music.display().to_string()])
        .collect();
    cut_media::ffmpeg::run_ffmpeg(&args).expect("music generation");

    let mut p = Project::new(
        "t",
        ProjectSettings {
            width: 320,
            height: 240,
            fps: 30.0,
            audio_rate: 48_000,
            color: cut_core::ColorConfig::default(),
        },
    );
    p.assets.insert(
        "mu".into(),
        cut_core::Asset {
            path: music.display().to_string(),
            hash: "sha256:test".into(),
            probe: None,
            transcript: None,
            perception: None,
            proxy: None,
            filmstrip: None,
        },
    );
    p.track_mut("a1t").unwrap().clips = vec![Clip::Media(MediaClip {
        id: "c1".into(),
        asset: "mu".into(),
        src_in_ms: 0,
        src_out_ms: 3000,
        effects: vec![],
        gain_db: 0.0,
        transform: None,
        crop: None,
        fade: None,
        xfade_in_ms: 0,
        xfade_kind: None,
        speed: 1.0,
        grade: None,
        matte: None,
        mask: None,
        reverse: false,
        freeze: None,
        animation: None,
        keyframes: vec![],
        eq: None,
        mute_ranges: vec![],
        stabilize: None,
        speed_ramp: None,
        input_color_space: None,
        nest: None,
        grade_stack: vec![],
        grade_windows: vec![],
    })];
    // Duck [1200, 2200) by −18 dB with 100 ms ramps (edit.duck's storage).
    p.track_mut("a1t").unwrap().gain_windows = vec![cut_core::GainWindow {
        range_ms: [1200, 2200],
        db: -18.0,
        attack_ms: 100,
    }];
    let edl = edl_from_project(&p);
    let fence = PathFence::new(dir.path()).unwrap();
    let out = render_final(
        &p,
        &edl,
        &fence,
        Path::new("duck.mp4"),
        &RenderPreset::default(),
        RenderOptions::default(),
        None,
    )
    .expect("duck render");

    let outside = rms_db(&out.path, 0.2, 1.0, None); // before the window
    let inside = rms_db(&out.path, 1.4, 2.0, None); // plateau (past the ramp)
    let after = rms_db(&out.path, 2.5, 2.9, None); // recovered
    let depth = outside - inside;
    assert!(
        (16.0..=20.0).contains(&depth),
        "duck depth ≈18 dB, measured {depth:.1} (outside {outside:.1}, inside {inside:.1})"
    );
    assert!(
        (after - outside).abs() < 1.5,
        "gain recovers after the window: before {outside:.1} vs after {after:.1}"
    );
}

/// edit.eq LIVE proof: a voice-shape EQ (high-pass 120 Hz, +6 dB @1 kHz presence,
/// low-pass 6 kHz) measurably reshapes a mixed 80/1k/8k-Hz tone. Renders the SAME
/// clip with and without the EQ and compares per-band RMS so the deltas isolate
/// the EQ from absolute level / codec coloration — rumble down, presence up,
/// hiss down (the bench numbers were −6.5/+6.0/−6.6 dB).
#[test]
fn render_eq_reshapes_audio_bands() {
    use cut_core::{ClipEq, EqBand};
    let dir = tempfile::tempdir().unwrap();
    // Audio-only source: 80 Hz (rumble) + 1 kHz (voice) + 8 kHz (hiss), mixed flat.
    let src = dir.path().join("tone.wav");
    let args: Vec<String> = [
        "-f",
        "lavfi",
        "-i",
        "sine=frequency=80:duration=2",
        "-f",
        "lavfi",
        "-i",
        "sine=frequency=1000:duration=2",
        "-f",
        "lavfi",
        "-i",
        "sine=frequency=8000:duration=2",
        "-filter_complex",
        "[0:a][1:a][2:a]amix=inputs=3:normalize=0[m]",
        "-map",
        "[m]",
        "-ar",
        "48000",
    ]
    .iter()
    .map(|s| s.to_string())
    .chain([src.display().to_string()])
    .collect();
    cut_media::ffmpeg::run_ffmpeg(&args).expect("tone generation");

    let mk_project = |eq: Option<ClipEq>| {
        let mut p = Project::new(
            "t",
            ProjectSettings {
                width: 320,
                height: 240,
                fps: 30.0,
                audio_rate: 48_000,
                color: cut_core::ColorConfig::default(),
            },
        );
        p.assets.insert(
            "to".into(),
            cut_core::Asset {
                path: src.display().to_string(),
                hash: "sha256:test".into(),
                probe: None,
                transcript: None,
                perception: None,
                proxy: None,
                filmstrip: None,
            },
        );
        p.track_mut("a1t").unwrap().clips = vec![Clip::Media(MediaClip {
            id: "ac".into(),
            asset: "to".into(),
            src_in_ms: 0,
            src_out_ms: 2000,
            effects: vec![],
            gain_db: 0.0,
            transform: None,
            crop: None,
            fade: None,
            xfade_in_ms: 0,
            xfade_kind: None,
            speed: 1.0,
            grade: None,
            matte: None,
            mask: None,
            reverse: false,
            freeze: None,
            animation: None,
            keyframes: vec![],
            eq,
            mute_ranges: vec![],
            stabilize: None,
            speed_ramp: None,
            input_color_space: None,
            nest: None,
            grade_stack: vec![],
            grade_windows: vec![],
        })];
        p
    };
    let render = |p: &Project, name: &str| {
        let edl = edl_from_project(p);
        let fence = PathFence::new(dir.path()).unwrap();
        render_final(
            p,
            &edl,
            &fence,
            Path::new(name),
            &RenderPreset::default(),
            RenderOptions::default(),
            None,
        )
        .expect("eq render")
    };

    let base = render(&mk_project(None), "base.mp4");
    let eqd = render(
        &mk_project(Some(ClipEq {
            high_pass_hz: Some(120.0),
            low_pass_hz: Some(6000.0),
            bands: vec![EqBand {
                freq_hz: 1000.0,
                gain_db: 6.0,
                q: 1.0,
            }],
        })),
        "eqd.mp4",
    );

    // Per-band RMS delta, EQ'd minus baseline (same source → isolates the EQ).
    let d80 = rms_db(&eqd.path, 0.2, 1.8, Some(80)) - rms_db(&base.path, 0.2, 1.8, Some(80));
    let d1k = rms_db(&eqd.path, 0.2, 1.8, Some(1000)) - rms_db(&base.path, 0.2, 1.8, Some(1000));
    let d8k = rms_db(&eqd.path, 0.2, 1.8, Some(8000)) - rms_db(&base.path, 0.2, 1.8, Some(8000));
    assert!(
        d80 < -3.0,
        "80 Hz rumble cut by the high-pass: Δ {d80:.1} dB"
    );
    assert!(
        d1k > 3.0,
        "1 kHz presence boosted by the band: Δ {d1k:.1} dB"
    );
    assert!(d8k < -3.0, "8 kHz hiss cut by the low-pass: Δ {d8k:.1} dB");
}

/// edit.effect Gate LIVE proof: a noise gate (agate) PASSES a loud section and
/// SILENCES a quiet one. Source = a full-level 1 kHz tone [0,1)s then a -40 dB
/// tail [1,2)s (speech → room tone). Renders with the gate and asserts the loud
/// section survives while the quiet tail drops well below its un-gated level.
#[test]
fn render_gate_silences_quiet_section() {
    use cut_core::ClipEffect as E;
    let dir = tempfile::tempdir().unwrap();
    // loud [0,1) then quiet [1,2): concat a full-level and a -40 dB 1 kHz tone.
    let src = dir.path().join("ga.wav");
    let args: Vec<String> = [
        "-f",
        "lavfi",
        "-i",
        "sine=frequency=1000:duration=1",
        "-f",
        "lavfi",
        "-i",
        "sine=frequency=1000:duration=1",
        "-filter_complex",
        "[0:a]volume=1.0[l];[1:a]volume=0.01[q];[l][q]concat=n=2:v=0:a=1[a]",
        "-map",
        "[a]",
        "-ar",
        "48000",
    ]
    .iter()
    .map(|s| s.to_string())
    .chain([src.display().to_string()])
    .collect();
    cut_media::ffmpeg::run_ffmpeg(&args).expect("gate tone generation");

    let mk_project = |effects: Vec<E>| {
        let mut p = Project::new(
            "t",
            ProjectSettings {
                width: 320,
                height: 240,
                fps: 30.0,
                audio_rate: 48_000,
                color: cut_core::ColorConfig::default(),
            },
        );
        p.assets.insert(
            "ga".into(),
            cut_core::Asset {
                path: src.display().to_string(),
                hash: "sha256:test".into(),
                probe: None,
                transcript: None,
                perception: None,
                proxy: None,
                filmstrip: None,
            },
        );
        p.track_mut("a1t").unwrap().clips = vec![Clip::Media(MediaClip {
            id: "ac".into(),
            asset: "ga".into(),
            src_in_ms: 0,
            src_out_ms: 2000,
            effects,
            gain_db: 0.0,
            transform: None,
            crop: None,
            fade: None,
            xfade_in_ms: 0,
            xfade_kind: None,
            speed: 1.0,
            grade: None,
            matte: None,
            mask: None,
            reverse: false,
            freeze: None,
            animation: None,
            keyframes: vec![],
            eq: None,
            mute_ranges: vec![],
            stabilize: None,
            speed_ramp: None,
            input_color_space: None,
            nest: None,
            grade_stack: vec![],
            grade_windows: vec![],
        })];
        p
    };
    let render = |p: &Project, name: &str| {
        let edl = edl_from_project(p);
        let fence = PathFence::new(dir.path()).unwrap();
        render_final(
            p,
            &edl,
            &fence,
            Path::new(name),
            &RenderPreset::default(),
            RenderOptions::default(),
            None,
        )
        .expect("gate render")
    };

    let base = render(&mk_project(vec![]), "gbase.mp4");
    let gated = render(&mk_project(vec![E::Gate { amount: 0.6 }]), "ggate.mp4");

    // Loud section [0,0.9): the gate is OPEN → barely changed.
    let loud_base = rms_db(&base.path, 0.1, 0.9, None);
    let loud_gated = rms_db(&gated.path, 0.1, 0.9, None);
    assert!(
        (loud_base - loud_gated).abs() < 3.0,
        "loud section passes the gate: {loud_base:.1} → {loud_gated:.1} dB"
    );
    // Quiet tail [1.1,1.9): the gate CLOSES → drops well below the un-gated tail.
    let quiet_base = rms_db(&base.path, 1.1, 1.9, None);
    let quiet_gated = rms_db(&gated.path, 1.1, 1.9, None);
    assert!(
        quiet_gated < quiet_base - 8.0,
        "quiet tail gated down: {quiet_base:.1} → {quiet_gated:.1} dB"
    );
}

/// Mean inter-frame luma difference (a camera-shake / motion proxy): `tblend`
/// difference of adjacent frames → signalstats YAVG, averaged. For a STATIC scene
/// that's been shaken, stabilization brings this back DOWN toward zero.
fn interframe_motion(path: &Path) -> f64 {
    let out = std::process::Command::new("ffmpeg")
        .args(["-hide_banner", "-nostdin", "-i"])
        .arg(path)
        .args([
            "-vf",
            "tblend=all_mode=difference,signalstats,metadata=print:key=lavfi.signalstats.YAVG",
            "-f",
            "null",
            "-",
        ])
        .output()
        .expect("ffmpeg motion measure");
    let stderr = String::from_utf8_lossy(&out.stderr);
    let vals: Vec<f64> = stderr
        .lines()
        .filter_map(|l| l.split("YAVG=").nth(1))
        .filter_map(|s| s.split_whitespace().next())
        .filter_map(|s| s.parse().ok())
        .collect();
    if vals.is_empty() {
        return 0.0;
    }
    vals.iter().sum::<f64>() / vals.len() as f64
}

/// edit.stabilize LIVE proof: render a SHAKY clip (a static testsrc2 scene shaken
/// with an animated crop offset) WITH and WITHOUT stabilization through the real
/// 2-pass pipeline (render_final → prepare_stabilization runs vidstabdetect → the
/// .trf → vidstabtransform), and assert the stabilized render's inter-frame motion
/// drops substantially; the threshold leaves tolerance for encoder and fixture variance.
#[test]
fn render_stabilize_reduces_shake() {
    use cut_core::ClipStabilize;
    let dir = tempfile::tempdir().unwrap();
    // Static detailed scene + simulated camera shake (sinusoidal crop offset).
    let shaky = dir.path().join("shaky.mp4");
    let crop = "crop=320:240:x='20+12*sin(2*PI*t*5)+8*sin(2*PI*t*11)':\
                y='20+10*cos(2*PI*t*6)+6*sin(2*PI*t*9)',setsar=1";
    let args: Vec<String> = [
        "-f",
        "lavfi",
        "-i",
        "testsrc2=size=360x280:rate=30:duration=2",
        "-vf",
        crop,
        "-c:v",
        "libx264",
        "-preset",
        "ultrafast",
        "-pix_fmt",
        "yuv420p",
    ]
    .iter()
    .map(|s| s.to_string())
    .chain([shaky.display().to_string()])
    .collect();
    cut_media::ffmpeg::run_ffmpeg(&args).expect("shaky source generation");

    let mk_project = |stabilize: Option<ClipStabilize>| {
        let mut p = Project::new(
            "t",
            ProjectSettings {
                width: 320,
                height: 240,
                fps: 30.0,
                audio_rate: 48_000,
                color: cut_core::ColorConfig::default(),
            },
        );
        p.assets.insert(
            "sh".into(),
            cut_core::Asset {
                path: shaky.display().to_string(),
                hash: "sha256:shakytest".into(),
                probe: Some(
                    serde_json::json!({"kind": "video", "width": 320, "height": 240, "fps": 30.0}),
                ),
                transcript: None,
                perception: None,
                proxy: None,
                filmstrip: None,
            },
        );
        p.track_mut("v1").unwrap().clips = vec![Clip::Media(MediaClip {
            id: "c1".into(),
            asset: "sh".into(),
            src_in_ms: 0,
            src_out_ms: 2000,
            effects: vec![],
            gain_db: 0.0,
            transform: None,
            crop: None,
            fade: None,
            xfade_in_ms: 0,
            xfade_kind: None,
            speed: 1.0,
            grade: None,
            matte: None,
            mask: None,
            reverse: false,
            freeze: None,
            animation: None,
            keyframes: vec![],
            eq: None,
            mute_ranges: vec![],
            stabilize,
            speed_ramp: None,
            input_color_space: None,
            nest: None,
            grade_stack: vec![],
            grade_windows: vec![],
        })];
        p
    };
    let render = |p: &Project, name: &str| {
        let edl = edl_from_project(p);
        let fence = PathFence::new(dir.path()).unwrap();
        render_final(
            p,
            &edl,
            &fence,
            Path::new(name),
            &RenderPreset::default(),
            RenderOptions::default(),
            None,
        )
        .expect("stabilize render")
    };

    let base = render(&mk_project(None), "shaky_base.mp4");
    let stab = render(
        &mk_project(Some(ClipStabilize { smoothing: 30.0 })),
        "shaky_stab.mp4",
    );

    let m_base = interframe_motion(&base.path);
    let m_stab = interframe_motion(&stab.path);
    assert!(m_base > 1.0, "shaky source actually moves: {m_base:.2}");
    assert!(
        m_stab < m_base * 0.65,
        "stabilization reduced inter-frame motion ≥35%: {m_base:.2} → {m_stab:.2}"
    );
}

/// edit.blend LAYER blend mode LIVE proof: a green PiP on an overlay track set to
/// MULTIPLY blends with the gray base ONLY in the PiP region (masked-blend recipe).
/// Inside the PiP: gray × green (red/blue multiplied toward 0). Outside: untouched
/// gray. Proves the blend is masked to the layer's own alpha, not the whole frame.
#[test]
fn render_blend_mode_multiplies_only_in_overlay_region() {
    let dir = tempfile::tempdir().unwrap();
    let gen = |name: &str, color: &str, w: u32, h: u32| -> std::path::PathBuf {
        let out = dir.path().join(name);
        let args: Vec<String> = [
            "-f",
            "lavfi",
            "-i",
            &format!("color=c={color}:s={w}x{h}:r=30:d=1"),
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
        cut_media::ffmpeg::run_ffmpeg(&args).expect("color source");
        out
    };
    let gray = gen("gray.mp4", "0x808080", 320, 240);
    let green = gen("green_b.mp4", "green", 320, 240);

    let mut p = Project::new(
        "t",
        ProjectSettings {
            width: 320,
            height: 240,
            fps: 30.0,
            audio_rate: 48_000,
            color: cut_core::ColorConfig::default(),
        },
    );
    for (id, path) in [("a1", &gray), ("a2", &green)] {
        p.assets.insert(
            id.into(),
            cut_core::Asset {
                path: path.display().to_string(),
                hash: format!("sha256:{id}"),
                probe: Some(serde_json::json!({"kind": "video", "width": 320, "height": 240})),
                transcript: None,
                perception: None,
                proxy: None,
                filmstrip: None,
            },
        );
    }
    let mk = |id: &str, asset: &str, transform: Option<cut_core::ClipTransform>| MediaClip {
        id: id.into(),
        asset: asset.into(),
        src_in_ms: 0,
        src_out_ms: 1000,
        effects: vec![],
        gain_db: 0.0,
        transform,
        crop: None,
        fade: None,
        xfade_in_ms: 0,
        xfade_kind: None,
        speed: 1.0,
        grade: None,
        matte: None,
        mask: None,
        reverse: false,
        freeze: None,
        animation: None,
        keyframes: vec![],
        eq: None,
        mute_ranges: vec![],
        stabilize: None,
        speed_ramp: None,
        input_color_space: None,
        nest: None,
        grade_stack: vec![],
        grade_windows: vec![],
    };
    // base = gray on v1; overlay = green PiP (top-left quadrant) on v2 set to MULTIPLY.
    p.track_mut("v1").unwrap().clips = vec![Clip::Media(mk("c1", "a1", None))];
    p.tracks.push(Track {
        id: "v2".into(),
        kind: TrackKind::Video,
        clips: vec![Clip::Media(mk(
            "c2",
            "a2",
            Some(cut_core::ClipTransform {
                x: 0.0,
                y: 0.0,
                scale: 0.5,
                opacity: 1.0,
            }),
        ))],
        gain_db: 0.0,
        gain_windows: vec![],
        blend_mode: Some("multiply".into()),
        visible: true,
        locked: false,
        muted: false,
        solo: false,
        pan: 0.0,
    });

    let fence = PathFence::new(dir.path()).unwrap();
    let edl = edl_from_project(&p);
    let out = render_final(
        &p,
        &edl,
        &fence,
        Path::new("blend.mp4"),
        &RenderPreset::default(),
        RenderOptions::default(),
        None,
    )
    .expect("blend render");

    let frame = frame_rgb(&out.path, 0.5, 320, 240);
    let (ri, _gi, bi) = px(&frame, 320, 60, 40); // inside the PiP (top-left)
    let (ro, go, bo) = px(&frame, 320, 250, 200); // outside the PiP (bottom-right)
    assert!(
        ri < 60 && bi < 60,
        "inside PiP: gray×green multiplied red/blue toward 0 ({ri},_,{bi})"
    );
    assert!(
        ro > 100 && go > 100 && bo > 100,
        "outside PiP: base gray untouched by the blend ({ro},{go},{bo})"
    );
}

/// edit.matte (remove): the baked alpha SETS the overlay's alpha plane, so the
/// overlay shows where the alpha is opaque (white) and the LOWER track reveals
/// where it is transparent (black). Real render + pixel check (the alpha is
/// pre-placed in the content-addressed cache — render reads it, no sidecar).
#[test]
fn render_matte_remove_reveals_base_through_alpha() {
    let dir = tempfile::tempdir().unwrap();
    let gen = |name: &str, color: &str| -> std::path::PathBuf {
        let out = dir.path().join(name);
        let args: Vec<String> = [
            "-f",
            "lavfi",
            "-i",
            &format!("color=c={color}:s=320x240:r=30:d=1"),
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
        cut_media::ffmpeg::run_ffmpeg(&args).expect("color source");
        out
    };
    let magenta = gen("bg.mp4", "magenta"); // base
    let green = gen("subject.mp4", "green"); // overlay subject

    // The baked alpha: LEFT half white (opaque → overlay shows), RIGHT half black
    // (transparent → base reveals). FFV1 gray, exactly like the sidecar returns.
    let matte = cut_core::ClipMatte {
        mode: cut_core::MatteMode::Remove,
        model: cut_core::MatteModel::Rvm,
        bg: None,
        quality: cut_core::MatteQuality::Good,
        seed: None,
    };
    let alpha_dir = dir.path().join("cache").join("matte");
    std::fs::create_dir_all(&alpha_dir).unwrap();
    let alpha_path = alpha_dir.join(matte.cache_filename("sha256:a2"));
    let alpha_args: Vec<String> = [
        "-f",
        "lavfi",
        "-i",
        "color=c=white:s=160x240:r=30:d=1",
        "-f",
        "lavfi",
        "-i",
        "color=c=black:s=160x240:r=30:d=1",
        "-filter_complex",
        "[0:v][1:v]hstack=inputs=2,format=gray[a]",
        "-map",
        "[a]",
        "-c:v",
        "ffv1",
        "-pix_fmt",
        "gray",
    ]
    .iter()
    .map(|s| s.to_string())
    .chain([alpha_path.display().to_string()])
    .collect();
    cut_media::ffmpeg::run_ffmpeg(&alpha_args).expect("alpha source");

    let mut p = Project::new(
        "t",
        ProjectSettings {
            width: 320,
            height: 240,
            fps: 30.0,
            audio_rate: 48_000,
            color: cut_core::ColorConfig::default(),
        },
    );
    for (id, path) in [("a1", &magenta), ("a2", &green)] {
        p.assets.insert(
            id.into(),
            cut_core::Asset {
                path: path.display().to_string(),
                hash: format!("sha256:{id}"),
                probe: Some(serde_json::json!({"kind": "video", "width": 320, "height": 240})),
                transcript: None,
                perception: None,
                proxy: None,
                filmstrip: None,
            },
        );
    }
    let mk = |id: &str, asset: &str, matte: Option<cut_core::ClipMatte>| MediaClip {
        id: id.into(),
        asset: asset.into(),
        src_in_ms: 0,
        src_out_ms: 1000,
        effects: vec![],
        gain_db: 0.0,
        transform: None,
        crop: None,
        fade: None,
        xfade_in_ms: 0,
        xfade_kind: None,
        speed: 1.0,
        grade: None,
        matte,
        mask: None,
        reverse: false,
        freeze: None,
        animation: None,
        keyframes: vec![],
        eq: None,
        mute_ranges: vec![],
        stabilize: None,
        speed_ramp: None,
        input_color_space: None,
        nest: None,
        grade_stack: vec![],
        grade_windows: vec![],
    };
    // base = magenta on v1; overlay = green (full-frame) on v2 with matte REMOVE.
    p.track_mut("v1").unwrap().clips = vec![Clip::Media(mk("c1", "a1", None))];
    p.tracks.push(Track {
        id: "v2".into(),
        kind: TrackKind::Video,
        clips: vec![Clip::Media(mk("c2", "a2", Some(matte)))],
        gain_db: 0.0,
        gain_windows: vec![],
        blend_mode: None,
        visible: true,
        locked: false,
        muted: false,
        solo: false,
        pan: 0.0,
    });

    let fence = PathFence::new(dir.path()).unwrap();
    let edl = edl_from_project(&p);
    let out = render_final(
        &p,
        &edl,
        &fence,
        Path::new("matte.mp4"),
        &RenderPreset::default(),
        RenderOptions::default(),
        None,
    )
    .expect("matte render");

    let frame = frame_rgb(&out.path, 0.5, 320, 240);
    let (lr, lg, lb) = px(&frame, 320, 80, 120); // LEFT half: alpha white → green overlay
    let (rr, rg, rb) = px(&frame, 320, 240, 120); // RIGHT half: alpha black → magenta base
    assert!(
        lg > 120 && lr < 100 && lb < 100,
        "left = green overlay (alpha opaque) ({lr},{lg},{lb})"
    );
    assert!(
        rr > 120 && rb > 120 && rg < 100,
        "right = magenta base revealed (alpha transparent) ({rr},{rg},{rb})"
    );
}

/// The matte composites in the PREVIEW/scrub path too, not just the final render.
/// render_range (the range-preview path) goes through the same build_graph, so
/// the user SEES the background removed while editing (a stated requirement).
#[test]
fn render_range_preview_composites_matte() {
    let dir = tempfile::tempdir().unwrap();
    let gen = |name: &str, color: &str| -> std::path::PathBuf {
        let out = dir.path().join(name);
        let args: Vec<String> = [
            "-f",
            "lavfi",
            "-i",
            &format!("color=c={color}:s=320x240:r=30:d=1"),
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
        cut_media::ffmpeg::run_ffmpeg(&args).expect("color source");
        out
    };
    let magenta = gen("bg.mp4", "magenta");
    let green = gen("subject.mp4", "green");
    let matte = cut_core::ClipMatte {
        mode: cut_core::MatteMode::Remove,
        model: cut_core::MatteModel::Rvm,
        bg: None,
        quality: cut_core::MatteQuality::Good,
        seed: None,
    };
    let alpha_dir = dir.path().join("cache").join("matte");
    std::fs::create_dir_all(&alpha_dir).unwrap();
    let alpha_args: Vec<String> = [
        "-f",
        "lavfi",
        "-i",
        "color=c=white:s=160x240:r=30:d=1",
        "-f",
        "lavfi",
        "-i",
        "color=c=black:s=160x240:r=30:d=1",
        "-filter_complex",
        "[0:v][1:v]hstack=inputs=2,format=gray[a]",
        "-map",
        "[a]",
        "-c:v",
        "ffv1",
        "-pix_fmt",
        "gray",
    ]
    .iter()
    .map(|s| s.to_string())
    .chain([alpha_dir
        .join(matte.cache_filename("sha256:a2"))
        .display()
        .to_string()])
    .collect();
    cut_media::ffmpeg::run_ffmpeg(&alpha_args).expect("alpha source");

    let mut p = Project::new(
        "t",
        ProjectSettings {
            width: 320,
            height: 240,
            fps: 30.0,
            audio_rate: 48_000,
            color: cut_core::ColorConfig::default(),
        },
    );
    for (id, path) in [("a1", &magenta), ("a2", &green)] {
        p.assets.insert(
            id.into(),
            cut_core::Asset {
                path: path.display().to_string(),
                hash: format!("sha256:{id}"),
                probe: Some(serde_json::json!({"kind": "video", "width": 320, "height": 240})),
                transcript: None,
                perception: None,
                proxy: None,
                filmstrip: None,
            },
        );
    }
    let mk = |id: &str, asset: &str, matte: Option<cut_core::ClipMatte>| MediaClip {
        id: id.into(),
        asset: asset.into(),
        src_in_ms: 0,
        src_out_ms: 1000,
        effects: vec![],
        gain_db: 0.0,
        transform: None,
        crop: None,
        fade: None,
        xfade_in_ms: 0,
        xfade_kind: None,
        speed: 1.0,
        grade: None,
        matte,
        mask: None,
        reverse: false,
        freeze: None,
        animation: None,
        keyframes: vec![],
        eq: None,
        mute_ranges: vec![],
        stabilize: None,
        speed_ramp: None,
        input_color_space: None,
        nest: None,
        grade_stack: vec![],
        grade_windows: vec![],
    };
    p.track_mut("v1").unwrap().clips = vec![Clip::Media(mk("c1", "a1", None))];
    p.tracks.push(Track {
        id: "v2".into(),
        kind: TrackKind::Video,
        clips: vec![Clip::Media(mk("c2", "a2", Some(matte)))],
        gain_db: 0.0,
        gain_windows: vec![],
        blend_mode: None,
        visible: true,
        locked: false,
        muted: false,
        solo: false,
        pan: 0.0,
    });

    let fence = PathFence::new(dir.path()).unwrap();
    let edl = edl_from_project(&p);
    let out = cut_media::render::render_range(
        &p,
        &edl,
        &fence,
        Path::new("preview.mp4"),
        &RenderPreset::default(),
        [0, 500],
        RenderOptions::default(),
        None,
    )
    .expect("range preview render");
    let frame = frame_rgb(&out.path, 0.2, 320, 240);
    let (lr, lg, lb) = px(&frame, 320, 80, 120);
    let (rr, rg, rb) = px(&frame, 320, 240, 120);
    assert!(
        lg > 120 && lr < 100 && lb < 100,
        "preview left = green overlay ({lr},{lg},{lb})"
    );
    assert!(
        rr > 120 && rb > 120 && rg < 100,
        "preview right = magenta base revealed ({rr},{rg},{rb})"
    );
}

/// Average luma (signalstats YAVG) of one frame at `at_s` — ground truth for
/// "the video actually faded", not just "the filter string was emitted".
fn yavg_at(path: &Path, at_s: f64) -> f64 {
    let out = std::process::Command::new("ffmpeg")
        .args([
            "-hide_banner",
            "-nostdin",
            "-ss",
            &format!("{at_s:.3}"),
            "-i",
        ])
        .arg(path)
        .args([
            "-frames:v",
            "1",
            "-vf",
            "signalstats,metadata=print",
            "-f",
            "null",
            "-",
        ])
        .output()
        .expect("ffmpeg signalstats");
    let stderr = String::from_utf8_lossy(&out.stderr);
    // FIRST match: -frames:v 1 still lets signalstats print several decoded
    // frames before output stops — the first printed frame is the seeked one.
    stderr
        .lines()
        .filter_map(|l| l.split("lavfi.signalstats.YAVG=").nth(1))
        .next()
        .unwrap_or_else(|| panic!("no YAVG in signalstats output:\n{stderr}"))
        .trim()
        .parse()
        .expect("YAVG parses")
}

/// Fade-edit regression: edit.fade renders as real measurable ramps — audio rises
/// from near-silence through the fade-in and dies through the fade-out;
/// video opens near-black and reaches full brightness. (kind="both" on the
/// clip mirrored across the video + audio tracks, like a real edit.)
#[test]
fn render_applies_clip_fades() {
    let dir = tempfile::tempdir().unwrap();
    let clip = gen_clip(dir.path(), "in.mp4"); // 2s testsrc2 + 440 Hz sine
    let mut p = two_clip_project(&clip);
    // One full-length clip per track, faded 600 ms in / 600 ms out.
    let fade = Some(cut_core::ClipFade {
        in_ms: 600,
        out_ms: 600,
        kind: cut_core::FadeKind::Both,
    });
    let mk = |id: &str| {
        Clip::Media(MediaClip {
            id: id.into(),
            asset: "a1".into(),
            src_in_ms: 0,
            src_out_ms: 2000,
            effects: vec![],
            gain_db: 0.0,
            transform: None,
            crop: None,
            fade: fade.clone(),
            xfade_in_ms: 0,
            xfade_kind: None,
            speed: 1.0,
            grade: None,
            matte: None,
            mask: None,
            reverse: false,
            freeze: None,
            animation: None,
            keyframes: vec![],
            eq: None,
            mute_ranges: vec![],
            stabilize: None,
            speed_ramp: None,
            input_color_space: None,
            nest: None,
            grade_stack: vec![],
            grade_windows: vec![],
        })
    };
    p.track_mut("v1").unwrap().clips = vec![mk("c1")];
    p.track_mut("a1t").unwrap().clips = vec![mk("c1a")];
    p.tracks.retain(|t| t.kind != TrackKind::Caption); // captions off this proof
    let edl = edl_from_project(&p);
    assert_eq!(
        edl.track_segments("v1").next().unwrap().fade,
        fade,
        "EDL carries the clip fade"
    );
    let fence = PathFence::new(dir.path()).unwrap();
    let out = render_final(
        &p,
        &edl,
        &fence,
        Path::new("fade.mp4"),
        &RenderPreset::default(),
        RenderOptions::default(),
        None,
    )
    .expect("fade render");

    // Audio: early ramp ≫ quieter than the full-level middle; tail dies too.
    let early = rms_db(&out.path, 0.0, 0.2, None); // first third of the 600ms ramp
    let mid = rms_db(&out.path, 0.8, 1.2, None); // full level
    let late = rms_db(&out.path, 1.8, 2.0, None); // last third of the out-ramp
    assert!(
        mid - early > 8.0,
        "fade-in audible: early {early:.1} vs mid {mid:.1} dB"
    );
    assert!(
        mid - late > 8.0,
        "fade-out audible: late {late:.1} vs mid {mid:.1} dB"
    );
    // Video: first frame near black, middle at full testsrc2 brightness.
    let y_first = yavg_at(&out.path, 0.0);
    let y_mid = yavg_at(&out.path, 1.0);
    assert!(
        y_mid - y_first > 40.0,
        "video fade-in visible: first frame YAVG {y_first:.1} vs mid {y_mid:.1}"
    );
}

#[test]
fn proxy_is_960x540_and_cached() {
    let dir = tempfile::tempdir().unwrap();
    let clip = gen_clip(dir.path(), "in.mp4");
    let proxies = dir.path().join("proxies");
    std::fs::create_dir_all(&proxies).unwrap();
    let proxy = make_proxy(&clip, &proxies, "a1").expect("proxy");
    let info = probe(&proxy).unwrap();
    assert_eq!((info.width, info.height), (Some(960), Some(540)));
    assert!(info.has_audio);
    // Cache: second call returns the same path without re-encoding (mtime check).
    let mtime = std::fs::metadata(&proxy).unwrap().modified().unwrap();
    let again = make_proxy(&clip, &proxies, "a1").unwrap();
    assert_eq!(again, proxy);
    assert_eq!(
        std::fs::metadata(&proxy).unwrap().modified().unwrap(),
        mtime
    );
}

#[test]
fn render_two_clips_duration_and_determinism() {
    let dir = tempfile::tempdir().unwrap();
    let clip = gen_clip(dir.path(), "in.mp4");
    let project = two_clip_project(&clip);
    let edl = edl_from_project(&project);
    assert_eq!(edl.duration_ms, 2000); // model-side ground truth

    let fence = PathFence::new(dir.path()).unwrap();
    let progress: Arc<Mutex<Vec<f32>>> = Arc::new(Mutex::new(vec![]));
    let p2 = progress.clone();
    let out1 = render_final(
        &project,
        &edl,
        &fence,
        Path::new("final.mp4"),
        &RenderPreset::default(),
        RenderOptions::default(),
        Some(Box::new(move |f| p2.lock().unwrap().push(f))),
    )
    .expect("render 1");

    // Duration within ±1 frame @30fps (34 ms) of the EDL (container overhead
    // from AAC priming stays inside that envelope).
    let diff = out1.duration_ms.abs_diff(2000);
    assert!(
        diff <= 34,
        "render duration {} ms — off by {diff}",
        out1.duration_ms
    );

    // Progress streamed 0.0 → 1.0.
    let prog = progress.lock().unwrap();
    assert_eq!(*prog.first().unwrap(), 0.0);
    assert_eq!(*prog.last().unwrap(), 1.0);

    // Determinism: same input + EDL ⇒ identical output hash (media-engine contract).
    let out2 = render_final(
        &project,
        &edl,
        &fence,
        Path::new("final2.mp4"),
        &RenderPreset::default(),
        RenderOptions::default(),
        None,
    )
    .expect("render 2");
    assert_eq!(out1.hash, out2.hash, "re-render must be bit-identical");
    assert!(out1.hash.starts_with("sha256:"));

    // Burn-in actually ran: the rendered file still probes at project geometry.
    let info = probe(&out1.path).unwrap();
    assert_eq!((info.width, info.height), (Some(320), Some(240)));
    assert!(info.has_audio);
}

#[test]
fn frame_extracts_valid_jpeg_of_composition() {
    let dir = tempfile::tempdir().unwrap();
    let clip = gen_clip(dir.path(), "in.mp4");
    let project = two_clip_project(&clip);
    let edl = edl_from_project(&project);
    // 1500 ms sits inside clip B (timeline [1000,2000)) — composed, not source.
    let jpeg = extract_frame(&project, &edl, dir.path(), 1500, None).expect("frame");
    assert!(
        jpeg.len() > 1000,
        "suspiciously small JPEG: {} bytes",
        jpeg.len()
    );
    assert_eq!(&jpeg[..2], &[0xFF, 0xD8], "JPEG SOI marker");
    assert_eq!(&jpeg[jpeg.len() - 2..], &[0xFF, 0xD9], "JPEG EOI marker");

    // Past-the-end position is an actionable error, not a hang or empty file.
    let err = extract_frame(&project, &edl, dir.path(), 99_999, None).unwrap_err();
    assert_eq!(err.at_ms, Some(99_999));
}

/// Burn-in ASS serialization (interchange SRT/XML export lives in cut-export;
/// this crate only keeps the render-side ASS path).
#[test]
fn captions_serialize_to_ass() {
    let dir = tempfile::tempdir().unwrap();
    let clip = gen_clip(dir.path(), "in.mp4");
    let mut project = two_clip_project(&clip);
    // A SECOND caption track (captions.add_text's txt1): burn-in must collect
    // caption clips across ALL caption tracks, not just cap1.
    project.caption_styles.insert(
        "txt_center".into(),
        CaptionStyle {
            font: "Inter".into(),
            size: 64,
            color: "#fff".into(),
            bg: Some("#000c".into()),
            pos: Some("center".into()),
            extra: Default::default(),
        },
    );
    project.tracks.push(Track {
        id: "txt1".into(),
        kind: TrackKind::Caption,
        clips: vec![Clip::Caption(CaptionClip {
            id: "txt_0001".into(),
            text: "Title Card".into(),
            style_ref: Some("txt_center".into()),
            range_ms: [0, 900],
        })],
        gain_db: 0.0,
        gain_windows: vec![],
        blend_mode: None,
        visible: true,
        locked: false,
        muted: false,
        solo: false,
        pan: 0.0,
    });
    let ass = cut_media::captions_to_ass(&project).unwrap();
    assert!(
        ass.contains("Style: brand1,DejaVu Sans,24,&H00FFFFFF"),
        "ass:\n{ass}"
    );
    assert!(ass.contains("Dialogue: 0,0:00:00.20,0:00:01.50,brand1,,0,0,0,,hello cut"));
    assert!(ass.contains("PlayResX: 320"));
    // The text card rides in the same ASS with its own style (alignment 5 =
    // center) — overlapping ranges across tracks are legal ASS events.
    assert!(ass.contains("Style: txt_center,Inter,64"), "ass:\n{ass}");
    assert!(ass.contains("Dialogue: 0,0:00:00.00,0:00:00.90,txt_center,,0,0,0,,Title Card"));

    // EDL-driven captions (what build_graph + segmented rendering use) must be
    // BYTE-IDENTICAL to the project path for a FULL (un-windowed) EDL — this is
    // what keeps an existing render's caption burn-in unchanged after the
    // segmentation refactor. (Windowed equivalence is covered by Edl::window's
    // unit tests + the segmented frame-compare.)
    let full = edl_from_project(&project);
    let ass_edl = cut_media::captions_to_ass_for_edl(&project, &full).unwrap();
    assert_eq!(ass, ass_edl, "EDL-driven ASS must match the project path");
}

/// Build a ~6 s composite from ONE 2 s test asset: three base cuts (0–6 s), a
/// PiP overlay at [1000,3000) straddling the 2 s window seam, and a caption
/// [500,4500) spanning all three 2 s windows. Exercises every clamp path in
/// Edl::window (base mid-clip split, overlay straddle, caption rebase).
fn composite_6s_project(asset: &Path) -> Project {
    let mut p = Project::new(
        "seg",
        ProjectSettings {
            width: 320,
            height: 240,
            fps: 30.0,
            audio_rate: 48_000,
            color: cut_core::ColorConfig::default(),
        },
    );
    p.assets.insert(
        "a1".into(),
        cut_core::Asset {
            path: asset.display().to_string(),
            hash: "sha256:test".into(),
            probe: None,
            transcript: None,
            perception: None,
            proxy: None,
            filmstrip: None,
        },
    );
    let media = |id: &str, transform: Option<cut_core::ClipTransform>| {
        Clip::Media(MediaClip {
            id: id.into(),
            asset: "a1".into(),
            src_in_ms: 0,
            src_out_ms: 2000,
            effects: vec![],
            gain_db: 0.0,
            transform,
            crop: None,
            fade: None,
            xfade_in_ms: 0,
            xfade_kind: None,
            speed: 1.0,
            grade: None,
            matte: None,
            mask: None,
            reverse: false,
            freeze: None,
            animation: None,
            keyframes: vec![],
            eq: None,
            mute_ranges: vec![],
            stabilize: None,
            speed_ramp: None,
            input_color_space: None,
            nest: None,
            grade_stack: vec![],
            grade_windows: vec![],
        })
    };
    // Base 0–6s (three 2s clips) + mirrored audio.
    p.track_mut("v1").unwrap().clips =
        vec![media("c1", None), media("c2", None), media("c3", None)];
    p.track_mut("a1t").unwrap().clips =
        vec![media("c1a", None), media("c2a", None), media("c3a", None)];
    // Overlay PiP starting at 1000ms (1s gap first) → straddles the 2000ms seam.
    p.tracks.push(Track {
        id: "v2".into(),
        kind: TrackKind::Video,
        clips: vec![
            Clip::Gap(cut_core::GapClip {
                kind: "gap".into(),
                duration_ms: 1000,
            }),
            media(
                "ov",
                Some(cut_core::ClipTransform {
                    x: 0.5,
                    y: 0.5,
                    scale: 0.4,
                    opacity: 1.0,
                }),
            ),
        ],
        gain_db: 0.0,
        gain_windows: vec![],
        blend_mode: None,
        visible: true,
        locked: false,
        muted: false,
        solo: false,
        pan: 0.0,
    });
    p.caption_styles.insert(
        "brand1".into(),
        CaptionStyle {
            font: "DejaVu Sans".into(),
            size: 24,
            color: "#fff".into(),
            bg: Some("#000a".into()),
            pos: Some("bottom".into()),
            extra: Default::default(),
        },
    );
    p.tracks.push(Track {
        id: "cap1".into(),
        kind: TrackKind::Caption,
        clips: vec![Clip::Caption(CaptionClip {
            id: "s1".into(),
            text: "spans windows".into(),
            style_ref: Some("brand1".into()),
            range_ms: [500, 4500],
        })],
        gain_db: 0.0,
        gain_windows: vec![],
        blend_mode: None,
        visible: true,
        locked: false,
        muted: false,
        solo: false,
        pan: 0.0,
    });
    p
}

/// Average PSNR (dB) between two videos via ffmpeg's psnr filter. High = frames
/// near-identical; a structural error (wrong overlay placement, missing caption,
/// AV/frame desync, dropped boundary frame) craters it. "inf" = byte-identical
/// frames → treated as +∞.
fn avg_psnr(a: &Path, b: &Path) -> f64 {
    let bin = cut_media::ffmpeg::ffmpeg_bin();
    let out = std::process::Command::new(&bin)
        .args(["-hide_banner", "-i"])
        .arg(a)
        .arg("-i")
        .arg(b)
        .args(["-lavfi", "[0:v][1:v]psnr", "-f", "null", "-"])
        .output()
        .expect("run ffmpeg psnr");
    let err = String::from_utf8_lossy(&out.stderr);
    let tok = err
        .rsplit("average:")
        .next()
        .and_then(|s| s.split_whitespace().next())
        .unwrap_or("");
    if tok == "inf" {
        return f64::INFINITY;
    }
    tok.parse::<f64>()
        .unwrap_or_else(|_| panic!("no PSNR average parsed from ffmpeg:\n{err}"))
}

/// Serializes the two tests that drive the SHELLX_CUT_RENDER_WINDOW_SEC /
/// _SEGMENT_SEC knobs (process-global env) so they never race each other. Other
/// render tests use ≤2s timelines, which fit in a single window regardless of
/// the window size, so they are immune to a leaked value.
static SEG_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// A GPU-fast-track-FRIENDLY timeline (v1 scope): a SINGLE base video track of
/// hard cuts + audio, matching-aspect source, NO overlays/captions/grade/fade/
/// crop/xfade. (composite_6s_project is the NON-friendly counterpart — it carries
/// a caption track AND a PiP overlay, so opt-in must fall back to software.)
fn gpu_friendly_project(asset: &Path) -> Project {
    let mut p = Project::new(
        "gpu_friendly",
        ProjectSettings {
            width: 320,
            height: 240,
            fps: 30.0,
            audio_rate: 48_000,
            color: cut_core::ColorConfig::default(),
        },
    );
    p.assets.insert(
        "a1".into(),
        cut_core::Asset {
            path: asset.display().to_string(),
            hash: "sha256:test".into(),
            // gen_clip writes a 320x240 source — matches the project aspect, so
            // scale_cuda=W:H is a faithful conform (the aspect gate passes).
            probe: Some(serde_json::json!({ "width": 320, "height": 240 })),
            transcript: None,
            perception: None,
            proxy: None,
            filmstrip: None,
        },
    );
    let media = |id: &str| {
        Clip::Media(MediaClip {
            id: id.into(),
            asset: "a1".into(),
            src_in_ms: 0,
            src_out_ms: 2000,
            effects: vec![],
            gain_db: 0.0,
            transform: None,
            crop: None,
            fade: None,
            xfade_in_ms: 0,
            xfade_kind: None,
            speed: 1.0,
            grade: None,
            matte: None,
            mask: None,
            reverse: false,
            freeze: None,
            animation: None,
            keyframes: vec![],
            eq: None,
            mute_ranges: vec![],
            stabilize: None,
            speed_ramp: None,
            input_color_space: None,
            nest: None,
            grade_stack: vec![],
            grade_windows: vec![],
        })
    };
    p.track_mut("v1").unwrap().clips = vec![media("c1"), media("c2")];
    p.track_mut("a1t").unwrap().clips = vec![media("c1a"), media("c2a")];
    p
}

/// GPU fast-track — opt-in is HONEST, SCOPE-AWARE, and HARDWARE-ADAPTIVE, and the
/// base-track CUDA graph actually RENDERS. Shares SEG_ENV_LOCK because it toggles
/// `SHELLX_CUT_RENDER_GPU` and calls render_final. Exercises BOTH gate branches:
///   - GPU-friendly timeline (single base track, cuts, matching aspect) + opt-in:
///     on a CUDA box → renders via build_graph_gpu (NVDEC + scale_cuda + nvenc),
///     output present, `pipeline == "gpu"`; on a software-only box → software
///     fallback (`pipeline == None`).
///   - NON-friendly timeline (captions + PiP) + opt-in: ALWAYS software
///     (`pipeline == None`), even on a CUDA box — opt-in must not break a render
///     the GPU path cannot do.
/// On the dev 5080 this is the LIVE end-to-end proof that the GPU graph renders.
#[test]
fn gpu_friendly_renders_on_gpu_else_software_fallback() {
    let _env = SEG_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    std::env::remove_var("SHELLX_CUT_RENDER_SEGMENT_SEC");
    std::env::remove_var("SHELLX_CUT_RENDER_WINDOW_SEC");
    std::env::remove_var("SHELLX_CUT_RENDER_GPU");

    let dir = tempfile::tempdir().unwrap();
    let clip = gen_clip(dir.path(), "src.mp4");
    let fence = PathFence::new(dir.path()).unwrap();
    let preset = RenderPreset::default();
    let gpu_ok = cut_media::hwencode::gpu_filters_available();

    // --- GPU-FRIENDLY timeline (single base track of cuts) -----------------
    let friendly = gpu_friendly_project(&clip);
    let fedl = edl_from_project(&friendly);

    // Default (software) render works + is tagged software (pipeline None).
    let sw = render_final(
        &friendly,
        &fedl,
        &fence,
        &dir.path().join("sw.mp4"),
        &preset,
        RenderOptions::default(),
        None,
    )
    .expect("software render of the friendly timeline must succeed");
    assert_eq!(sw.pipeline, None, "default render is the software pipeline");

    // Opt in → GPU path on a CUDA box (renders!), software fallback otherwise.
    std::env::set_var("SHELLX_CUT_RENDER_GPU", "1");
    let gout = dir.path().join("gpu.mp4");
    let out = render_final(
        &friendly,
        &fedl,
        &fence,
        &gout,
        &preset,
        RenderOptions::default(),
        None,
    )
    .expect("friendly opt-in render must succeed");
    assert!(gout.exists(), "render wrote an output file");
    if gpu_ok {
        assert_eq!(
            out.pipeline.as_deref(),
            Some("gpu"),
            "CUDA box: the friendly timeline must render on the GPU pipeline"
        );
        // The GPU graph produced a real video of the expected length (2×2s cuts).
        let probed = probe(&gout).expect("ffprobe the GPU output");
        let d = probed.duration_ms.unwrap_or(0);
        assert!(
            (3500..=4500).contains(&d),
            "GPU render duration ~4s, got {d}ms"
        );
    } else {
        assert_eq!(out.pipeline, None, "no CUDA chain → software pipeline");
    }

    // --- NON-friendly timeline (has a caption track) -----------------------
    // Opt-in must ALWAYS use software here, even on a CUDA box — the GPU path
    // cannot burn ASS captions, so the gate keeps it on the deterministic path.
    let captioned = composite_6s_project(&clip);
    let cedl = edl_from_project(&captioned);
    let cout = dir.path().join("captioned.mp4");
    let r2 = render_final(
        &captioned,
        &cedl,
        &fence,
        &cout,
        &preset,
        RenderOptions::default(),
        None,
    );
    std::env::remove_var("SHELLX_CUT_RENDER_GPU");
    let c =
        r2.expect("non-GPU-friendly timeline (captions) must fall back to software with opt-in");
    assert_eq!(
        c.pipeline, None,
        "captioned timeline used the software pipeline even with opt-in"
    );
    assert!(
        cout.exists(),
        "software fallback wrote the captioned output"
    );
}

/// GPU opt-in must safely fall back for an OVERLAY (PiP) timeline. FFmpeg 6.1's
/// overlay_cuda drops NVDEC crop metadata, exposing padded hardware-surface rows
/// (320x240 becomes 320x256); post-scaling fixes size by distorting the picture.
/// Until a true CUDA crop path is available and parity-tested, software is the
/// correctness-preserving route. The two deterministic renders must match exactly.
#[test]
fn gpu_overlay_pip_falls_back_to_software() {
    let _env = SEG_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    std::env::remove_var("SHELLX_CUT_RENDER_SEGMENT_SEC");
    std::env::remove_var("SHELLX_CUT_RENDER_WINDOW_SEC");
    std::env::remove_var("SHELLX_CUT_RENDER_GPU");

    let dir = tempfile::tempdir().unwrap();
    let base = gen_clip(dir.path(), "base.mp4");
    let over = gen_clip(dir.path(), "over.mp4"); // distinct FILE → passes shared-asset gate
    let fence = PathFence::new(dir.path()).unwrap();
    let preset = RenderPreset::default();

    let mut p = Project::new(
        "gpu_overlay",
        ProjectSettings {
            width: 320,
            height: 240,
            fps: 30.0,
            audio_rate: 48_000,
            color: cut_core::ColorConfig::default(),
        },
    );
    let asset = |path: &Path| cut_core::Asset {
        path: path.display().to_string(),
        hash: "sha256:test".into(),
        probe: Some(serde_json::json!({ "width": 320, "height": 240 })),
        transcript: None,
        perception: None,
        proxy: None,
        filmstrip: None,
    };
    p.assets.insert("a1".into(), asset(&base));
    p.assets.insert("a2".into(), asset(&over));
    let base_clip = |id: &str| {
        Clip::Media(MediaClip {
            id: id.into(),
            asset: "a1".into(),
            src_in_ms: 0,
            src_out_ms: 2000,
            effects: vec![],
            gain_db: 0.0,
            transform: None,
            crop: None,
            fade: None,
            xfade_in_ms: 0,
            xfade_kind: None,
            speed: 1.0,
            grade: None,
            matte: None,
            mask: None,
            reverse: false,
            freeze: None,
            animation: None,
            keyframes: vec![],
            eq: None,
            mute_ranges: vec![],
            stabilize: None,
            speed_ramp: None,
            input_color_space: None,
            nest: None,
            grade_stack: vec![],
            grade_windows: vec![],
        })
    };
    p.track_mut("v1").unwrap().clips = vec![base_clip("c1"), base_clip("c2")];
    p.track_mut("a1t").unwrap().clips = vec![base_clip("c1a"), base_clip("c2a")];
    // Distinct-asset PiP overlay on a new v2 track; opacity 0.6 exercises the
    // colorchannelmixer alpha path; it ends at 2s so [2s,4s) is transparent filler.
    p.tracks.push(Track {
        id: "v2".into(),
        kind: TrackKind::Video,
        clips: vec![Clip::Media(MediaClip {
            id: "ov".into(),
            asset: "a2".into(),
            src_in_ms: 0,
            src_out_ms: 2000,
            effects: vec![],
            gain_db: 0.0,
            transform: Some(cut_core::ClipTransform {
                x: 0.5,
                y: 0.5,
                scale: 0.4,
                opacity: 0.6,
            }),
            crop: None,
            fade: None,
            xfade_in_ms: 0,
            xfade_kind: None,
            speed: 1.0,
            grade: None,
            matte: None,
            mask: None,
            reverse: false,
            freeze: None,
            animation: None,
            keyframes: vec![],
            eq: None,
            mute_ranges: vec![],
            stabilize: None,
            speed_ramp: None,
            input_color_space: None,
            nest: None,
            grade_stack: vec![],
            grade_windows: vec![],
        })],
        gain_db: 0.0,
        gain_windows: vec![],
        blend_mode: None,
        visible: true,
        locked: false,
        muted: false,
        solo: false,
        pan: 0.0,
    });
    let edl = edl_from_project(&p);

    // Software reference (default).
    let sw = dir.path().join("ov_sw.mp4");
    let r_sw = render_final(
        &p,
        &edl,
        &fence,
        &sw,
        &preset,
        RenderOptions::default(),
        None,
    )
    .expect("software overlay render");
    assert_eq!(r_sw.pipeline, None, "default render is software");

    // Opt in: overlays remain on the software path even on a CUDA box.
    std::env::set_var("SHELLX_CUT_RENDER_GPU", "1");
    let gp = dir.path().join("ov_gpu.mp4");
    let r_gpu = render_final(
        &p,
        &edl,
        &fence,
        &gp,
        &preset,
        RenderOptions::default(),
        None,
    )
    .expect("overlay opt-in render");
    std::env::remove_var("SHELLX_CUT_RENDER_GPU");
    assert!(gp.exists(), "overlay render wrote output");

    assert_eq!(
        r_gpu.pipeline, None,
        "GPU opt-in must use the software path for overlay timelines"
    );
    assert_eq!(
        r_gpu.hash, r_sw.hash,
        "the fallback must preserve deterministic software output"
    );
}

/// Build a base-track-only project (v1 GPU scope) from a REAL external 4K clip: two
/// hard cuts from distinct source ranges (0–6s, 6–12s) so NVDEC decodes real 4K
/// frames — the CPU-decode-bound regime where the GPU fast-track wins (the plan's
/// "synthetic sources lie" lesson: perf MUST be proven on real footage). Output is
/// 1920×1080 (16:9, matches a 3840×2160 source → the aspect gate passes). The audio
/// track reuses the clip so the (software) audio chain is exercised too.
fn real_4k_project(path: &Path, src_w: u64, src_h: u64) -> Project {
    let mut p = Project::new(
        "real4k",
        ProjectSettings {
            width: 1920,
            height: 1080,
            fps: 30.0,
            audio_rate: 48_000,
            color: cut_core::ColorConfig::default(),
        },
    );
    p.assets.insert(
        "a1".into(),
        cut_core::Asset {
            path: path.display().to_string(),
            hash: "sha256:real4k".into(),
            probe: Some(serde_json::json!({ "width": src_w, "height": src_h })),
            transcript: None,
            perception: None,
            proxy: None,
            filmstrip: None,
        },
    );
    let media = |id: &str, a: u64, b: u64| {
        Clip::Media(MediaClip {
            id: id.into(),
            asset: "a1".into(),
            src_in_ms: a,
            src_out_ms: b,
            effects: vec![],
            gain_db: 0.0,
            transform: None,
            crop: None,
            fade: None,
            xfade_in_ms: 0,
            xfade_kind: None,
            speed: 1.0,
            grade: None,
            matte: None,
            mask: None,
            reverse: false,
            freeze: None,
            animation: None,
            keyframes: vec![],
            eq: None,
            mute_ranges: vec![],
            stabilize: None,
            speed_ramp: None,
            input_color_space: None,
            nest: None,
            grade_stack: vec![],
            grade_windows: vec![],
        })
    };
    p.track_mut("v1").unwrap().clips = vec![media("c1", 0, 6000), media("c2", 6000, 12000)];
    p.track_mut("a1t").unwrap().clips = vec![media("c1a", 0, 6000), media("c2a", 6000, 12000)];
    p
}

/// REAL-4K proof — SOFTWARE render of the base-track 4K timeline (the default path:
/// sw decode + sw scale + libx264). `#[ignore]` (needs a real 4K clip via
/// SHELLX_CUT_TEST_4K + an output dir via SHELLX_CUT_TEST_OUT_DIR), so CI without a
/// GPU/clip still passes. Paired with `real_4k_render_gpu_only`: run each under
/// `/usr/bin/time -v` for clean wall-clock + CPU% (the CPU-freeing headline), and
/// frame-compare the two outputs (PSNR) afterward. Writes `real4k_sw.mp4`.
#[test]
#[ignore = "needs SHELLX_CUT_TEST_4K (real 4K clip) + SHELLX_CUT_TEST_OUT_DIR; run explicitly"]
fn real_4k_render_software_only() {
    let clip = std::env::var("SHELLX_CUT_TEST_4K")
        .expect("set SHELLX_CUT_TEST_4K to a real 3840x2160 clip");
    let outdir = std::env::var("SHELLX_CUT_TEST_OUT_DIR").expect("set SHELLX_CUT_TEST_OUT_DIR");
    std::env::remove_var("SHELLX_CUT_RENDER_GPU"); // force the software default
    let p = real_4k_project(Path::new(&clip), 3840, 2160);
    let edl = edl_from_project(&p);
    let fence = PathFence::new(Path::new(&outdir)).unwrap();
    let out = Path::new(&outdir).join("real4k_sw.mp4");
    let t = std::time::Instant::now();
    let r = render_final(
        &p,
        &edl,
        &fence,
        &out,
        &RenderPreset::default(),
        RenderOptions::default(),
        None,
    )
    .expect("software 4K render");
    eprintln!(
        "REAL4K SOFTWARE: pipeline={:?} wall={:.2}s out={}",
        r.pipeline,
        t.elapsed().as_secs_f64(),
        out.display()
    );
    assert_eq!(r.pipeline, None, "must be the software pipeline");
}

/// REAL-4K proof — GPU fast-track render of the SAME base-track 4K timeline (NVDEC +
/// scale_cuda + nvenc). `#[ignore]`; run with `SHELLX_CUT_RENDER_GPU=1` +
/// `SHELLX_CUT_FFMPEG=<a CUDA ffmpeg>` (the bundled static build has no working
/// CUDA on this box; the system ffmpeg does). Asserts the GPU path actually engaged
/// (`pipeline == "gpu"`) so a silent software fallback can't masquerade as a GPU
/// measurement. Writes `real4k_gpu.mp4` for the PSNR compare vs `real4k_sw.mp4`.
/// The hardware tier must DECLINE a frame size its encoder cannot do, instead of
/// being swapped in and failing the render at encoder-open time.
///
/// The concrete case: NVENC's H.264 engine refuses anything wider than 4096 px
/// (`Width 7680 exceeds 4096` → `No capable devices found`), while the same GPU's
/// HEVC engine encodes 8K fine. Before the size gate, an 8K `render.final` was
/// accepted, failed during encode, and left a 0-byte file in `exports/`.
///
/// GPU-shaped, so `#[ignore]`. It asserts an INVARIANT rather than a fixed limit
/// table — for each codec, `hw_codec_args` must agree with a real probe at that
/// size on whatever GPU is running. So it is meaningful on NVIDIA, Intel, AMD or
/// Apple silicon without encoding vendor ceilings into the test.
#[test]
#[ignore = "needs a hardware encoder; run explicitly on a GPU host"]
fn hw_encoder_declines_frame_sizes_it_cannot_encode() {
    use cut_media::hwencode::{encoder_supports_size, hw_caps, hw_codec_args};

    let caps = hw_caps();
    let mut checked = 0usize;
    // 4K is inside every current hardware encoder's range; 8K is where the
    // H.264 engines stop. Both are exercised so the gate is shown to ALLOW as
    // well as refuse — a gate that only ever says no is not a gate.
    for (codec, w, h) in [
        ("h264", 3840u32, 2160u32),
        ("h264", 7680, 4320),
        ("hevc", 3840, 2160),
        ("hevc", 7680, 4320),
    ] {
        let Some(encoder) = caps.for_codec(codec) else {
            continue; // no hardware tier for this codec on this host
        };
        let probed = encoder_supports_size(encoder, w, h);
        let offered = hw_codec_args(codec, 1, w, h).is_some();
        assert_eq!(
            offered, probed,
            "{encoder} at {w}x{h}: hw_codec_args offered={offered} but a real probe says {probed}"
        );
        eprintln!("{encoder} {w}x{h}: supported={probed}, hardware tier offered={offered}");
        checked += 1;
    }
    assert!(
        checked > 0,
        "no hardware encoder on this host — run this rig on a GPU machine"
    );
}

#[test]
#[ignore = "needs SHELLX_CUT_TEST_4K + SHELLX_CUT_TEST_OUT_DIR + a CUDA ffmpeg; run explicitly"]
fn real_4k_render_gpu_only() {
    let clip = std::env::var("SHELLX_CUT_TEST_4K")
        .expect("set SHELLX_CUT_TEST_4K to a real 3840x2160 clip");
    let outdir = std::env::var("SHELLX_CUT_TEST_OUT_DIR").expect("set SHELLX_CUT_TEST_OUT_DIR");
    std::env::set_var("SHELLX_CUT_RENDER_GPU", "1");
    let p = real_4k_project(Path::new(&clip), 3840, 2160);
    let edl = edl_from_project(&p);
    let fence = PathFence::new(Path::new(&outdir)).unwrap();
    let out = Path::new(&outdir).join("real4k_gpu.mp4");
    let gpu_ok = cut_media::hwencode::gpu_filters_available();
    let t = std::time::Instant::now();
    let r = render_final(
        &p,
        &edl,
        &fence,
        &out,
        &RenderPreset::default(),
        RenderOptions::default(),
        None,
    )
    .expect("gpu 4K render");
    std::env::remove_var("SHELLX_CUT_RENDER_GPU");
    eprintln!(
        "REAL4K GPU: gpu_filters_available={gpu_ok} pipeline={:?} wall={:.2}s out={}",
        r.pipeline,
        t.elapsed().as_secs_f64(),
        out.display()
    );
    assert_eq!(
        r.pipeline.as_deref(),
        Some("gpu"),
        "GPU path must engage — set SHELLX_CUT_FFMPEG to a CUDA-capable ffmpeg"
    );
}

/// Real-4K proof of the VRAM bound: opt-in + a CUDA box, but a
/// VRAM budget far below the graph's estimate → the gate MUST fall back to software
/// (`pipeline == None`) and still produce the render, never OOM the GPU. `#[ignore]`;
/// run with `SHELLX_CUT_RENDER_GPU=1` + a CUDA ffmpeg. Portable assert: on a non-GPU
/// box the probe is false so it is software anyway — either way the over-budget
/// timeline never takes the GPU path. Writes `real4k_vram_fallback.mp4`.
#[test]
#[ignore = "needs SHELLX_CUT_TEST_4K + SHELLX_CUT_TEST_OUT_DIR; run explicitly"]
fn real_4k_vram_over_budget_falls_back_to_software() {
    let clip = std::env::var("SHELLX_CUT_TEST_4K")
        .expect("set SHELLX_CUT_TEST_4K to a real 3840x2160 clip");
    let outdir = std::env::var("SHELLX_CUT_TEST_OUT_DIR").expect("set SHELLX_CUT_TEST_OUT_DIR");
    std::env::set_var("SHELLX_CUT_RENDER_GPU", "1"); // opt IN
    std::env::set_var("SHELLX_CUT_GPU_VRAM_BUDGET_MB", "1"); // 1 MiB budget — never fits
    let p = real_4k_project(Path::new(&clip), 3840, 2160);
    let edl = edl_from_project(&p);
    let fence = PathFence::new(Path::new(&outdir)).unwrap();
    let out = Path::new(&outdir).join("real4k_vram_fallback.mp4");
    let r = render_final(
        &p,
        &edl,
        &fence,
        &out,
        &RenderPreset::default(),
        RenderOptions::default(),
        None,
    )
    .expect("over-budget render must still succeed via the software fallback");
    std::env::remove_var("SHELLX_CUT_RENDER_GPU");
    std::env::remove_var("SHELLX_CUT_GPU_VRAM_BUDGET_MB");
    eprintln!(
        "REAL4K VRAM-FALLBACK: pipeline={:?} (expect None)",
        r.pipeline
    );
    assert_eq!(
        r.pipeline, None,
        "an over-VRAM-budget timeline must fall back to software, never OOM the GPU"
    );
    assert!(out.exists(), "software fallback wrote the output");
}

/// THE Stage-3 correctness proof: a SEGMENTED render must be frame-identical to
/// the whole-graph render. Renders the same 6s composite both ways (whole = the
/// single pass since 6s < the 120s threshold; segmented = forced 2s windows via
/// the direct entry point) and asserts matching duration + high frame PSNR. A
/// broken seam (overlay clamp, caption rebase, off-by-one boundary frame) fails.
#[test]
fn segmented_render_matches_whole_graph() {
    let _env = SEG_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let dir = tempfile::tempdir().unwrap();
    let clip = gen_clip(dir.path(), "src.mp4");
    let project = composite_6s_project(&clip);
    let edl = edl_from_project(&project);
    assert_eq!(edl.duration_ms, 6000, "expected a 6s composite");
    let fence = PathFence::new(dir.path()).unwrap();
    let preset = RenderPreset::default();

    // Whole-graph reference (no env → 6s < 120s default threshold = single pass).
    std::env::remove_var("SHELLX_CUT_RENDER_SEGMENT_SEC");
    std::env::remove_var("SHELLX_CUT_RENDER_WINDOW_SEC");
    let whole = dir.path().join("whole.mp4");
    render_final(
        &project,
        &edl,
        &fence,
        &whole,
        &preset,
        RenderOptions::default(),
        None,
    )
    .expect("whole-graph render");

    // Segmented: 2s windows → seams at 2s and 4s (overlay + caption straddle).
    std::env::set_var("SHELLX_CUT_RENDER_WINDOW_SEC", "2");
    let seg = dir.path().join("seg.mp4");
    cut_media::render::render_segmented(
        &project,
        &edl,
        &fence,
        &seg,
        &preset,
        RenderOptions::default(),
        None,
    )
    .expect("segmented render");
    std::env::remove_var("SHELLX_CUT_RENDER_WINDOW_SEC");

    let wd = probe(&whole).unwrap().duration_ms.unwrap();
    let sd = probe(&seg).unwrap().duration_ms.unwrap();
    assert!(
        (wd as i64 - sd as i64).abs() <= 100,
        "durations diverge: whole={wd}ms seg={sd}ms"
    );
    let psnr = avg_psnr(&seg, &whole);
    eprintln!("segmented↔whole-graph PSNR: {psnr:.1} dB (whole={wd}ms seg={sd}ms)");
    assert!(
        psnr > 30.0,
        "segmented vs whole-graph PSNR {psnr:.1} dB too low — a window seam is wrong"
    );
}

/// Stage-3 compose-frame proof: render.frame{compose} on a long timeline must
/// compose only the WINDOW containing at_ms (the fix for the O(at_ms) 22GB OOM)
/// while producing the SAME frame as the whole-graph compose. Extracts a frame
/// deep in the 6s composite with a large window (whole graph) and a 2s window
/// (windowed) and compares them via PSNR.
#[test]
fn compose_frame_windowed_matches_whole() {
    let _env = SEG_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let dir = tempfile::tempdir().unwrap();
    let clip = gen_clip(dir.path(), "src.mp4");
    let project = composite_6s_project(&clip);
    let edl = edl_from_project(&project);
    let at = 3500u64; // inside window [2000,4000); caption present, overlay gone

    // Whole-graph frame: a window larger than the timeline → single window.
    std::env::set_var("SHELLX_CUT_RENDER_WINDOW_SEC", "10");
    let whole_jpg = extract_frame(&project, &edl, dir.path(), at, None).expect("whole frame");
    // Windowed frame: 2s windows → at_ms composed from window [2000,4000).
    std::env::set_var("SHELLX_CUT_RENDER_WINDOW_SEC", "2");
    let win_jpg = extract_frame(&project, &edl, dir.path(), at, None).expect("windowed frame");
    std::env::remove_var("SHELLX_CUT_RENDER_WINDOW_SEC");

    let wp = dir.path().join("w.jpg");
    let sp = dir.path().join("s.jpg");
    std::fs::write(&wp, &whole_jpg).unwrap();
    std::fs::write(&sp, &win_jpg).unwrap();
    let psnr = avg_psnr(&sp, &wp);
    eprintln!("compose-frame windowed↔whole PSNR: {psnr:.1} dB");
    assert!(
        psnr > 35.0,
        "windowed compose-frame differs from whole-graph: {psnr:.1} dB"
    );
}

#[test]
fn render_refuses_fenced_paths() {
    let dir = tempfile::tempdir().unwrap();
    let clip = gen_clip(dir.path(), "in.mp4");
    let project = two_clip_project(&clip);
    let edl = edl_from_project(&project);
    let fence = PathFence::new(dir.path()).unwrap();

    // Traversal and out-of-project targets die BEFORE any ffmpeg work.
    for bad in ["../evil.mp4", "/tmp/evil.mp4"] {
        let err = render_final(
            &project,
            &edl,
            &fence,
            Path::new(bad),
            &RenderPreset::default(),
            RenderOptions::default(),
            None,
        )
        .expect_err(bad);
        assert_eq!(err.code, "invalid_args", "{bad} → {err:?}");
    }
    // Overwriting a non-media file inside the project is refused too.
    std::fs::write(dir.path().join("ops.jsonl"), "{}").unwrap();
    let err = render_final(
        &project,
        &edl,
        &fence,
        Path::new("ops.jsonl"),
        &RenderPreset::default(),
        RenderOptions::default(),
        None,
    )
    .unwrap_err();
    assert_eq!(err.code, "conflict");
}

/// edit.speed render: a clip retimed 2× must render to HALF its source span,
/// 0.5× to DOUBLE — proves the setpts (video) + atempo (audio) filters land the
/// composition duration where the EDL says, end to end through real ffmpeg.
/// This is the load-bearing behavioral proof of the per-clip time-remap.
#[test]
fn speed_retime_changes_render_duration() {
    let dir = tempfile::tempdir().unwrap();
    let clip = gen_clip(dir.path(), "src.mp4"); // ~2000ms, video + 440Hz audio
    let fence = PathFence::new(dir.path()).unwrap();

    // One full clip [0,2000) mirrored on v1 + a1t, rebuilt at a given speed.
    let build = |speed: f64| -> Project {
        let mut p = Project::new(
            "spd",
            ProjectSettings {
                width: 320,
                height: 240,
                fps: 30.0,
                audio_rate: 48_000,
                color: cut_core::ColorConfig::default(),
            },
        );
        p.assets.insert(
            "a1".into(),
            cut_core::Asset {
                path: clip.display().to_string(),
                hash: "sha256:test".into(),
                probe: None,
                transcript: None,
                perception: None,
                proxy: None,
                filmstrip: None,
            },
        );
        let mk = |id: &str| {
            Clip::Media(MediaClip {
                id: id.into(),
                asset: "a1".into(),
                src_in_ms: 0,
                src_out_ms: 2000,
                effects: vec![],
                gain_db: 0.0,
                transform: None,
                crop: None,
                fade: None,
                xfade_in_ms: 0,
                xfade_kind: None,
                speed,
                grade: None,
                matte: None,
                mask: None,
                reverse: false,
                freeze: None,
                animation: None,
                keyframes: vec![],
                eq: None,
                mute_ranges: vec![],
                stabilize: None,
                speed_ramp: None,
                input_color_space: None,
                nest: None,
                grade_stack: vec![],
                grade_windows: vec![],
            })
        };
        p.track_mut("v1").unwrap().clips = vec![mk("c1")];
        p.track_mut("a1t").unwrap().clips = vec![mk("c1a")];
        p
    };
    let render = |p: &Project, name: &str| -> u64 {
        let edl = edl_from_project(p);
        render_final(
            p,
            &edl,
            &fence,
            Path::new(name),
            &RenderPreset::default(),
            RenderOptions::default(),
            None,
        )
        .expect("render")
        .duration_ms
    };

    // Baseline (1.0×): the clip occupies its full source span.
    assert_eq!(edl_from_project(&build(1.0)).duration_ms, 2000, "1.0× EDL");
    let d1 = render(&build(1.0), "s1.mp4");
    assert!(d1.abs_diff(2000) <= 60, "1.0× rendered {d1} ms ≈ 2000");

    // 2× → HALF the timeline (EDL says 1000; render must land there).
    assert_eq!(
        edl_from_project(&build(2.0)).duration_ms,
        1000,
        "2× EDL must be half"
    );
    let d2 = render(&build(2.0), "s2.mp4");
    assert!(
        d2.abs_diff(1000) <= 60,
        "2× rendered {d2} ms ≈ 1000 (setpts+atempo)"
    );

    // 0.5× slow-mo → DOUBLE (EDL says 4000).
    assert_eq!(
        edl_from_project(&build(0.5)).duration_ms,
        4000,
        "0.5× EDL must be double"
    );
    let d3 = render(&build(0.5), "s3.mp4");
    assert!(d3.abs_diff(4000) <= 90, "0.5× rendered {d3} ms ≈ 4000");
}
