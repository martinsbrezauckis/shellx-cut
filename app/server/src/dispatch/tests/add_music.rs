//! `audio.add_music` end to end at the dispatch layer.

use super::*;

/// audio.add_music: places a bed on an auto-created music1 track,
/// auto-ducks under the base speech track using perception silences
/// (windows RECORDED on the lowered edit.duck step), surfaces beat:N
/// markers from the music asset's perception BeatGrid, and applies the bed
/// gain. Proves the whole lowered chain end-to-end at the dispatch layer.
#[tokio::test]
async fn add_music_beds_ducks_and_marks_beats() {
    let dir = tempfile::tempdir().unwrap();
    // narrowing_fixture: a1 placed on v1 [src 0-5000] + a1t [src 5000-10000]
    // at timeline [0,5000). a1t is the speech track the bed ducks under.
    let state = narrowing_fixture(dir.path()).await;
    let receipts = dir.path().join("t.cutproj/receipts");
    std::fs::create_dir_all(&receipts).unwrap();
    // Speech facts on a1 (the VO): silences [5500,6500]+[8000,10000] in
    // source → speech complement within a1t's window [5000,10000) maps to
    // timeline [0,500)+[1500,3000) (same maths as the duck test).
    engine_reports::write_engine_report(
        receipts.join("a1.perception.json"),
        serde_json::to_string(&json!({
            "schema": "shellx-cut/perception/1",
            "asset_hash": "sha256:test",
            "source_path": "clip.mp4",
            "instruments_run": ["silence"],
            "silences": [
                {"start_ms": 5500, "end_ms": 6500, "source": "both"},
                {"start_ms": 8000, "end_ms": 10000, "source": "both"}
            ],
        }))
        .unwrap(),
    )
    .unwrap();
    // Import a "music" asset (dummy file) + give it a probe duration and a
    // perception report with a beat grid (the import chain doesn't run in
    // tests, so we wire the facts directly — the same pattern the duck test
    // uses for silences).
    let music = dir.path().join("music.wav");
    std::fs::write(&music, b"not-really-audio").unwrap();
    let r = dispatch(&state, "media.import", json!({"path": music}), test_actor()).await;
    assert!(r.ok, "{:?}", r.error);
    let music_asset = r.result.unwrap()["asset_id"].as_str().unwrap().to_string();
    update_asset(&state, &music_asset, |a| {
        a.probe = Some(json!({"kind":"audio","duration_ms":4000,"has_audio":true}));
    })
    .await
    .unwrap();
    engine_reports::write_engine_report(
        receipts.join(format!("{music_asset}.perception.json")),
        serde_json::to_string(&json!({
            "schema": "shellx-cut/perception/1",
            "asset_hash": "sha256:music",
            "source_path": "music.wav",
            "instruments_run": ["beats"],
            // Beats every 500ms (120 bpm) within [0,4000): 0,500...,3500.
            "beats": {"bpm": 120.0, "beats_ms": [0, 500, 1000, 1500, 2000, 2500, 3000, 3500]},
        }))
        .unwrap(),
    )
    .unwrap();

    // Add the music bed (defaults: music1 track, bed_gain -18, auto-duck
    // against the base speech track a1t at -15, beat markers on).
    let r = dispatch(
        &state,
        "audio.add_music",
        json!({"asset": music_asset, "rationale": "podcast bed"}),
        test_actor(),
    )
    .await;
    assert!(r.ok, "{:?}", r.error);
    let res = r.result.unwrap();
    assert_eq!(
        res["track_id"], "music1",
        "auto-created the dedicated bed track"
    );
    assert_eq!(res["created_track"], true);
    assert_eq!(res["bed_gain_db"], -18.0);
    assert_eq!(res["bed_duration_ms"], 4000);
    // Auto-duck produced 2 windows from the VO speech (same as duck test).
    assert_eq!(
        res["ducked_windows"], 2,
        "ducked under the two speech spans"
    );
    // 8 beats within [0,4000) → 8 markers.
    assert_eq!(res["beats_marked"], 8);

    // State checks: the bed clip exists on music1, gain set, duck windows
    // recorded on the track, and 8 beat markers landed.
    let s = dispatch(&state, "project.state", json!({}), test_actor()).await;
    let project: cut_core::Project = serde_json::from_value(s.result.unwrap()).unwrap();
    let bed_track = project.track("music1").expect("music1 track created");
    match bed_track.clips.first().expect("bed clip placed") {
        cut_core::Clip::Media(c) => {
            assert_eq!(c.asset, music_asset);
            assert_eq!(c.gain_db, -18.0, "bed gain applied");
        }
        _ => unreachable!("bed clip should be media"),
    }
    assert_eq!(
        bed_track.gain_windows.len(),
        2,
        "auto-duck windows on the bed track"
    );
    assert_eq!(bed_track.gain_windows[0].db, -15.0, "default duck depth");
    let beats: Vec<&cut_core::Marker> = project
        .markers
        .iter()
        .filter(|m| m.label == "beat")
        .collect();
    assert_eq!(beats.len(), 8, "one marker per beat in range");
    assert_eq!(beats[0].at_ms, 0);
    assert_eq!(
        beats[1].at_ms, 500,
        "beat at source 500 → timeline 500 (bed at 0)"
    );

    // Replay determinism: the whole lowered chain (add_track + insert +
    // gain + duck + 8 add_marker) reproduces the TIMELINE byte-identically
    // from the log. (Asset.probe is a CACHE field written by update_asset,
    // not an op — replay legitimately lacks it; compare tracks+markers, the
    // state the lowered steps actually produce.)
    let ops = dispatch(&state, "project.ops", json!({}), test_actor()).await;
    let log: Vec<cut_core::OpRecord> =
        serde_json::from_value(ops.result.unwrap()["ops"].clone()).unwrap();
    let rebuilt = cut_core::rebuild_from_log(&log).expect("replay");
    assert_eq!(rebuilt.tracks, project.tracks, "tracks replay identically");
    assert_eq!(
        rebuilt.markers, project.markers,
        "beat markers replay identically"
    );

    // duck:false skips ducking; beat_markers:false skips markers.
    let r = dispatch(
            &state,
            "audio.add_music",
            json!({"asset": music_asset, "track": "music1", "at_ms": 5000, "duck": false, "beat_markers": false}),
            test_actor(),
        )
        .await;
    assert!(r.ok, "{:?}", r.error);
    let res = r.result.unwrap();
    assert_eq!(res["ducked_windows"], 0, "duck:false skips ducking");
    assert_eq!(res["beats_marked"], 0, "beat_markers:false skips markers");
    assert_eq!(
        res["created_track"], false,
        "reused the existing music1 track"
    );
}
