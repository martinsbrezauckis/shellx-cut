//! Server tests for converting recording take markers (`studio-events.jsonl`)
//! into clip markers when added via raw take, polished take, or recovered take,
//! plus verifying legacy 0.6.121 projects are never rewritten on read.

use super::super::dispatch;
use super::test_actor;
use crate::state::AppState;
use cut_core::Clip;
use serde_json::json;
use std::path::Path;

fn synth_recording_mp4(path: &Path, seconds: u32) {
    let status = std::process::Command::new(cut_media::toolpath::ffmpeg())
        .args([
            "-nostats",
            "-loglevel",
            "error",
            "-f",
            "lavfi",
            "-i",
            &format!("testsrc=size=320x180:rate=30:duration={seconds}"),
            "-f",
            "lavfi",
            "-i",
            "anullsrc=r=48000:cl=stereo",
            "-map",
            "0:v:0",
            "-map",
            "1:a:0",
            "-t",
            &seconds.to_string(),
            "-pix_fmt",
            "yuv420p",
            "-c:a",
            "aac",
            "-y",
        ])
        .arg(path)
        .status()
        .unwrap();
    assert!(status.success(), "ffmpeg synthetic mp4 failed");
}

fn write_wav(path: &Path, seconds: u32) {
    let status = std::process::Command::new(cut_media::toolpath::ffmpeg())
        .args([
            "-nostats",
            "-loglevel",
            "error",
            "-f",
            "lavfi",
            "-i",
            &format!("anullsrc=r=48000:cl=stereo:d={seconds}"),
            "-y",
        ])
        .arg(path)
        .status()
        .unwrap();
    assert!(status.success(), "ffmpeg synthetic wav failed");
}

fn write_studio_markers(capture_dir: &Path, markers: &[(u64, &str)]) {
    let mut lines = String::new();
    for (idx, (t_ms, label)) in markers.iter().enumerate() {
        let rec = json!({
            "version": 1,
            "logical_ts": (idx as u64) + 1,
            "event": {
                "logical_ts": (idx as u64) + 1,
                "t_ms": t_ms,
                "source": "recording",
                "kind": "marker",
                "label": label
            }
        });
        lines.push_str(&serde_json::to_string(&rec).unwrap());
        lines.push('\n');
    }
    std::fs::write(capture_dir.join("studio-events.jsonl"), lines).unwrap();
}

async fn wait_job_done(state: &AppState, job_id: &str) -> serde_json::Value {
    for _ in 0..120 {
        let js = dispatch(
            state,
            "jobs.status",
            json!({"job_id": job_id}),
            test_actor(),
        )
        .await;
        let res = js.result.unwrap();
        match res["state"].as_str().unwrap_or("") {
            "done" => return res,
            "failed" => panic!("job {job_id} failed: {res}"),
            _ => tokio::time::sleep(std::time::Duration::from_millis(100)).await,
        }
    }
    panic!("job {job_id} timed out");
}

