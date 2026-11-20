//! Item 134: the mono check and the render must use the SAME audio stream.
//!
//! The render binds a filtergraph label `[N:a]`, for which ffmpeg takes the
//! FIRST audio stream of input N (stream order), not the stream marked
//! default (which is what ffmpeg picks when nothing is mapped). The mono
//! check (`render::options::mono_to_both_channels`) reads the first audio
//! stream of the probe, so the two agree. This test pins it with a real file
//! whose first audio stream is mono and not default and whose second is
//! stereo and default: the render must play the first (mono) stream on both
//! channels at its own level.

use cut_core::{edl_from_project, Clip, MediaClip, Project, ProjectSettings};
use cut_media::PathFence;
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

/// Video, then a:0 = mono 440 Hz at -12 dBFS (not default), a:1 = stereo
/// 880 Hz (default).
fn two_audio_streams(dir: &Path) -> PathBuf {
    let out = dir.join("two_audio.mp4");
    let args: Vec<String> = [
        "-f",
        "lavfi",
        "-i",
        "testsrc2=size=160x120:rate=25:duration=2",
        "-f",
        "lavfi",
        "-i",
        "sine=frequency=440:duration=2,volume=0.25",
        "-f",
        "lavfi",
        "-i",
        "sine=frequency=880:duration=2",
        "-filter_complex",
        "[2:a]pan=stereo|c0=c0|c1=c0[st]",
        "-map",
        "0:v",
        "-map",
        "1:a",
        "-map",
        "[st]",
        "-c:v",
        "libx264",
        "-preset",
        "ultrafast",
        "-pix_fmt",
        "yuv420p",
        "-c:a",
        "aac",
        "-disposition:a:0",
        "0",
        "-disposition:a:1",
        "default",
    ]
    .iter()
    .map(|s| s.to_string())
    .chain([out.display().to_string()])
    .collect();
    cut_media::ffmpeg::run_ffmpeg(&args).expect("test asset generation");
    out
}

fn media_clip(id: &str, asset: &str) -> Clip {
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
}

/// (channels, RMS dB per channel) of `path` via ffmpeg astats.
fn channel_rms(path: &Path) -> Vec<f64> {
    let out = std::process::Command::new(cut_media::ffmpeg::ffmpeg_bin())
        .args(["-hide_banner", "-nostats", "-i"])
        .arg(path)
        .args([
            "-af",
            "astats=measure_overall=none:measure_perchannel=RMS_level",
            "-f",
            "null",
            "-",
        ])
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stderr)
        .lines()
        .filter_map(|l| l.split("RMS level dB: ").nth(1))
        .filter_map(|v| v.trim().parse::<f64>().ok())
        .collect()
}

#[test]
fn render_and_mono_check_use_the_first_audio_stream() {
    let dir = private_dir();
    let src = two_audio_streams(dir.path());
    let probe = cut_media::probe::probe(&src).expect("probe");
    let mut p = Project::new(
        "t",
        ProjectSettings {
            width: 160,
            height: 120,
            fps: 25.0,
            audio_rate: 48_000,
            color: cut_core::ColorConfig::default(),
        },
    );
    p.assets.insert(
        "a1".into(),
        cut_core::Asset {
            path: src.display().to_string(),
            hash: "sha256:test".into(),
            probe: Some(serde_json::to_value(&probe).unwrap()),
            transcript: None,
            perception: None,
            proxy: None,
            filmstrip: None,
        },
    );
    p.track_mut("v1").unwrap().clips = vec![media_clip("c1", "a1")];
    p.track_mut("a1t").unwrap().clips = vec![media_clip("c1a", "a1")];
    let edl = edl_from_project(&p);
    let fence = PathFence::new(dir.path()).unwrap();
    let out = dir.path().join("mix.wav");
    cut_media::render::render_audio(
        &p,
        &edl,
        &fence,
        &out,
        &["-c:a".to_string(), "pcm_s16le".to_string()],
        None,
    )
    .expect("audio render");

    // The mono 440 Hz stream at 0.25 amplitude is about -15 dB RMS. Played at
    // unity on both channels each channel keeps that level; had the mono
    // check looked at the default (stereo) stream, the pan would be skipped
    // and the mono stream would come out about 3 dB lower per channel; had the
    // render bound the default stream, the level would be the stereo 880 Hz
    // one (about -3 dB).
    let mono_ref = dir.path().join("mono_ref.wav");
    cut_media::ffmpeg::run_ffmpeg(&[
        "-i".into(),
        src.display().to_string(),
        "-map".into(),
        "0:a:0".into(),
        mono_ref.display().to_string(),
    ])
    .unwrap();
    let mono_db = channel_rms(&mono_ref)[0];
    let rendered = channel_rms(&out);
    assert_eq!(rendered.len(), 2, "{rendered:?}");
    for ch in rendered {
        assert!(
            (ch - mono_db).abs() < 0.5,
            "channel {ch} dB vs mono stream {mono_db} dB"
        );
    }
}

cut_core::isolate_test_process!();
