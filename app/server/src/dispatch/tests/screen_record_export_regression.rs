//! Real-FFmpeg regression for recorder stop → autoedit → export timebase/audio.

use std::path::{Path, PathBuf};

use super::super::dispatch;
use super::test_actor;
use crate::state::AppState;
use serde_json::{json, Value};

mod fixtures;
use fixtures::*;

#[test]
fn polished_stop_saves_collision_safe_raw_mp4_and_editable_plan() {
    let _output_fixture = crate::output_paths::test_fixture::SessionOutputDirFixture::new();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        let temp = tempfile::tempdir().unwrap();
        let project_dir = temp.path().join("polished_raw.cutproj");
        let state = AppState::new();
        let created = dispatch(
            &state,
            "project.create",
            json!({"name":"polished_raw","dir":project_dir}),
            test_actor(),
        )
        .await;
        assert!(created.ok, "{:?}", created.error);

        let mut first: Option<Value> = None;
        for (capture_id, name) in [
            ("cap-polished-1", "raw_recording.mp4"),
            ("cap-polished-2", "raw_recording-2.mp4"),
        ] {
            let capture = crate::screen_record::screen_record_cache_dir(&project_dir)
                .unwrap()
                .join(capture_id);
            std::fs::create_dir_all(&capture).unwrap();
            let source = capture.join("source.mp4");
            synth_video(&source, 1);
            std::fs::write(
                capture.join("project.json"),
                serde_json::to_vec(&recording_project(&source, None, 1_000)).unwrap(),
            )
            .unwrap();
            let stopped = dispatch(
                &state,
                "screen_record.stop",
                json!({"capture_id":capture_id,"mux_raw":true,"autoedit":true}),
                test_actor(),
            )
            .await;
            assert!(stopped.ok, "combined Stop failed: {:?}", stopped.error);
            let result = stopped.result.unwrap();
            let raw = PathBuf::from(result["raw_path"].as_str().unwrap());
            assert_eq!(raw.file_name().unwrap(), name);
            assert_eq!(raw.parent().unwrap(), project_dir.join("exports"));
            assert!(raw.metadata().unwrap().len() > 0);
            assert!(Path::new(result["plan"].as_str().unwrap()).is_file());
            if first.is_none() {
                first = Some(result);
            }
        }

        let first = first.unwrap();
        let polished = dispatch(
            &state,
            "screen_record.polish",
            json!({"source":first["source"],"plan":first["plan"]}),
            test_actor(),
        )
        .await;
        assert!(polished.ok, "editable polish failed: {:?}", polished.error);
        assert!(polished.result.unwrap()["clip_id"].is_string());
        assert!(project_dir.join("exports/raw_recording.mp4").is_file());
    });
}