#[tokio::test]
async fn raw_take_stop_and_import_attaches_clip_markers_on_empty_and_nonempty_timeline() {
    let dir = tempfile::tempdir().unwrap();
    let project_dir = dir.path().join("raw_markers.cutproj");
    let state = AppState::new();
    let created = dispatch(
        &state,
        "project.create",
        json!({"name": "raw_markers", "dir": project_dir}),
        test_actor(),
    )
    .await;
    assert!(created.ok, "{:?}", created.error);

    let cache = crate::screen_record::screen_record_cache_dir(&project_dir).unwrap();
    let cap_dir = cache.join("cap_raw_1");
    std::fs::create_dir_all(&cap_dir).unwrap();
    let source = cap_dir.join("source.mp4");
    synth_recording_mp4(&source, 4);
    write_wav(&cap_dir.join("mic.wav"), 4);
    write_studio_markers(&cap_dir, &[(800, "Intro"), (2_600, "After pause")]);
    std::fs::write(
        cap_dir.join("project.json"),
        serde_json::to_vec(&json!({
            "source_video": source,
            "settings": {"fps": 30.0},
            "events": {
                "duration_ms": 4000, "screen_w": 320, "screen_h": 180,
                "monitors": [], "cursor": [], "scrolls": [], "keys": [], "clicks": []
            }
        }))
        .unwrap(),
    )
    .unwrap();

    let stopped = dispatch(
        &state,
        "screen_record.stop",
        json!({"capture_id": "cap_raw_1", "autoedit": false, "mux_raw": true}),
        test_actor(),
    )
    .await;
    assert!(stopped.ok, "{:?}", stopped.error);
    let raw_path = stopped.result.unwrap()["raw_path"]
        .as_str()
        .unwrap()
        .to_string();

    // 1. Empty timeline: media.import auto-places v1 (+ a1t); v1 gets the markers.
    let imp = dispatch(
        &state,
        "media.import",
        json!({"path": raw_path, "proxy": false}),
        test_actor(),
    )
    .await;
    assert!(imp.ok, "{:?}", imp.error);
    let imp_res = imp.result.unwrap();
    let job_res = wait_job_done(&state, imp_res["job_id"].as_str().unwrap()).await;
    assert_eq!(
        job_res["result"]["placed"], true,
        "expected auto-place: {job_res}"
    );

    {
        let guard = state.project.read().await;
        let project = &guard.as_ref().unwrap().project;
        let Clip::Media(v1_clip) = &project.track("v1").unwrap().clips[0] else {
            panic!("expected video clip");
        };
        assert_eq!(v1_clip.markers.len(), 2);
        assert_eq!(v1_clip.markers[0].source_ms, 800);
        assert_eq!(v1_clip.markers[0].label, "Intro");
        assert_eq!(v1_clip.markers[1].source_ms, 2_600);
        assert_eq!(v1_clip.markers[1].label, "After pause");

        // Linked audio should not duplicate ticks unless audio-only.
        let Clip::Media(a1_clip) = &project.track("a1t").unwrap().clips[0] else {
            panic!("expected linked audio clip");
        };
        assert!(a1_clip.markers.is_empty());
    }

    // 2. Populated second project: edit.insert_linked also attaches capture markers.
    let project_dir2 = dir.path().join("raw_populated.cutproj");
    let created2 = dispatch(
        &state,
        "project.create",
        json!({"name": "raw_populated", "dir": project_dir2}),
        test_actor(),
    )
    .await;
    assert!(created2.ok, "{:?}", created2.error);
    let prior = dir.path().join("prior.mp4");
    synth_recording_mp4(&prior, 2);
    let p_imp = dispatch(
        &state,
        "media.import",
        json!({"path": prior, "proxy": false}),
        test_actor(),
    )
    .await
    .result
    .unwrap();
    wait_job_done(&state, p_imp["job_id"].as_str().unwrap()).await;

    let cap2_dir = crate::screen_record::screen_record_cache_dir(&project_dir2)
        .unwrap()
        .join("cap_raw_2");
    std::fs::create_dir_all(&cap2_dir).unwrap();
    let src2 = cap2_dir.join("source.mp4");
    synth_recording_mp4(&src2, 4);
    write_wav(&cap2_dir.join("mic.wav"), 4);
    write_studio_markers(&cap2_dir, &[(500, "Mark A"), (3_100, "Mark B")]);
    std::fs::write(
        cap2_dir.join("project.json"),
        serde_json::to_vec(&json!({
            "source_video": src2,
            "settings": {"fps": 30.0},
            "events": {
                "duration_ms": 4000, "screen_w": 320, "screen_h": 180,
                "monitors": [], "cursor": [], "scrolls": [], "keys": [], "clicks": []
            }
        }))
        .unwrap(),
    )
    .unwrap();
    let stop2 = dispatch(
        &state,
        "screen_record.stop",
        json!({"capture_id": "cap_raw_2", "autoedit": false, "mux_raw": true}),
        test_actor(),
    )
    .await;
    let raw2 = stop2.result.unwrap()["raw_path"]
        .as_str()
        .unwrap()
        .to_string();
    let imp2 = dispatch(
        &state,
        "media.import",
        json!({"path": raw2, "proxy": false}),
        test_actor(),
    )
    .await
    .result
    .unwrap();
    wait_job_done(&state, imp2["job_id"].as_str().unwrap()).await;
    let placed2 = dispatch(
        &state,
        "edit.insert_linked",
        json!({
            "asset": imp2["asset_id"],
            "video_track": "v1",
            "audio_track": "a1t",
            "at_ms": 2_000,
            "src_range_ms": [0, 4_000]
        }),
        test_actor(),
    )
    .await;
    assert!(placed2.ok, "{:?}", placed2.error);
    let guard = state.project.read().await;
    let store = guard.as_ref().unwrap();
    let Clip::Media(v_second) = store.project.track("v1").unwrap().clips.last().unwrap() else {
        panic!("expected second video clip");
    };
    assert_eq!(v_second.markers.len(), 2);
    assert_eq!(v_second.markers[0].source_ms, 500);
    let take_asset = v_second.asset.clone();
    drop(guard);

    // 3. edit.overwrite in the middle of the take clip (splits it, so a second
    // new id is minted): the markers go to the overwriting take clip itself.
    let over = dispatch(
        &state,
        "edit.overwrite",
        json!({"asset": take_asset, "video_track": "v1", "at_ms": 3_000, "src_range_ms": [0, 1_000]}),
        test_actor(),
    )
    .await;
    assert!(over.ok, "{:?}", over.error);
    let over_id = over.result.unwrap()["tracks"][0]["clip_id"]
        .as_str()
        .unwrap()
        .to_string();
    let guard = state.project.read().await;
    let store = guard.as_ref().unwrap();
    let over_clip = store
        .project
        .track("v1")
        .unwrap()
        .clips
        .iter()
        .find_map(|c| match c {
            Clip::Media(m) if m.id == over_id => Some(m),
            _ => None,
        })
        .expect("overwrite clip on v1");
    assert_eq!(
        (over_clip.asset.as_str(), over_clip.src_out_ms),
        (take_asset.as_str(), 1_000)
    );
    let labels: Vec<&str> = over_clip.markers.iter().map(|m| m.label.as_str()).collect();
    assert_eq!(labels, vec!["Mark A", "Mark B"]);
    // The split-off right part of the earlier take clip keeps its own copy.
    let right_parts = store
        .project
        .track("v1")
        .unwrap()
        .clips
        .iter()
        .filter(|c| matches!(c, Clip::Media(m) if m.id != over_id && m.asset == take_asset && m.markers.len() == 2))
        .count();
    assert_eq!(
        right_parts, 2,
        "left and right parts of the overwritten take clip keep markers"
    );
    // Replaying the op log rebuilds the same clips with the same markers (asset
    // probes are not part of the log, so compare the timeline).
    let rebuilt = cut_core::rebuild_from_log(&store.log.read_all().unwrap()).unwrap();
    assert_eq!(
        serde_json::to_string(&rebuilt.tracks).unwrap(),
        serde_json::to_string(&store.project.tracks).unwrap()
    );
}

