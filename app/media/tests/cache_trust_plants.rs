//! Item 155: derived files planted in a received project are not used by a
//! final render. Each case plants a VALID but different file under the exact
//! cache name and renders; the plant must not reach the output.
//! (Mask and power-window alphas: `render/cache_trust_tests.rs`; nest bakes:
//! server `dispatch/tests/nest_cache_links.rs`; matte bakes: server
//! `matte_cache_bake_tests.rs`.)

cut_core::isolate_test_process!();

use cut_core::{
    edl_from_project, Clip, ClipMatte, ClipStabilize, MediaClip, Project, ProjectSettings,
};
use cut_media::{render_final, PathFence, RenderOptions, RenderPreset};
use std::path::{Path, PathBuf};

fn private_dir() -> ::tempfile::TempDir {
    let dir = ::tempfile::tempdir().unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    dir
}

fn ffmpeg(args: &[&str], out: &Path) {
    let args: Vec<String> = args
        .iter()
        .map(|s| s.to_string())
        .chain([out.display().to_string()])
        .collect();
    cut_media::ffmpeg::run_ffmpeg(&args).expect("ffmpeg");
}

/// A 2 s clip; `motion` picks a moving crop so vidstab finds real motion.
fn clip(dir: &Path, name: &str, motion: &str) -> PathBuf {
    let out = dir.join(name);
    let vf = format!(
        "crop=320:240:x='20+{motion}*sin(2*PI*t*5)':y='20+{motion}*cos(2*PI*t*6)',setsar=1"
    );
    ffmpeg(
        &[
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=360x280:rate=30:duration=2",
            "-vf",
            &vf,
            "-c:v",
            "libx264",
            "-preset",
            "ultrafast",
            "-pix_fmt",
            "yuv420p",
            "-threads",
            "1",
        ],
        &out,
    );
    out
}

fn project_with(
    path: &Path,
    hash: &str,
    stabilize: Option<ClipStabilize>,
    matte: Option<ClipMatte>,
) -> Project {
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
            path: path.display().to_string(),
            hash: hash.into(),
            probe: Some(serde_json::to_value(cut_media::probe(path).unwrap()).unwrap()),
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
        matte,
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
}

/// The .trf the renderer would look for (stabilization.rs `trf_path`).
fn trf_path(project_dir: &Path, hash: &str) -> PathBuf {
    let safe: String = hash
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    project_dir.join("stab").join(format!("{safe}_0_2000.trf"))
}

#[test]
fn planted_stabilization_data_is_detected_again() {
    let project_dir = private_dir();
    let dir = project_dir.path();
    let shaky = clip(dir, "shaky.mp4", "12");
    let hash = "sha256:shakyplant";
    // A valid .trf of a DIFFERENT clip (other motion), planted under the name
    // this clip's render will look for.
    let other = clip(dir, "other.mp4", "3");
    let planted_dir = private_dir();
    let planted = planted_dir.path().join("planted.trf");
    ffmpeg(
        &[
            "-i",
            &other.display().to_string(),
            "-vf",
            // The product's own filter-path escaping: a raw Windows path (drive
            // colon, backslashes) breaks the filter parser.
            &format!(
                "vidstabdetect=result={}",
                cut_media::ffmpeg::escape_filter_path(&planted)
            ),
            "-f",
            "null",
        ],
        Path::new("-"),
    );
    let trf = trf_path(dir, hash);
    std::fs::create_dir_all(trf.parent().unwrap()).unwrap();
    std::fs::copy(&planted, &trf).unwrap();
    let planted_bytes = std::fs::read(&trf).unwrap();

    let project = project_with(&shaky, hash, Some(ClipStabilize { smoothing: 15.0 }), None);
    let edl = edl_from_project(&project);
    let fence = PathFence::new(dir).unwrap();
    render_final(
        &project,
        &edl,
        &fence,
        &dir.join("out.mp4"),
        &RenderPreset::default(),
        RenderOptions::default(),
        None,
    )
    .expect("render");
    assert_ne!(
        std::fs::read(&trf).unwrap(),
        planted_bytes,
        "the planted .trf was used"
    );
    assert!(
        cut_media::cache_trust::trusted(&trf),
        "the new detection is recorded"
    );
}

#[test]
fn planted_matte_alpha_is_refused_not_used() {
    let project_dir = private_dir();
    let dir = project_dir.path();
    let src = clip(dir, "src.mp4", "0");
    let hash = "sha256:matteplant";
    let matte = ClipMatte {
        mode: cut_core::MatteMode::Remove,
        model: cut_core::MatteModel::Rvm,
        bg: None,
        quality: cut_core::MatteQuality::Good,
        seed: None,
    };
    // A valid alpha (all white) planted under the exact cache name.
    let alpha_dir = dir.join("cache").join("matte");
    std::fs::create_dir_all(&alpha_dir).unwrap();
    let alpha = alpha_dir.join(matte.cache_filename(hash));
    ffmpeg(
        &[
            "-f",
            "lavfi",
            "-i",
            "color=c=white:s=320x240:r=30:d=2",
            "-vf",
            "format=gray",
            "-c:v",
            "ffv1",
            "-pix_fmt",
            "gray",
        ],
        &alpha,
    );
    let project = project_with(&src, hash, None, Some(matte));
    let edl = edl_from_project(&project);
    let fence = PathFence::new(dir).unwrap();
    let err = render_final(
        &project,
        &edl,
        &fence,
        &dir.join("out.mp4"),
        &RenderPreset::default(),
        RenderOptions::default(),
        None,
    )
    .expect_err("a matte this engine did not bake must not be rendered");
    assert!(
        err.message.contains("not made on this computer"),
        "{}",
        err.message
    );
    assert!(err.suggested_action.unwrap().contains("render.final"));
    assert!(
        !dir.join("out.mp4").exists() || std::fs::metadata(dir.join("out.mp4")).unwrap().len() == 0
    );
}
