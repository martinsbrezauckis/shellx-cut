use super::{dispatch, test_actor};
use crate::state::AppState;
use serde_json::json;

#[tokio::test]
async fn reversing_video_follows_only_its_linked_audio_and_replays() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("small.mp4");
    let status = std::process::Command::new(cut_media::ffmpeg::ffmpeg_bin())
        .args(["-v", "error", "-y", "-f", "lavfi", "-i"])
        .arg("testsrc2=size=320x180:rate=30:duration=2")
        .args([
            "-c:v",
            "libx264",
            "-preset",
            "ultrafast",
            "-pix_fmt",
            "yuv420p",
        ])
        .arg(&source)
        .status()
        .expect("ffmpeg is needed for this source-facts test");
    assert!(status.success());

    let state = AppState::new();
    let created = dispatch(
        &state,
        "project.create",
        json!({"name": "reverse-admission", "dir": root.path().join("reverse-admission.cutproj")}),
        test_actor(),
    )
    .await;
    assert!(created.ok, "{:?}", created.error);
    {
        let mut guard = state.project.write().await;
        guard
            .as_mut()
            .unwrap()
            .record_import(
                Some("a1".into()),
                cut_core::Asset {
                    path: source.display().to_string(),
                    hash: "sha256:received".into(),
                    probe: Some(json!({"kind": "video", "duration_ms": 2_000})),
                    transcript: None,
                    perception: None,
                    proxy: None,
                    filmstrip: None,
                },
                test_actor(),
                None,
            )
            .unwrap();
        // A received cache can replace the display probe without earning a
        // verified-probe binding for these source facts.
        let project = &mut guard.as_mut().unwrap().project;
        project.assets.get_mut("a1").unwrap().probe = Some(json!({
            "kind": "video", "duration_ms": 2_000,
            "width": 16_384, "height": 16_384, "fps": 1_000.0,
            "raw": {"streams": [{"codec_type": "video", "pix_fmt": "yuva444p16le"}]}
        }));
        assert!(project.verified_probe("a1").is_none());
    }
    let inserted = dispatch(
        &state,
        "edit.insert",
        json!({"asset": "a1", "track": "v1", "at_ms": 0, "src_range_ms": [0, 2_000]}),
        test_actor(),
    )
    .await;
    assert!(inserted.ok, "{:?}", inserted.error);
    let clip = inserted.result.unwrap()["clip_id"]
        .as_str()
        .unwrap()
        .to_string();
    // Only the audio with the same asset, source range and timeline span is linked.
    let audio = dispatch(
        &state,
        "edit.insert",
        json!({"asset":"a1", "track":"a1t", "at_ms":0, "src_range_ms":[0,2_000], "ripple":false}),
        test_actor(),
    )
    .await;
    assert!(audio.ok, "{:?}", audio.error);
    let audio = audio.result.unwrap()["clip_id"]
        .as_str()
        .unwrap()
        .to_string();
    let other = dispatch(
        &state,
        "edit.insert",
        json!({"asset":"a1", "track":"a1t", "at_ms":2_000, "src_range_ms":[0,1_000], "ripple":false}),
        test_actor(),
    )
    .await;
    assert!(other.ok, "{:?}", other.error);
    let other = other.result.unwrap()["clip_id"]
        .as_str()
        .unwrap()
        .to_string();
    let before = state.project.read().await.as_ref().unwrap().project.clone();
    let edl = cut_core::edl_from_project(&before);
    let spans: Vec<_> = edl
        .segments
        .iter()
        .filter(|s| s.clip_id.as_deref() == Some(&clip) || s.clip_id.as_deref() == Some(&audio))
        .map(|s| (s.timeline_in_ms, s.timeline_out_ms))
        .collect();
    assert_eq!(
        spans,
        vec![(0, 2_000), (0, 2_000)],
        "the audio is linked before reversal"
    );
    let reversed = dispatch(&state, "edit.reverse", json!({"clip": clip}), test_actor()).await;
    assert!(
        reversed.ok,
        "measured 320x180 source should fit: {:?}",
        reversed.error
    );
    let after = state.project.read().await.as_ref().unwrap().project.clone();
    let reversed = |p: &cut_core::Project, id: &str| {
        let (track, idx) = p.find_clip(id).unwrap();
        matches!(&p.track(track).unwrap().clips[idx], cut_core::Clip::Media(m) if m.reverse)
    };
    assert!(reversed(&after, &clip));
    assert!(
        reversed(&after, &audio),
        "linked sound must reverse with video"
    );
    assert!(
        !reversed(&after, &other),
        "independent sound must stay forward"
    );
    let log = state
        .project
        .read()
        .await
        .as_ref()
        .unwrap()
        .log
        .read_all()
        .unwrap();
    // The received probe was injected only into the display cache, not the log.
    assert_eq!(
        cut_core::rebuild_from_log(&log).unwrap().tracks,
        after.tracks
    );
    assert!(
        dispatch(&state, "project.undo", json!({}), test_actor())
            .await
            .ok
    );
    assert_eq!(state.project.read().await.as_ref().unwrap().project, before);
    assert!(
        dispatch(&state, "project.redo", json!({}), test_actor())
            .await
            .ok
    );
    assert_eq!(state.project.read().await.as_ref().unwrap().project, after);
    assert!(
        dispatch(
            &state,
            "edit.reverse",
            json!({"clip":clip,"enabled":false}),
            test_actor()
        )
        .await
        .ok
    );
    let cleared = state.project.read().await.as_ref().unwrap().project.clone();
    assert!(!reversed(&cleared, &clip));
    assert!(!reversed(&cleared, &audio));
    assert!(!reversed(&cleared, &other));
    state
        .project
        .write()
        .await
        .as_mut()
        .unwrap()
        .project
        .track_mut("a1t")
        .unwrap()
        .locked = true;
    let locked = state.project.read().await.as_ref().unwrap().project.clone();
    let refused = dispatch(
        &state,
        "edit.reverse",
        json!({"clip":clip,"enabled":false}),
        test_actor(),
    )
    .await;
    assert!(
        !refused.ok,
        "a locked linked audio leg must refuse the whole action"
    );
    assert_eq!(state.project.read().await.as_ref().unwrap().project, locked);
}