#[tokio::test]
async fn polished_take_attaches_markers_and_emits_notice_when_mapping_unknown() {
    let dir = tempfile::tempdir().unwrap();
    let project_dir = dir.path().join("polish_markers.cutproj");
    let state = AppState::new();
    dispatch(
        &state,
        "project.create",
        json!({"name": "polish_markers", "dir": project_dir}),
        test_actor(),
    )
    .await;

    let cap = crate::screen_record::screen_record_cache_dir(&project_dir)
        .unwrap()
        .join("cap_polish");
    std::fs::create_dir_all(&cap).unwrap();
    let source = cap.join("source.mp4");
    synth_recording_mp4(&source, 3);
    write_studio_markers(&cap, &[(750, "Step 1"), (2_200, "Step 2")]);

    let valid_plan = record_core::EditPlan::empty(320, 180, 3_000, 30.0);
    let valid_plan_path = cap.join("plan-valid.json");
    std::fs::write(
        &valid_plan_path,
        serde_json::to_vec_pretty(&valid_plan).unwrap(),
    )
    .unwrap();

    let pr = dispatch(
        &state,
        "screen_record.polish",
        json!({"source": source.display().to_string(), "plan": valid_plan_path.display().to_string()}),
        test_actor(),
    )
    .await;
    assert!(pr.ok, "{:?}", pr.error);
    let pres = pr.result.unwrap();
    assert_eq!(pres["clip_markers_added"], 2);
    assert!(pres["clip_markers_notice"].is_null(), "{pres}");
    let cid = pres["clip_id"].as_str().unwrap().to_string();

    {
        let guard = state.project.read().await;
        let p = &guard.as_ref().unwrap().project;
        let (tr, idx) = p.find_clip(&cid).unwrap();
        let Clip::Media(m) = &p.track(tr).unwrap().clips[idx] else {
            panic!("expected media clip");
        };
        assert_eq!(m.markers.len(), 2);
        assert_eq!(m.markers[0].source_ms, 750);
        assert_eq!(m.markers[1].label, "Step 2");
    }

    // Mismatched duration plan: mapping cannot be known -> no markers added AND clear notice!
    let mismatched_plan = record_core::EditPlan::empty(320, 180, 9_000, 30.0);
    let mismatched_plan_path = cap.join("plan-mismatched.json");
    std::fs::write(
        &mismatched_plan_path,
        serde_json::to_vec_pretty(&mismatched_plan).unwrap(),
    )
    .unwrap();
    let pr2 = dispatch(
        &state,
        "screen_record.polish",
        json!({
            "source": source.display().to_string(),
            "plan": mismatched_plan_path.display().to_string(),
            "at_ms": 3_000
        }),
        test_actor(),
    )
    .await;
    assert!(pr2.ok, "{:?}", pr2.error);
    let pres2 = pr2.result.unwrap();
    assert_eq!(pres2["clip_markers_added"], 0, "{pres2}");
    let notice = pres2["clip_markers_notice"]
        .as_str()
        .expect("must report honest notice when mapping is unknown");
    assert!(
        notice.starts_with("2 take markers were not added to the polished clip")
            && notice.contains("polish settings 9.0 s"),
        "{notice}"
    );
    let cid2 = pres2["clip_id"].as_str().unwrap().to_string();
    let guard = state.project.read().await;
    let p = &guard.as_ref().unwrap().project;
    let (tr2, idx2) = p.find_clip(&cid2).unwrap();
    let Clip::Media(m2) = &p.track(tr2).unwrap().clips[idx2] else {
        panic!("expected second media clip");
    };
    assert!(m2.markers.is_empty());
}

