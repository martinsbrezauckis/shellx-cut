use super::*;
use cut_core::{edit::make_media_clip, Asset, Clip, Project, ProjectSettings, Track, TrackKind};
use std::process::Command;

fn ffmpeg(args: &[&str]) {
    let output = Command::new(crate::ffmpeg::ffmpeg_bin())
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "ffmpeg failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn samples(path: &Path) -> (u32, u64) {
    let output = Command::new(crate::ffmpeg::ffprobe_bin())
        .args([
            "-v",
            "error",
            "-select_streams",
            "a:0",
            "-show_entries",
            "stream=sample_rate,duration_ts",
            "-of",
            "json",
        ])
        .arg(path)
        .output()
        .unwrap();
    assert!(output.status.success());
    let parsed: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let stream = &parsed["streams"][0];
    (
        stream["sample_rate"].as_str().unwrap().parse().unwrap(),
        stream["duration_ts"].as_i64().unwrap().try_into().unwrap(),
    )
}

fn graph_pcm_samples(
    project: &Project,
    edl: &Edl,
    dir: &Path,
    name: &str,
    target: Option<i32>,
) -> (u32, u64) {
    let graph = build_graph(
        project,
        edl,
        dir,
        false,
        true,
        false,
        RenderOptions {
            loudness_target: target,
            ..RenderOptions::default()
        },
        None,
    )
    .unwrap();
    let output = dir.join(format!("{name}.wav"));
    let mut args = vec!["-v".to_string(), "error".to_string()];
    for input in &graph.inputs {
        args.extend(["-i".into(), input.path.display().to_string()]);
    }
    args.extend([
        "-filter_complex".into(),
        graph.filter.trim_end_matches(['\n', ';']).into(),
        "-map".into(),
        format!("[{}]", graph.audio_out.as_deref().unwrap()),
        "-c:a".into(),
        "pcm_s16le".into(),
        "-y".into(),
        output.display().to_string(),
    ]);
    let run = Command::new(crate::ffmpeg::ffmpeg_bin())
        .args(&args)
        .output()
        .unwrap();
    assert!(
        run.status.success(),
        "{name}: {}",
        String::from_utf8_lossy(&run.stderr)
    );
    samples(&output)
}