#[tokio::test]
async fn a_received_video_probe_cannot_refuse_audio_reverse() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("tone.wav");
    let status = std::process::Command::new(cut_media::ffmpeg::ffmpeg_bin())
        .args(["-v", "error", "-y", "-f", "lavfi", "-i"])
        .arg("sine=frequency=440:duration=2")
        .arg(&source)
        .status()
        .expect("ffmpeg is needed for this source-facts test");
    assert!(status.success());

    let state = AppState::new();
    let created = dispatch(
        &state,
        "project.create",
        json!({"name": "reverse-audio", "dir": root.path().join("reverse-audio.cutproj")}),
        test_actor(),
    )
    .await;
    assert!(created.ok, "{:?}", created.error);
    {
        let mut guard = state.project.write().await;
        guard
            .as_mut()
            .unwrap()
            .record_import(
                Some("a1".into()),
                cut_core::Asset {
                    path: source.display().to_string(),
                    hash: "sha256:received-audio".into(),
                    probe: Some(json!({
                        "kind": "audio", "duration_ms": 2_000, "has_audio": true,
                        "width": 16_384, "height": 16_384, "fps": 1_000.0,
                        "raw": {"streams": [{"codec_type": "video", "pix_fmt": "yuva444p16le"}]}
                    })),
                    transcript: None,
                    perception: None,
                    proxy: None,
                    filmstrip: None,
                },
                test_actor(),
                None,
            )
            .unwrap();
    }
    let inserted = dispatch(
        &state,
        "edit.insert",
        json!({"asset": "a1", "track": "a1t", "at_ms": 0, "src_range_ms": [0, 2_000]}),
        test_actor(),
    )
    .await;
    assert!(inserted.ok, "{:?}", inserted.error);
    let clip = inserted.result.unwrap()["clip_id"]
        .as_str()
        .unwrap()
        .to_string();
    let reversed = dispatch(&state, "edit.reverse", json!({"clip": clip}), test_actor()).await;
    assert!(
        reversed.ok,
        "audio reverse should not use video frame size: {:?}",
        reversed.error
    );
}