#[tokio::test]
async fn recovered_take_attaches_in_range_clip_markers_and_notes_unrecovered() {
    let dir = tempfile::tempdir().unwrap();
    let project_dir = dir.path().join("recover_markers.cutproj");
    let state = AppState::new();
    dispatch(
        &state,
        "project.create",
        json!({"name": "recover_markers", "dir": project_dir}),
        test_actor(),
    )
    .await;

    let cap_id = "cap_recover_1";
    let cap_dir = crate::screen_record::create_capture_dir(&project_dir, cap_id).unwrap();
    let cap_dir = crate::screen_record::strip_verbatim_prefix(&cap_dir);
    record_recovery::ManifestOwner::begin(
        &cap_dir,
        record_recovery::CaptureStart::new(cap_id, 15_000),
    )
    .unwrap();
    let source = cap_dir.join("source.mp4");
    synth_recording_mp4(&source, 3);
    let mut owner = record_recovery::ManifestOwner::open(&cap_dir).unwrap();
    owner
        .publish_receipt(record_recovery::RecoveryReceipt {
            state: record_recovery::RecoveryState::Complete,
            recovered_segments: 0,
            lost_tail_ms: Some(0),
            lost_tail_lower_bound_ms: 0,
            lost_tail_upper_bound_ms: Some(0),
            audio_first_packet_offset_ms: None,
            source: Some("source.mp4".into()),
            note: "test".into(),
        })
        .unwrap();
    write_studio_markers(&cap_dir, &[(600, "Good mark"), (8_500, "Past end")]);

    let rec = dispatch(
        &state,
        "screen_record.recover",
        json!({"capture_id": cap_id}),
        test_actor(),
    )
    .await;
    assert!(rec.ok, "{:?}", rec.error);
    let res = rec.result.unwrap();
    let clip_id = res["clip_ids"][0].as_str().unwrap().to_string();
    let guard = state.project.read().await;
    let p = &guard.as_ref().unwrap().project;
    let (tr, idx) = p.find_clip(&clip_id).unwrap();
    let Clip::Media(m) = &p.track(tr).unwrap().clips[idx] else {
        panic!("expected recovered media clip");
    };
    assert_eq!(m.markers.len(), 1);
    assert_eq!(m.markers[0].source_ms, 600);
    assert_eq!(m.markers[0].label, "Good mark");
    let msg = res["message"].as_str().unwrap_or("");
    assert!(
        msg.contains("marker") || res["clip_markers_notice"].as_str().is_some(),
        "unrecovered marker beyond saved duration must not be silently dropped: {res}"
    );
}

/// A take paused between its two markers: the Record panel writes each marker's
/// t_ms from the recorder's recorded time (pause excluded, `getRecordedMs` ->
/// `recordedMs`), so the journal already holds recorded time and the import
/// takes it as the clip's source time unchanged. Non-marker events (camera,
/// other sources) are ignored; an empty label reads "Marker".
#[test]
fn journal_markers_after_a_pause_keep_their_recorded_time() {
    let dir = tempfile::tempdir().unwrap();
    let cap = dir.path();
    // 2 s recorded, then a 30 s pause (no events, nothing recorded), then the
    // second marker 1.5 s after resuming = 3.5 s of recorded time.
    let records = [
        json!({"t_ms": 2_000, "source": "recording", "kind": "marker", "label": "Before pause"}),
        json!({"t_ms": 2_100, "source": "camera", "kind": "transform", "x": 0.1, "y": 0.1}),
        json!({"t_ms": 3_500, "source": "recording", "kind": "marker", "label": "  "}),
    ];
    let mut lines = String::new();
    for (i, ev) in records.iter().enumerate() {
        let mut ev = ev.clone();
        ev["logical_ts"] = json!(i + 1);
        lines.push_str(
            &serde_json::to_string(&json!({"version": 1, "logical_ts": i + 1, "event": ev}))
                .unwrap(),
        );
        lines.push('\n');
    }
    std::fs::write(cap.join("studio-events.jsonl"), lines).unwrap();
    let seeds = crate::screen_record::clip_markers::read_capture_markers(cap);
    let got: Vec<(u64, &str)> = seeds
        .iter()
        .map(|m| (m.source_ms, m.label.as_str()))
        .collect();
    assert_eq!(got, vec![(2_000, "Before pause"), (3_500, "Marker")]);
}