#[test]
fn final_render_paths_keep_encoded_audio_within_the_video_edl() {
    let dir = tempfile::tempdir().unwrap();
    #[cfg(unix)]
    std::fs::set_permissions(
        dir.path(),
        std::os::unix::fs::PermissionsExt::from_mode(0o700),
    )
    .unwrap();
    let screen = dir.path().join("screen.mp4");
    let aac = dir.path().join("polished-96k.m4a");
    let wav = dir.path().join("system-48k.wav");
    ffmpeg(&[
        "-v",
        "error",
        "-f",
        "lavfi",
        "-i",
        "color=c=blue:size=128x72:rate=24:duration=9.2",
        "-c:v",
        "libx264",
        "-pix_fmt",
        "yuv420p",
        "-y",
        screen.to_str().unwrap(),
    ]);
    ffmpeg(&[
        "-v",
        "error",
        "-f",
        "lavfi",
        "-i",
        "sine=frequency=440:sample_rate=96000:duration=9.2",
        "-c:a",
        "aac",
        "-y",
        aac.to_str().unwrap(),
    ]);
    ffmpeg(&[
        "-v",
        "error",
        "-f",
        "lavfi",
        "-i",
        "sine=frequency=660:sample_rate=48000:duration=9.2",
        "-c:a",
        "pcm_s16le",
        "-y",
        wav.to_str().unwrap(),
    ]);
    let mut project = Project::new(
        "render-extent",
        ProjectSettings {
            width: 128,
            height: 72,
            fps: 24.0,
            audio_rate: 48_000,
            color: cut_core::ColorConfig::default(),
        },
    );
    for (id, path, probe) in [
        (
            "screen",
            &screen,
            serde_json::json!({
                "kind":"video", "width":128, "height":72, "fps":24,
                "duration_ms":9200, "has_audio":false
            }),
        ),
        (
            "aac",
            &aac,
            serde_json::json!({
                "kind":"audio", "duration_ms":9200, "has_audio":true,
                "audio_rate":96000
            }),
        ),
        (
            "wav",
            &wav,
            serde_json::json!({
                "kind":"audio", "duration_ms":9200, "has_audio":true,
                "audio_rate":48000
            }),
        ),
    ] {
        project.assets.insert(
            id.into(),
            Asset {
                path: path.to_string_lossy().into_owned(),
                hash: format!("sha256:{id}"),
                probe: Some(probe),
                transcript: None,
                perception: None,
                proxy: None,
                filmstrip: None,
            },
        );
    }
    project
        .track_mut("v1")
        .unwrap()
        .clips
        .push(Clip::Media(make_media_clip(
            "screen-clip",
            "screen",
            0,
            8917,
        )));
    project
        .track_mut("a1t")
        .unwrap()
        .clips
        .push(Clip::Media(make_media_clip("aac-clip", "aac", 0, 8917)));
    project.tracks.push(Track {
        id: "system".into(),
        kind: TrackKind::Audio,
        clips: vec![Clip::Media(make_media_clip("wav-clip", "wav", 0, 8912))],
        gain_db: 0.0,
        gain_windows: vec![],
        blend_mode: None,
        visible: true,
        locked: false,
        muted: false,
        solo: false,
        pan: 0.0,
    });
    let edl = cut_core::edl_from_project(&project);
    assert_eq!(edl.duration_ms, 8917);
    assert!(!should_segment(&project, &edl, 128, 72));
    for (name, target) in [("plain", None), ("normalized", Some(-16))] {
        assert_eq!(
            graph_pcm_samples(&project, &edl, dir.path(), name, target),
            (48_000, 428_016),
            "{name}"
        );
    }
    // An intentionally longer audio clip extends the EDL; its extra samples
    // remain present rather than being cut at the shorter video clip endpoint.
    let mut long_audio = project.clone();
    let Clip::Media(clip) = &mut long_audio.track_mut("a1t").unwrap().clips[0] else {
        panic!("expected audio media clip");
    };
    clip.src_out_ms = 9200;
    let long_edl = cut_core::edl_from_project(&long_audio);
    assert_eq!(long_edl.duration_ms, 9200);
    assert_eq!(
        graph_pcm_samples(&long_audio, &long_edl, dir.path(), "long-audio", None),
        (48_000, 441_600)
    );

    let fence = PathFence::new(dir.path()).unwrap();
    let preset = RenderPreset::named("draft").unwrap();
    for (name, target) in [("plain", None), ("normalized", Some(-16))] {
        let opts = RenderOptions {
            loudness_target: target,
            ..RenderOptions::default()
        };
        for segmented in [false, true] {
            let kind = if segmented { "segmented" } else { "single" };
            let out = dir.path().join(format!("{name}-{kind}.mp4"));
            let result = if segmented {
                render_segmented(&project, &edl, &fence, &out, &preset, opts, None)
            } else {
                render_final(&project, &edl, &fence, &out, &preset, opts, None)
            }
            .unwrap();
            assert!(
                result.duration_ms.abs_diff(edl.duration_ms) <= 42,
                "{name}/{kind}: encoded duration {} ms exceeds EDL {} ms",
                result.duration_ms,
                edl.duration_ms
            );
        }
    }

    let mut video_only = project.clone();
    video_only.track_mut("a1t").unwrap().clips.clear();
    video_only.track_mut("system").unwrap().clips.clear();
    let video_edl = cut_core::edl_from_project(&video_only);
    let video_graph = build_graph(
        &video_only,
        &video_edl,
        dir.path(),
        false,
        true,
        false,
        RenderOptions::default(),
        None,
    )
    .unwrap();
    assert!(video_graph.audio_out.is_none());
    let video_out = dir.path().join("video-only.mp4");
    let video_result = render_final(
        &video_only,
        &video_edl,
        &fence,
        &video_out,
        &preset,
        RenderOptions::default(),
        None,
    )
    .unwrap();
    assert!(video_result.duration_ms.abs_diff(video_edl.duration_ms) <= 42);

    let empty = Project::new("empty", project.settings.clone());
    let empty_edl = cut_core::edl_from_project(&empty);
    assert_eq!(empty_edl.duration_ms, 0);
    assert!(render_final(
        &empty,
        &empty_edl,
        &fence,
        &dir.path().join("empty.mp4"),
        &preset,
        RenderOptions::default(),
        None,
    )
    .is_err());
}
