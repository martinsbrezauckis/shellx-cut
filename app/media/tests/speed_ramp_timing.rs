//! Real ffmpeg proof for frame-grid speed-ramp duration semantics.

use cut_core::{
    edl_from_project, timeline_frame_count, Actor, Clip, ColorConfig, MediaClip, Project,
    ProjectSettings, ProjectStore, SpeedRamp, SpeedRampPoint,
};
use cut_media::{render_final, PathFence, RenderOptions, RenderPreset};
use serde_json::json;
use std::path::{Path, PathBuf};
use std::process::Command;

fn generate_source(dir: &Path, fps: f64) -> PathBuf {
    let source = dir.join("source.mp4");
    let status = Command::new("ffmpeg")
        .args([
            "-y",
            "-hide_banner",
            "-loglevel",
            "error",
            "-f",
            "lavfi",
            "-i",
            &format!("testsrc2=size=320x240:rate={fps}:duration=4"),
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
            source.to_str().unwrap(),
        ])
        .status()
        .expect("ffmpeg installed");
    assert!(status.success());
    source
}

fn clip(id: &str, ramp: SpeedRamp) -> Clip {
    Clip::Media(MediaClip {
        id: id.into(),
        asset: "a1".into(),
        src_in_ms: 0,
        src_out_ms: 4000,
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
        speed_ramp: Some(ramp),
        input_color_space: None,
        nest: None,
        grade_stack: vec![],
        grade_windows: vec![],
    })
}

fn ramp(fps: f64) -> SpeedRamp {
    SpeedRamp {
        points: vec![
            SpeedRampPoint {
                at_ms: 0,
                factor: 0.5,
            },
            SpeedRampPoint {
                at_ms: 2000,
                factor: 2.0,
            },
            SpeedRampPoint {
                at_ms: 4000,
                factor: 0.5,
            },
        ],
        segments: 24,
        preferred_segments: Some(24),
        timebase_fps: Some(fps),
        timebase_audio_rate: Some(48_000),
    }
}

fn counted_video_frames(path: &Path) -> u64 {
    let output = Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-select_streams",
            "v:0",
            "-count_frames",
            "-show_entries",
            "stream=nb_read_frames",
            "-of",
            "default=noprint_wrappers=1:nokey=1",
            path.to_str().unwrap(),
        ])
        .output()
        .expect("ffprobe installed");
    assert!(output.status.success());
    String::from_utf8(output.stdout)
        .unwrap()
        .trim()
        .parse()
        .expect("frame count")
}

#[test]
fn ramp_render_uses_the_same_frame_and_sample_timebase_as_its_edl() {
    for fps in [30.0, 30_000.0 / 1001.0] {
        let dir = tempfile::tempdir().unwrap();
        let source = generate_source(dir.path(), fps);
        let mut project = Project::new(
            "ramp",
            ProjectSettings {
                width: 320,
                height: 240,
                fps,
                audio_rate: 48_000,
                color: ColorConfig::default(),
            },
        );
        project.assets.insert(
            "a1".into(),
            cut_core::Asset {
                path: source.display().to_string(),
                hash: "sha256:test".into(),
                probe: None,
                transcript: None,
                perception: None,
                proxy: None,
                filmstrip: None,
            },
        );
        project.track_mut("v1").unwrap().clips = vec![clip("v", ramp(fps))];
        project.track_mut("a1t").unwrap().clips = vec![clip("a", ramp(fps))];
        let edl = edl_from_project(&project);
        let expected_frames = timeline_frame_count(edl.duration_ms, fps);
        let fence = PathFence::new(dir.path()).unwrap();
        let output = render_final(
            &project,
            &edl,
            &fence,
            Path::new("ramp.mp4"),
            &RenderPreset::default(),
            RenderOptions::default(),
            None,
        )
        .expect("ramp render");
        let tolerance_ms = (1000.0 / fps).ceil() as u64;
        assert!(
            output.duration_ms.abs_diff(edl.duration_ms) <= tolerance_ms,
            "fps={fps}: output={} edl={}",
            output.duration_ms,
            edl.duration_ms
        );
        assert_eq!(
            counted_video_frames(&output.path),
            expected_frames,
            "fps={fps}"
        );
    }
}