#[test]
fn stop_autoedit_export_preserves_capture_timebase_and_aligned_capture_audio() {
    let _output_fixture = crate::output_paths::test_fixture::SessionOutputDirFixture::new();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        let temp = tempfile::tempdir().unwrap();
        let project_dir = temp.path().join("export_timebase.cutproj");
        let state = AppState::new();
        let created = dispatch(
            &state,
            "project.create",
            json!({"name": "export_timebase", "dir": project_dir}),
            test_actor(),
        )
        .await;
        assert!(created.ok, "project create failed: {:?}", created.error);

        let capture = crate::screen_record::screen_record_cache_dir(&project_dir)
            .unwrap()
            .join("cap-timebase");
        std::fs::create_dir_all(&capture).unwrap();
        let source = capture.join("source.mp4");
        let mic = capture.join("mic.wav");
        let system = capture.join("system.wav");
        synth_video(&source, 60);
        synth_tone(&mic, 440, 60);
        synth_tone(&system, 880, 60);
        std::fs::write(
            capture.join("system-audio.json"),
            br#"{"schema":"shellx-cut/system-audio-timing/1","first_packet_offset_ms":200}"#,
        )
        .unwrap();
        std::fs::write(
            capture.join("project.json"),
            serde_json::to_vec(&recording_project(&source, Some(&mic), 60_000)).unwrap(),
        )
        .unwrap();

        let stopped = dispatch(
            &state,
            "screen_record.stop",
            json!({"capture_id": "cap-timebase", "autoedit": true}),
            test_actor(),
        )
        .await;
        assert!(stopped.ok, "stop failed: {:?}", stopped.error);
        let plan = PathBuf::from(
            stopped.result.unwrap()["plan"]
                .as_str()
                .expect("stop autoedit plan"),
        );
        assert!(
            plan.is_file(),
            "stop returned a missing plan: {}",
            plan.display()
        );
        let plan_json: Value = serde_json::from_slice(&std::fs::read(&plan).unwrap()).unwrap();
        assert_eq!(plan_json["fps"], 25.0);
        assert_eq!(plan_json["duration_ms"], 60_000);

        let exports = project_dir.join("exports");
        std::fs::create_dir_all(&exports).unwrap();
        let output = exports.join("timebase-audio.mp4");
        let result = export_capture(&state, &source, &plan, &output).await;
        assert_eq!(result["frames"], 1_500);
        assert!(
            output.is_file(),
            "export output missing: {}",
            output.display()
        );
        let facts = ffprobe(&output);
        let streams = facts["streams"].as_array().expect("ffprobe streams");
        let video = streams
            .iter()
            .find(|stream| stream["codec_type"] == "video")
            .expect("output video stream");
        assert_eq!(video["avg_frame_rate"], "25/1");
        assert_eq!(video["nb_read_frames"], "1500");
        let duration = facts["format"]["duration"]
            .as_str()
            .expect("output duration")
            .parse::<f64>()
            .unwrap();
        assert!((duration - 60.0).abs() < 0.12, "wrong duration: {duration}");
        let audio = streams
            .iter()
            .find(|stream| stream["codec_type"] == "audio")
            .expect("output audio stream");
        assert!(
            audio["sample_rate"]
                .as_str()
                .and_then(|rate| rate.parse::<u32>().ok())
                .is_some_and(|rate| rate >= 48_000),
            "output audio has no usable sample rate: {audio}"
        );
        assert_eq!(audio["channels"], 2);
        let mic_only_rms = audio_rms(&output, 0.050);
        let mixed_rms = audio_rms(&output, 0.500);
        assert!(mic_only_rms > 100.0, "mic was not audible: {mic_only_rms}");
        assert!(
            mixed_rms > mic_only_rms * 1.20,
            "system tone was not mixed after its 200ms alignment delay: before={mic_only_rms}, after={mixed_rms}"
        );
    });
}

#[test]
fn export_system_only_audio_materializes_its_packet_offset() {
    let _output_fixture = crate::output_paths::test_fixture::SessionOutputDirFixture::new();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        let temp = tempfile::tempdir().unwrap();
        let project_dir = temp.path().join("export_system_only.cutproj");
        let state = AppState::new();
        let created = dispatch(
            &state,
            "project.create",
            json!({"name": "export_system_only", "dir": project_dir}),
            test_actor(),
        )
        .await;
        assert!(created.ok, "project create failed: {:?}", created.error);

        let capture = crate::screen_record::screen_record_cache_dir(&project_dir)
            .unwrap()
            .join("cap-system-only");
        std::fs::create_dir_all(&capture).unwrap();
        let source = capture.join("source.mp4");
        let system = capture.join("system.wav");
        synth_video(&source, 1);
        synth_tone(&system, 880, 1);
        std::fs::write(
            capture.join("system-audio.json"),
            br#"{"schema":"shellx-cut/system-audio-timing/1","first_packet_offset_ms":200}"#,
        )
        .unwrap();
        let plan = capture.join("system-only.plan.json");
        std::fs::write(
            &plan,
            serde_json::to_vec(&record_core::EditPlan::empty(160, 90, 1_000, 25.0)).unwrap(),
        )
        .unwrap();
        let exports = project_dir.join("exports");
        std::fs::create_dir_all(&exports).unwrap();
        let output = exports.join("system-only.mp4");
        export_capture(&state, &source, &plan, &output).await;

        let silent_rms = audio_rms(&output, 0.020);
        let audible_rms = audio_rms(&output, 0.400);
        assert!(
            audible_rms > 100.0,
            "system audio was not audible: {audible_rms}"
        );
        assert!(
            audible_rms > silent_rms * 10.0,
            "system-only export ignored its 200ms packet offset: before={silent_rms}, after={audible_rms}"
        );
    });
}