#[test]
fn format_regrid_reopens_and_renders_on_the_new_frame_grid() {
    let dir = tempfile::tempdir().unwrap();
    let source = generate_source(dir.path(), 24.0);
    let mut store = ProjectStore::create(
        dir.path(),
        "regrid",
        Some(ProjectSettings {
            width: 320,
            height: 240,
            fps: 24.0,
            audio_rate: 48_000,
            color: ColorConfig::default(),
        }),
    )
    .unwrap();
    store
        .record_import(
            Some("a1".into()),
            cut_core::Asset {
                path: source.display().to_string(),
                hash: "sha256:test".into(),
                probe: None,
                transcript: None,
                perception: None,
                proxy: None,
                filmstrip: None,
            },
            Actor::system(),
            None,
        )
        .unwrap();
    for track in ["v1", "a1t"] {
        store
            .apply(
                "edit.insert",
                json!({"asset":"a1","track":track,"at_ms":0,"src_range_ms":[0,4000]}),
                Actor::system(),
                None,
            )
            .unwrap();
    }
    for clip in ["c1", "c2"] {
        store
            .apply(
                "edit.speed_ramp",
                json!({
                    "clip": clip,
                    "points": [
                        {"at_ms": 0, "factor": 0.5},
                        {"at_ms": 2000, "factor": 2.0},
                        {"at_ms": 4000, "factor": 0.5}
                    ],
                    "segments": 24,
                    "timebase_fps": 24.0,
                    "timebase_audio_rate": 48_000
                }),
                Actor::system(),
                None,
            )
            .unwrap();
    }
    store
        .set_format(None, None, Some(60.0), Actor::system(), None)
        .unwrap();
    assert!(store
        .project
        .all_sequence_tracks()
        .flat_map(|track| &track.clips)
        .all(|clip| {
            matches!(clip, Clip::Media(media) if media.speed_ramp.as_ref().is_none_or(|ramp| {
                ramp.preferred_segments == Some(24)
                    && ramp.segments == 24
                    && ramp.timebase_fps == Some(60.0)
                    && ramp.timebase_audio_rate == Some(48_000)
            }))
        }));
    let project_dir = store.dir.clone();
    drop(store);

    std::fs::remove_file(project_dir.join("project.json")).unwrap();
    let reopened = ProjectStore::open(&project_dir).unwrap();
    let regridded_ramps: Vec<_> = reopened
        .project
        .all_sequence_tracks()
        .flat_map(|track| &track.clips)
        .filter_map(|clip| match clip {
            Clip::Media(media) => media.speed_ramp.as_ref(),
            _ => None,
        })
        .collect();
    assert_eq!(regridded_ramps.len(), 2);
    assert!(regridded_ramps.iter().all(|ramp| {
        ramp.preferred_segments == Some(24)
            && ramp.segments == 24
            && ramp.timebase_fps == Some(60.0)
            && ramp.timebase_audio_rate == Some(48_000)
    }));
    let edl = edl_from_project(&reopened.project);
    let expected_frames = timeline_frame_count(edl.duration_ms, 60.0);
    let output = render_final(
        &reopened.project,
        &edl,
        &PathFence::new(dir.path()).unwrap(),
        Path::new("regridded.mp4"),
        &RenderPreset::default(),
        RenderOptions::default(),
        None,
    )
    .expect("24fps ramp regridded to 60fps render");
    assert!(
        output.duration_ms.abs_diff(edl.duration_ms) <= (1000.0_f64 / 60.0).ceil() as u64,
        "output={} edl={}",
        output.duration_ms,
        edl.duration_ms
    );
    assert_eq!(counted_video_frames(&output.path), expected_frames);
}
