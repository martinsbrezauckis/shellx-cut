//! roundtrip.rs — cut-core integration tests (timeline/op-log contract gates).
//!
//! The four gates the core build track must prove:
//! 1. DETERMINISM: replaying the same ops.jsonl yields a byte-identical
//!    project.json — twice, and equal to the live materialized state.
//! 2. RESTORE roundtrip: edit.restore(op) returns the timeline to its
//!    exact pre-op state, via an APPENDED op (log never rewritten).
//! 3. CHECKPOINT + REVERT: project.revert(checkpoint) appends restore ops
//!    and lands exactly on the checkpointed timeline.
//! 4. DIFF: ops between two checkpoints + computed summary matches what
//!    the edits actually did.
//!
//! Dependencies: cut-core public API only (what the server sees). Runs on a
//! tempdir; no network, no ffmpeg.

use cut_core::rebase;
use cut_core::store::rebuild_skipping;
use cut_core::{diff, rebuild_from_log, Actor, ActorKind, Asset, ProjectStore};
use serde_json::json;

/// Test actor — an agent over MCP, like production traffic.
fn actor() -> Actor {
    Actor {
        kind: ActorKind::Agent,
        name: "claude".into(),
        via: "mcp".into(),
    }
}

/// A probed 10s asset (no real file needed — core never touches media bytes).
fn asset() -> Asset {
    Asset {
        path: "/testdata/talking_head.mp4".into(),
        hash: "sha256:deadbeef".into(),
        probe: Some(json!({"duration_ms": 10_000})),
        transcript: None,
        perception: None,
        proxy: None,
        filmstrip: None,
    }
}

/// Build a store and run a representative edit session through the single
/// commit path. Returns the store (log on disk in `dir`).
fn build_session(dir: &std::path::Path) -> ProjectStore {
    let mut s = ProjectStore::create(dir, "demo", None).expect("create");
    let (aid, _) = s
        .record_import(None, asset(), actor(), None)
        .expect("import");
    assert_eq!(aid, "a1");
    // Fill video + audio, then cut things up. Every call goes through apply().
    s.apply(
        "edit.insert",
        json!({"asset":"a1","track":"v1","at_ms":0}),
        actor(),
        Some("lay down the main take".into()),
    )
    .expect("insert v1");
    s.apply(
        "edit.insert",
        json!({"asset":"a1","track":"a1t","at_ms":0}),
        actor(),
        None,
    )
    .expect("insert a1t");
    s.apply(
        "edit.split",
        json!({"track":"v1","at_ms":4000}),
        actor(),
        None,
    )
    .expect("split");
    s.apply(
        "edit.ripple_delete",
        json!({"range_ms":[1000,2000]}),
        actor(),
        Some("dead air 1.0-2.0s".into()),
    )
    .expect("ripple");
    s.apply("edit.gain", json!({"track":"a1t","db":-3.0}), actor(), None)
        .expect("gain");
    s.apply(
        "edit.add_marker",
        json!({"at_ms":500,"label":"intro"}),
        actor(),
        None,
    )
    .expect("marker");
    // Two adjustment LAYERS (edit.adjustment): a grade band + an effect band. These
    // allocate deterministic adjN ids (pure project-state fn), so the replay-
    // determinism gate below proves rebuild_from_log re-derives adj1/adj2 identically.
    s.apply(
        "edit.adjustment",
        json!({"range_ms":[0,2000],"grade":{"saturation":0.0}}),
        actor(),
        Some("desaturate the cold open".into()),
    )
    .expect("adjustment grade band");
    s.apply(
        "edit.adjustment",
        json!({"range_ms":[1000,3000],"effect":{"type":"vignette","amount":0.7}}),
        actor(),
        None,
    )
    .expect("adjustment effect band");
    s
}

/// Gate 1 — timeline/op-log contract determinism: same log replay = identical project.json.
#[test]
fn replay_is_deterministic_and_matches_live_state() {
    let dir = tempfile::tempdir().unwrap();
    let store = build_session(dir.path());
    let ops = store.log.read_all().unwrap();

    let rebuilt_once = rebuild_from_log(&ops).expect("replay 1");
    let rebuilt_twice = rebuild_from_log(&ops).expect("replay 2");

    // Replay equals the live materialized state — full struct AND bytes.
    assert_eq!(rebuilt_once, store.project, "replayed state != live state");
    assert_eq!(
        serde_json::to_string_pretty(&rebuilt_once).unwrap(),
        serde_json::to_string_pretty(&store.project).unwrap(),
        "replayed project.json bytes differ from live cache"
    );
    // And replay is self-identical across runs.
    assert_eq!(
        serde_json::to_string(&rebuilt_once).unwrap(),
        serde_json::to_string(&rebuilt_twice).unwrap(),
        "two replays of the same log diverged"
    );

    // Reopen with a deleted cache → open() rebuilds from the log to the
    // same state (the "log is the truth" path).
    std::fs::remove_file(store.dir.join("project.json")).unwrap();
    let reopened = ProjectStore::open(&store.dir).expect("open with no cache");
    assert_eq!(reopened.project, store.project);
}

/// Marker notes are ordinary timeline metadata. Updating them through
/// the verb path must survive both deterministic replay and cold reopen.
#[test]
fn marker_note_update_replays_and_survives_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = build_session(dir.path());
    let marker_id = store.project.markers[0].id.clone();

    store
        .apply(
            "edit.update_marker",
            json!({"id": marker_id, "note": "tighten the intro beat"}),
            actor(),
            Some("add marker review note".into()),
        )
        .expect("update marker note");

    let live_marker = store
        .project
        .markers
        .iter()
        .find(|m| m.id == marker_id)
        .expect("live marker still present");
    assert_eq!(live_marker.note.as_deref(), Some("tighten the intro beat"));
    assert_eq!(live_marker.label, "intro");

    let rebuilt = rebuild_from_log(&store.log.read_all().unwrap()).expect("replay marker note");
    let rebuilt_marker = rebuilt
        .markers
        .iter()
        .find(|m| m.id == marker_id)
        .expect("rebuilt marker still present");
    assert_eq!(
        rebuilt_marker.note.as_deref(),
        Some("tighten the intro beat")
    );
    assert_eq!(rebuilt_marker.label, "intro");

    std::fs::remove_file(store.dir.join("project.json")).unwrap();
    let reopened = ProjectStore::open(&store.dir).expect("cold reopen marker note");
    let reopened_marker = reopened
        .project
        .markers
        .iter()
        .find(|m| m.id == marker_id)
        .expect("reopened marker still present");
    assert_eq!(
        reopened_marker.note.as_deref(),
        Some("tighten the intro beat")
    );
    assert_eq!(reopened_marker.label, "intro");
}

/// Gate 2 — restore roundtrip: the timeline returns to its exact pre-op
/// state and the log GROWS (append-only, never rewritten).
#[test]
fn restore_roundtrip() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = build_session(dir.path());
    let before = s.project.clone();
    let len_before = s.log.read_all().unwrap().len();

    // Mutate, then undo via edit.restore.
    let rec = s
        .apply(
            "edit.ripple_delete",
            json!({"range_ms":[0,3000]}),
            actor(),
            None,
        )
        .expect("ripple");
    assert_ne!(
        s.project.tracks, before.tracks,
        "ripple must change the timeline"
    );
    s.apply(
        "edit.restore",
        json!({"op_id": rec.op_id}),
        actor(),
        Some("reject: cut too aggressive".into()),
    )
    .expect("restore");

    // Timeline (tracks + markers) is back; log gained exactly 2 ops.
    assert_eq!(s.project.tracks, before.tracks);
    assert_eq!(s.project.markers, before.markers);
    assert_eq!(s.log.read_all().unwrap().len(), len_before + 2);

    // Determinism still holds across the restore (replay reproduces it).
    let rebuilt = rebuild_from_log(&s.log.read_all().unwrap()).unwrap();
    assert_eq!(rebuilt, s.project);
}

/// selective-undo guardrail: inverses are full-timeline snapshots, so restoring
/// a NON-TIP op would silently discard every later edit. edit.restore must
/// refuse (code "guardrail", later-op count + project.revert pointer in the
/// error), leave the timeline untouched, and append nothing to the log.
/// Restoring the tip op afterwards still works (the guard is depth-only).
#[test]
fn restore_of_non_tip_op_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = build_session(dir.path());
    let old = s
        .apply(
            "edit.ripple_delete",
            json!({"range_ms":[0,1000]}),
            actor(),
            None,
        )
        .expect("old op");
    let tip = s
        .apply("edit.gain", json!({"track":"a1t","db":-6.0}), actor(), None)
        .expect("tip op");
    let before = s.project.clone();
    let len_before = s.log.read_all().unwrap().len();

    let err = s
        .apply("edit.restore", json!({"op_id": old.op_id}), actor(), None)
        .expect_err("non-tip restore must refuse");
    assert_eq!(err.code, "guardrail");
    assert!(
        err.message.contains("1 timeline op(s) deep"),
        "depth named: {}",
        err.message
    );
    assert!(
        err.suggested_action
            .as_deref()
            .unwrap_or("")
            .contains("project.revert"),
        "rollback path named: {:?}",
        err.suggested_action
    );
    // Nothing changed, nothing appended.
    assert_eq!(s.project, before);
    assert_eq!(s.log.read_all().unwrap().len(), len_before);

    // Tip restore still works; ops without inverses (checkpoint) don't block.
    s.checkpoint("cp-after", actor(), None).expect("checkpoint");
    s.apply("edit.restore", json!({"op_id": tip.op_id}), actor(), None)
        .expect("tip restore allowed through a trailing checkpoint op");
    assert_eq!(
        s.project.track("a1t").unwrap().gain_db,
        -3.0,
        "gain change undone"
    );
}

/// Gate 3 — checkpoint is an op (the append-only operation-log contract) and revert appends restore ops
/// that land exactly on the checkpointed timeline.
#[test]
fn checkpoint_revert_roundtrip() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = build_session(dir.path());
    let (cp, _op) = s
        .checkpoint("before-tighten", actor(), None)
        .expect("checkpoint");
    let at_checkpoint = s.project.clone();

    // The checkpoint itself is in the log AND in project.json.
    let ops = s.log.read_all().unwrap();
    assert_eq!(ops.last().unwrap().verb, "project.checkpoint");
    assert_eq!(ops.last().unwrap().op_id, cp.at_op);
    assert_eq!(s.project.checkpoints.len(), 1);

    // Three more edits, then revert by checkpoint NAME.
    s.apply(
        "edit.ripple_delete",
        json!({"range_ms":[500,1500]}),
        actor(),
        None,
    )
    .unwrap();
    s.apply(
        "edit.add_marker",
        json!({"at_ms":100,"label":"x"}),
        actor(),
        None,
    )
    .unwrap();
    s.apply("edit.gain", json!({"track":"a1t","db":6.0}), actor(), None)
        .unwrap();
    let restored_ids = s.revert("before-tighten", actor()).expect("revert");
    assert_eq!(restored_ids.len(), 1, "Option B: revert is ONE atomic op");
    assert_eq!(
        s.log.read_all().unwrap().last().unwrap().verb,
        "project.revert",
        "the appended op is a project.revert (not N edit.restore peels)"
    );

    // Timeline matches the checkpoint exactly (checkpoints list may differ —
    // it's metadata, not timeline).
    assert_eq!(s.project.tracks, at_checkpoint.tracks);
    assert_eq!(s.project.markers, at_checkpoint.markers);
    assert_eq!(
        s.project.track("a1t").unwrap().gain_db,
        -3.0,
        "gain reverted"
    );

    // Log was appended, never truncated; replay still reproduces the state.
    let all = s.log.read_all().unwrap();
    assert!(all.len() > ops.len());
    assert_eq!(rebuild_from_log(&all).unwrap(), s.project);
}

/// Option B — `project.revert` is a SINGLE atomic op, and undoing it (a tip
/// restore) recomputes the FULL pre-revert edit byte-exact. The old peel macro
/// could only reverse the LAST peeled op, leaving a confusing partial state;
/// this is the regression gate that the atomic revert fixed it without breaking
/// either the reached-state guarantee or log replayability.
#[test]
fn atomic_revert_is_one_op_and_cleanly_undoable() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = build_session(dir.path());
    let (_cp, _) = s.checkpoint("base", actor(), None).expect("checkpoint");
    let at_checkpoint = s.project.clone();

    // Edits AFTER the checkpoint = the "full edit" we must be able to get back.
    s.apply(
        "edit.ripple_delete",
        json!({"range_ms":[500,1500]}),
        actor(),
        None,
    )
    .unwrap();
    s.apply(
        "edit.add_marker",
        json!({"at_ms":100,"label":"x"}),
        actor(),
        None,
    )
    .unwrap();
    let full_edit = s.project.clone();
    assert_ne!(
        full_edit.tracks, at_checkpoint.tracks,
        "the edits changed the timeline"
    );

    // Revert to the checkpoint — ONE op, lands exactly on the checkpoint state.
    let ids = s.revert("base", actor()).expect("revert");
    assert_eq!(ids.len(), 1, "atomic revert appends exactly one op");
    let all = s.log.read_all().unwrap();
    assert_eq!(all.last().unwrap().verb, "project.revert");
    assert_eq!(
        s.project.tracks, at_checkpoint.tracks,
        "revert reached the checkpoint timeline"
    );
    assert_eq!(s.project.markers, at_checkpoint.markers);

    // Undo the revert (tip restore) → the FULL pre-revert edit returns byte-exact.
    let revert_op = all.last().unwrap().op_id.clone();
    s.apply("edit.restore", json!({"op_id": revert_op}), actor(), None)
        .expect("undo the revert");
    assert_eq!(
        s.project.tracks, full_edit.tracks,
        "undo-of-revert restores the full edit"
    );
    assert_eq!(s.project.markers, full_edit.markers);

    // Cold replay of the whole log (revert + its undo) reproduces the live state.
    let all2 = s.log.read_all().unwrap();
    assert_eq!(
        rebuild_from_log(&all2).unwrap(),
        s.project,
        "log with an atomic revert replays"
    );
}

/// Gate 4 — diff between two checkpoints: ops slice + computed summary.
#[test]
fn diff_between_checkpoints() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = ProjectStore::create(dir.path(), "demo", None).unwrap();
    s.record_import(None, asset(), actor(), None).unwrap();
    s.apply(
        "edit.insert",
        json!({"asset":"a1","track":"v1","at_ms":0}),
        actor(),
        None,
    )
    .unwrap();
    // c2 must exist BEFORE checkpoint a so the ripple's shift registers as a
    // MOVE (added/moved are relative to the from-state).
    s.apply(
        "edit.split",
        json!({"track":"v1","at_ms":4000}),
        actor(),
        None,
    )
    .unwrap();
    s.checkpoint("a", actor(), None).unwrap();

    // Between the checkpoints: one ripple — splits c1 (new right half = c3)
    // and shifts c2 left from 4000 to 3000.
    s.apply(
        "edit.ripple_delete",
        json!({"range_ms":[1000,2000]}),
        actor(),
        None,
    )
    .unwrap();
    s.checkpoint("b", actor(), None).unwrap();

    let all = s.log.read_all().unwrap();
    let d = diff(&s.project, &all, &"a".to_string(), &"b".to_string()).expect("diff");

    assert_eq!(d.ops.len(), 2, "ripple + checkpoint b");
    assert_eq!(d.duration_delta_ms, -1000, "ripple tightened by 1s");
    // The ripple split c1 → right half is a fresh clip id (c3).
    assert_eq!(
        d.clips_added,
        vec!["c3".to_string()],
        "added: {:?}",
        d.clips_added
    );
    assert!(d.clips_removed.is_empty());
    // c2 shifted left by the ripple → reported as moved.
    assert_eq!(
        d.clips_moved,
        vec!["c2".to_string()],
        "moved: {:?}",
        d.clips_moved
    );
    // v1 was touched in the removed range.
    let v1 = d
        .tracks_touched
        .iter()
        .find(|t| t.track == "v1")
        .expect("v1 touched");
    assert!(
        v1.ranges_ms.contains(&[1000, 2000]),
        "ranges: {:?}",
        v1.ranges_ms
    );

    // Endpoint validation: reversed refs are an actionable error.
    let err = diff(&s.project, &all, &"b".to_string(), &"a".to_string()).unwrap_err();
    assert_eq!(err.code, "invalid_args");

    // current-head resolution contract: `to:"now"` resolves to the log head — the docs' canonical
    // pre-render invocation runs as written. Here head == checkpoint b's op,
    // so the summary matches diff(a, b) exactly.
    let d_now = diff(&s.project, &all, &"a".to_string(), &"now".to_string()).expect("diff to now");
    assert_eq!(d_now.to_op, all.last().unwrap().op_id);
    assert_eq!(d_now.duration_delta_ms, d.duration_delta_ms);
    assert_eq!(d_now.clips_added, d.clips_added);
    // One more edit past the checkpoint: "now" tracks the new head.
    s.apply("edit.gain", json!({"track":"a1t","db":-2.0}), actor(), None)
        .unwrap();
    let all2 = s.log.read_all().unwrap();
    let d_now2 =
        diff(&s.project, &all2, &"b".to_string(), &"now".to_string()).expect("diff b..now");
    assert_eq!(d_now2.ops.len(), 1, "exactly the post-checkpoint gain op");
    assert_eq!(d_now2.ops[0].verb, "edit.gain");
}

#[test]
fn diff_reports_in_place_trim_as_clip_change() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = ProjectStore::create(dir.path(), "demo", None).unwrap();
    s.record_import(None, asset(), actor(), None).unwrap();
    s.apply(
        "edit.insert",
        json!({"asset":"a1","track":"v1","at_ms":0}),
        actor(),
        None,
    )
    .unwrap();
    s.checkpoint("a", actor(), None).unwrap();
    s.apply(
        "edit.trim",
        json!({"clip":"c1","src_out_ms":4000}),
        actor(),
        None,
    )
    .unwrap();
    s.checkpoint("b", actor(), None).unwrap();

    let all = s.log.read_all().unwrap();
    let d = diff(&s.project, &all, &"a".to_string(), &"b".to_string()).expect("diff");
    assert!(d.clips_added.is_empty());
    assert!(d.clips_removed.is_empty());
    assert_eq!(
        d.clips_moved,
        vec!["c1".to_string()],
        "trimming an existing clip without moving its start must still appear in clip summary"
    );
    assert_eq!(d.duration_delta_ms, -6000);
}

// ===========================================================================
// op-rebase (selective non-tip undo).
// The contract these gates protect: a rebase reproduces the timeline AS IF an
// older op never happened, KEEPS every later op + its allocated ids stable,
// REFUSES (naming the dependents) when a later op depends on the target, and
// APPENDS — never rewrites the log. Existing logs replay byte-identical (the
// gates above already prove that; id-pinning is byte-identical to positional
// allocation in the no-skip case).
// ===========================================================================

/// ID-PINNING: skip-replaying a log with an EARLY op omitted keeps every
/// surviving op's allocated ids stable (does NOT renumber). Without pinning,
/// dropping the op that allocated `c2` would renumber c3→c2, c4→c3, ... and
/// every later id reference would point at the wrong clip. With pinning the
/// recorded ids hold. This is the prerequisite the whole feature rests on.
#[test]
fn rebase_skip_replay_pins_ids_stable() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = ProjectStore::create(dir.path(), "demo", None).unwrap();
    s.record_import(None, asset(), actor(), None).unwrap();
    s.apply(
        "edit.insert",
        json!({"asset":"a1","track":"v1","at_ms":0}),
        actor(),
        None,
    )
    .unwrap(); // c1
               // An INDEPENDENT marker op in the middle — creates m1, referenced by no one.
    let mid = s
        .apply(
            "edit.add_marker",
            json!({"at_ms":1000,"label":"mid"}),
            actor(),
            None,
        )
        .unwrap();
    // Later ops that allocate ids AFTER the middle op: a split (c2) + an insert
    // (c3). With positional allocation these depend on how many clips exist; with
    // pinning they keep their recorded ids regardless of the skipped middle op.
    s.apply(
        "edit.split",
        json!({"track":"v1","at_ms":4000}),
        actor(),
        None,
    )
    .unwrap(); // c2
    s.apply(
        "edit.insert",
        json!({"asset":"a1","track":"v1","at_ms":8000}),
        actor(),
        None,
    )
    .unwrap(); // c3

    let all = s.log.read_all().unwrap();
    let mid_idx = all.iter().position(|o| o.op_id == mid.op_id).unwrap();

    // Skip-replay with the middle marker op omitted.
    let skipped = rebuild_skipping(&all, mid_idx).expect("skip-replay");

    // Every clip id the later ops allocated is PRESENT and UNRENUMBERED.
    let clip_ids: Vec<String> = skipped
        .tracks
        .iter()
        .flat_map(|t| t.clips.iter().filter_map(|c| c.id().map(String::from)))
        .collect();
    assert!(
        clip_ids.contains(&"c1".to_string()),
        "c1 present: {clip_ids:?}"
    );
    assert!(
        clip_ids.contains(&"c2".to_string()),
        "split right half c2 pinned: {clip_ids:?}"
    );
    assert!(
        clip_ids.contains(&"c3".to_string()),
        "inserted c3 pinned: {clip_ids:?}"
    );
    // The skipped op's marker (m1) is gone — that is the whole point.
    assert!(skipped.markers.is_empty(), "rebased-out marker removed");
}

/// DEPENDENCY GATE — successful rebase of an INDEPENDENT middle op, with the
/// later ops preserved and the change APPENDED (log never rewritten). Mirrors
/// the live-proof case (a): undo an old edit.gain on a track, keep later trims.
#[test]
fn rebase_independent_middle_op_succeeds_and_appends() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = ProjectStore::create(dir.path(), "demo", None).unwrap();
    s.record_import(None, asset(), actor(), None).unwrap();
    s.apply(
        "edit.insert",
        json!({"asset":"a1","track":"v1","at_ms":0}),
        actor(),
        None,
    )
    .unwrap(); // c1
    s.apply(
        "edit.insert",
        json!({"asset":"a1","track":"a1t","at_ms":0}),
        actor(),
        None,
    )
    .unwrap(); // c2
               // The TARGET: an old track-gain on a1t — creates no ids, so nothing can
               // depend on it. Provably independent.
    let target = s
        .apply("edit.gain", json!({"track":"a1t","db":-6.0}), actor(), None)
        .unwrap();
    // Later independent ops that must survive untouched.
    s.apply(
        "edit.trim",
        json!({"clip":"c1","src_out_ms":4000}),
        actor(),
        None,
    )
    .unwrap();
    s.apply(
        "edit.add_marker",
        json!({"at_ms":500,"label":"keep"}),
        actor(),
        None,
    )
    .unwrap();

    let len_before = s.log.read_all().unwrap().len();
    let trim_before = s
        .project
        .tracks
        .iter()
        .flat_map(|t| t.clips.iter())
        .find_map(|c| {
            if c.id() == Some("c1") {
                Some(c.timeline_duration_ms())
            } else {
                None
            }
        });
    assert_eq!(trim_before, Some(4000), "c1 trimmed to 4s before rebase");

    // Rebase out the gain op via edit.restore{mode:"rebase"}.
    let rec = s
        .apply(
            "edit.restore",
            json!({"op_id": target.op_id, "mode": "rebase"}),
            actor(),
            Some("reject the -6dB dip, keep the trim + marker".into()),
        )
        .expect("rebase independent op");

    // The gain is undone (a1t back to 0 dB) ...
    assert_eq!(
        s.project.track("a1t").unwrap().gain_db,
        0.0,
        "gain rebased out"
    );
    // ... while the LATER ops are preserved (trim still 4s, marker still there).
    let trim_after = s
        .project
        .tracks
        .iter()
        .flat_map(|t| t.clips.iter())
        .find_map(|c| {
            if c.id() == Some("c1") {
                Some(c.timeline_duration_ms())
            } else {
                None
            }
        });
    assert_eq!(
        trim_after,
        Some(4000),
        "later trim PRESERVED across the rebase"
    );
    assert_eq!(s.project.markers.len(), 1, "later marker PRESERVED");
    assert_eq!(s.project.markers[0].label, "keep");

    // The log was APPENDED to (never rewritten): exactly one new op, the rebase.
    let all = s.log.read_all().unwrap();
    assert_eq!(all.len(), len_before + 1, "rebase appended exactly one op");
    assert_eq!(all.last().unwrap().op_id, rec.op_id);
    assert_eq!(all.last().unwrap().verb, "edit.restore");
    assert_eq!(all.last().unwrap().args["mode"], "rebase");
    // The original target op is STILL in the log unchanged (append-only).
    assert!(all
        .iter()
        .any(|o| o.op_id == target.op_id && o.verb == "edit.gain"));

    // COLD REPLAY of the log INCLUDING the rebase op reproduces the live state.
    let rebuilt = rebuild_from_log(&all).unwrap();
    assert_eq!(
        rebuilt, s.project,
        "replay of log-with-rebase == live state"
    );
}

/// DEPENDENCY GATE — REFUSAL: rebasing out an op a LATER op depends on must be
/// refused with a structured guardrail error that NAMES the dependent op(s).
/// Mirrors live-proof case (b). Nothing is mutated, nothing appended.
#[test]
fn rebase_dependent_op_is_refused_naming_dependents() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = ProjectStore::create(dir.path(), "demo", None).unwrap();
    s.record_import(None, asset(), actor(), None).unwrap();
    s.apply(
        "edit.insert",
        json!({"asset":"a1","track":"v1","at_ms":0}),
        actor(),
        None,
    )
    .unwrap(); // c1
               // The TARGET: a split that CREATES c2 (the right half).
    let target = s
        .apply(
            "edit.split",
            json!({"track":"v1","at_ms":4000}),
            actor(),
            None,
        )
        .unwrap();
    // A LATER op that REFERENCES c2 — this binds it to the target.
    let dependent = s
        .apply(
            "edit.transform",
            json!({"clip":"c2","scale":0.8}),
            actor(),
            None,
        )
        .unwrap();

    let before = s.project.clone();
    let len_before = s.log.read_all().unwrap().len();

    let err = s
        .apply(
            "edit.restore",
            json!({"op_id": target.op_id, "mode": "rebase"}),
            actor(),
            None,
        )
        .expect_err("rebase of a depended-on op must refuse");

    assert_eq!(err.code, "guardrail");
    assert!(
        err.message.contains("cannot be rebased out"),
        "message: {}",
        err.message
    );
    // The dependent op id AND the binding clip id are named in the cause.
    let cause = &err.cause;
    assert!(
        cause.contains(&dependent.op_id),
        "names dependent op: {cause}"
    );
    assert!(cause.contains("c2"), "names the binding clip id: {cause}");
    assert!(
        err.suggested_action
            .as_deref()
            .unwrap_or("")
            .contains("project.revert"),
        "points at an escape hatch: {:?}",
        err.suggested_action
    );

    // Nothing changed, nothing appended (the refusal is total).
    assert_eq!(s.project, before);
    assert_eq!(s.log.read_all().unwrap().len(), len_before);
}

/// REVERT + REPLAY survive a rebase: project.revert still lands on a checkpoint,
/// and cold-replay of a log that CONTAINS a rebase op is deterministic. Mirrors
/// live-proof cases (c) + (d).
#[test]
fn rebase_then_checkpoint_revert_and_cold_replay_hold() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = ProjectStore::create(dir.path(), "demo", None).unwrap();
    s.record_import(None, asset(), actor(), None).unwrap();
    s.apply(
        "edit.insert",
        json!({"asset":"a1","track":"v1","at_ms":0}),
        actor(),
        None,
    )
    .unwrap();
    s.apply(
        "edit.insert",
        json!({"asset":"a1","track":"a1t","at_ms":0}),
        actor(),
        None,
    )
    .unwrap();
    let target = s
        .apply("edit.gain", json!({"track":"a1t","db":-4.0}), actor(), None)
        .unwrap();
    s.apply(
        "edit.add_marker",
        json!({"at_ms":500,"label":"m"}),
        actor(),
        None,
    )
    .unwrap();
    // Rebase out the independent gain.
    s.apply(
        "edit.restore",
        json!({"op_id": target.op_id, "mode":"rebase"}),
        actor(),
        None,
    )
    .expect("rebase");
    let after_rebase = s.project.clone();

    // (c) checkpoint AFTER the rebase, edit more, then revert to the checkpoint.
    let (_cp, _) = s
        .checkpoint("post-rebase", actor(), None)
        .expect("checkpoint");
    s.apply("edit.gain", json!({"track":"a1t","db":3.0}), actor(), None)
        .unwrap();
    s.revert("post-rebase", actor())
        .expect("revert past a rebase");
    assert_eq!(
        s.project.tracks, after_rebase.tracks,
        "revert lands on post-rebase timeline"
    );

    // (d) cold replay of the FULL log (incl. the rebase op) == live state.
    let all = s.log.read_all().unwrap();
    let rebuilt = rebuild_from_log(&all).unwrap();
    assert_eq!(
        rebuilt, s.project,
        "cold replay of log-with-rebase reproduces live state"
    );
    // And re-open from disk with the cache deleted → identical (the log truth).
    std::fs::remove_file(s.dir.join("project.json")).unwrap();
    let reopened = ProjectStore::open(&s.dir).expect("reopen cold");
    assert_eq!(
        reopened.project, s.project,
        "cold reopen reproduces the rebased state"
    );
}

/// PURE DEPENDENCY ANALYSIS over a realistic log: the gate must see through a
/// lowered op (transcript.* lowering to ripple_delete) and refuse when a later
/// op references a clip a lowered split created.
#[test]
fn rebase_analysis_sees_lowered_op_outputs() {
    // Build a log where a lowered-style ripple split records split_clip c9, and
    // a later trim references c9. can_rebase_out must return false for the
    // lowered op (index 0) because the trim (index 1) depends on its output.
    use cut_core::{OpEffect, OpRecord, OpStatus};
    fn mk(op_id: &str, verb: &str, args: serde_json::Value, effects: Vec<OpEffect>) -> OpRecord {
        OpRecord {
            op_id: op_id.into(),
            ts: "2026-06-11T00:00:00.000Z".into(),
            actor: actor(),
            verb: verb.into(),
            args,
            rationale: None,
            effects,
            inverse: Some(cut_core::InverseOp {
                verb: "edit._set_timeline".into(),
                args: json!({}),
            }),
            status: OpStatus::Applied,
        }
    }
    let lowered_step =
        json!([{"verb":"edit.ripple_delete","args":{"track":"v1","range_ms":[1000,2000]}}]);
    let ops = vec![
        mk(
            "op_000001",
            "transcript.cut_words",
            json!({"asset":"a1"}),
            vec![
                OpEffect {
                    track: Some("v1".into()),
                    detail: json!({"removed_ms":[1000,2000],"split_clip":"c9"})
                        .as_object()
                        .unwrap()
                        .clone(),
                },
                OpEffect {
                    track: None,
                    detail: json!({"lowered": lowered_step})
                        .as_object()
                        .unwrap()
                        .clone(),
                },
            ],
        ),
        mk(
            "op_000002",
            "edit.trim",
            json!({"clip":"c9","src_out_ms":3000}),
            vec![],
        ),
    ];
    let blockers = rebase::rebase_blockers(&ops, 0);
    assert_eq!(
        blockers.len(),
        1,
        "the trim on c9 blocks rebasing the lowered op"
    );
    assert_eq!(blockers[0].op_id, "op_000002");
    assert!(blockers[0].via_ids.contains(&"c9".to_string()));
    assert!(!rebase::can_rebase_out(&ops, 0));
}

// ---------------------------------------------------------------------------
// the project-path contract — relative project dir must canonicalize at the create boundary, so
// project-internal paths (exports/, receipts/) never double up or resolve
// against the server cwd. Mirrors open()'s long-standing canonicalize.
// ---------------------------------------------------------------------------

/// A RELATIVE parent dir passed to create() must yield an ABSOLUTE store.dir,
/// and every derived project path (receipts/, proxies/) must live UNDER that
/// absolute root exactly once (no doubling). This is the root cause behind
/// render.final's default export landing at <proj>/<relative-proj>/exports and
/// verify.judge failing to find <proj>/receipts/<id>.output.perception.json.
#[test]
fn create_canonicalizes_relative_dir_no_path_doubling() {
    // cd into a tempdir so a relative "parent" is well-defined and isolated.
    let tmp = tempfile::tempdir().unwrap();
    let prev_cwd = std::env::current_dir().unwrap();
    std::env::set_current_dir(tmp.path()).unwrap();
    // Run inside a closure so we always restore cwd, even on assert failure.
    let result = std::panic::catch_unwind(|| {
        std::fs::create_dir_all("nested/sub").unwrap();
        // Relative parent — the exact regression shape (".scratch/nested/...").
        let store = ProjectStore::create(std::path::Path::new("nested/sub"), "demo", None)
            .expect("create with relative dir");
        // 1) The stored dir is absolute (the canonicalize at the boundary).
        assert!(
            store.dir.is_absolute(),
            "store.dir must be canonicalized to absolute, got {:?}",
            store.dir
        );
        // 2) Derived subdirs sit directly under it — no relative-segment doubling.
        let receipts = store.receipts_dir();
        let proxies = store.proxies_dir();
        assert!(receipts.is_absolute() && receipts.starts_with(&store.dir));
        assert!(proxies.is_absolute() && proxies.starts_with(&store.dir));
        // 3) The simulated default export path resolves UNDER the project once.
        //    (This is what fence_output_path joins; doubling showed up as a
        //    second copy of the relative project segment in the path.)
        let export = store.dir.join("exports/render_001.mp4");
        assert!(
            export.starts_with(&store.dir),
            "default export must be inside the project dir"
        );
        assert_eq!(
            export
                .components()
                .filter(|c| c.as_os_str() == "demo.cutproj")
                .count(),
            1,
            "the project dir segment must appear exactly once (no doubling): {:?}",
            export
        );
        // 4) The simulated judge perception path resolves under the project too.
        let pcept = store
            .receipts_dir()
            .join("render_001.output.perception.json");
        assert!(pcept.starts_with(&store.dir));
        // 5) The on-disk dirs created during create() match the canonical root.
        assert!(store.dir.join("receipts").is_dir());
        assert!(store.dir.join("ops.jsonl").is_file());
    });
    std::env::set_current_dir(prev_cwd).unwrap();
    result.unwrap();
}

/// Absolute-dir behavior is unchanged: store.dir equals the canonicalized
/// absolute path that was passed in. (Regression guard so the project-path contract fix can
/// never alter the already-correct absolute path.)
#[test]
fn create_absolute_dir_behavior_unchanged() {
    let tmp = tempfile::tempdir().unwrap();
    let store = ProjectStore::create(tmp.path(), "demo", None).expect("create with absolute dir");
    // canonicalize() of an already-absolute, existing dir is a no-op modulo
    // symlink resolution; the parent tempdir canonicalizes to compare cleanly.
    let expected = tmp.path().canonicalize().unwrap().join("demo.cutproj");
    assert_eq!(store.dir, expected);
    assert!(store.dir.is_absolute());
}

/// edit.speed through the commit path: the retimed clip occupies half the
/// timeline, LATER clips ripple left automatically, replay reproduces it
/// byte-identically, and edit.restore undoes it cleanly. This is the
/// "receipts stay correct for free" payoff — EDL durations/positions come out
/// right with no special-casing because the source↔timeline mapping is
/// centralized and the clip's source range is untouched.
#[test]
fn speed_retime_ripples_and_roundtrips() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = ProjectStore::create(dir.path(), "spd", None).unwrap();
    s.record_import(None, asset(), actor(), None).unwrap();
    // Two back-to-back 2s clips on v1: [0,2000) then [2000,4000) source.
    s.apply(
        "edit.insert",
        json!({"asset":"a1","track":"v1","at_ms":0,"src_range_ms":[0,2000]}),
        actor(),
        None,
    )
    .unwrap();
    s.apply(
        "edit.insert",
        json!({"asset":"a1","track":"v1","at_ms":2000,"src_range_ms":[2000,4000]}),
        actor(),
        None,
    )
    .unwrap();
    assert_eq!(
        cut_core::edl_from_project(&s.project).duration_ms,
        4000,
        "two 2s clips = 4000ms"
    );
    let v1 = s.project.track("v1").unwrap();
    let c1 = v1.clips[0].id().unwrap().to_string();
    let c2 = v1.clips[1].id().unwrap().to_string();
    let before = s.project.clone();

    // 2× the FIRST clip: it now occupies 1000ms; the second clip ripples to
    // start at 1000ms; the composition shrinks to 3000ms.
    let rec = s
        .apply(
            "edit.speed",
            json!({"clip": c1, "factor": 2.0}),
            actor(),
            Some("tighten the intro".into()),
        )
        .unwrap();
    assert_eq!(
        rec.effects[0]
            .detail
            .get("new_timeline_duration_ms")
            .and_then(|v| v.as_u64()),
        Some(1000)
    );
    let edl = cut_core::edl_from_project(&s.project);
    assert_eq!(
        edl.duration_ms, 3000,
        "2× first clip shrinks the comp by 1000ms"
    );
    let seg1 = edl
        .segments
        .iter()
        .find(|sg| sg.clip_id.as_deref() == Some(&c1))
        .unwrap();
    let seg2 = edl
        .segments
        .iter()
        .find(|sg| sg.clip_id.as_deref() == Some(&c2))
        .unwrap();
    assert_eq!(
        (seg1.timeline_in_ms, seg1.timeline_out_ms),
        (0, 1000),
        "retimed clip occupies half"
    );
    assert_eq!(
        seg2.timeline_in_ms, 1000,
        "second clip rippled left to 1000ms"
    );
    assert_eq!(
        seg1.speed, 2.0,
        "EDL carries the clip speed for the renderer"
    );
    // Source range UNTOUCHED — speed remaps onto the timeline, it does not re-trim.
    assert_eq!((seg1.src_in_ms, seg1.src_out_ms), (Some(0), Some(2000)));

    // Replay determinism: rebuild from the log == live state (the gate).
    let ops = s.log.read_all().unwrap();
    assert_eq!(
        rebuild_from_log(&ops).unwrap(),
        s.project,
        "speed op must replay byte-identical"
    );

    // Undo via edit.restore returns to the exact pre-speed timeline.
    s.apply("edit.restore", json!({"op_id": rec.op_id}), actor(), None)
        .unwrap();
    assert_eq!(
        s.project.tracks, before.tracks,
        "restore undoes the speed op"
    );
    assert_eq!(cut_core::edl_from_project(&s.project).duration_ms, 4000);

    // factor 1.0 CLEARS the retime: serde-skipped, so the clip serializes
    // exactly as a never-sped clip would (byte-identical, no speed key).
    s.apply(
        "edit.speed",
        json!({"clip": c1, "factor": 2.0}),
        actor(),
        None,
    )
    .unwrap();
    s.apply(
        "edit.speed",
        json!({"clip": c1, "factor": 1.0}),
        actor(),
        None,
    )
    .unwrap();
    let cleared = cut_core::edl_from_project(&s.project);
    assert_eq!(
        cleared.duration_ms, 4000,
        "factor 1.0 restores normal duration"
    );
    assert!(
        !serde_json::to_string(&s.project).unwrap().contains("speed"),
        "cleared speed must be serde-skipped (byte-identical to unsped)"
    );
}

/// edit.speed refuses a non-media target (a caption clip) at the core layer —
/// the dispatch range/pitch gates are exercised in the server integration test;
/// this proves the core type-guard (gaps/captions have no source to stretch).
#[test]
fn speed_refuses_non_media_clip() {
    use cut_core::{CaptionClip, Clip, Track, TrackKind};
    let dir = tempfile::tempdir().unwrap();
    let mut s = ProjectStore::create(dir.path(), "spd2", None).unwrap();
    s.project.tracks.push(Track {
        id: "cap1".into(),
        kind: TrackKind::Caption,
        clips: vec![Clip::Caption(CaptionClip {
            id: "s1".into(),
            text: "hi".into(),
            style_ref: None,
            range_ms: [0, 1000],
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
    let err = s
        .apply(
            "edit.speed",
            json!({"clip":"s1","factor":2.0}),
            actor(),
            None,
        )
        .unwrap_err();
    assert_eq!(
        err.code, "invalid_args",
        "caption clip must reject: {err:?}"
    );
}

/// The recompute-by-replay disk win (the reason for the whole refactor): a
/// mutating op carries NO per-op timeline snapshot, so ops.jsonl grows O(N) not
/// O(N²). Proven by: (a) no op record carries a snapshot inverse, and (b) the
/// Nth insert's serialized line is within a small constant of the first's — flat
/// in clip count, where the old full-snapshot model made it grow O(N) per op.
#[test]
fn recompute_keeps_oplog_linear() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = ProjectStore::create(dir.path(), "disk", None).unwrap();
    s.record_import(None, asset(), actor(), None).unwrap();
    // 60 inserts → 60 clips. Under the old model op #60 carried a 60-clip
    // snapshot inverse (~O(N) bytes/op → O(N²) total ops.jsonl).
    for k in 0..60u64 {
        s.apply(
            "edit.insert",
            json!({"asset":"a1","track":"v1","at_ms": k * 100,"src_range_ms":[0,100]}),
            actor(),
            None,
        )
        .unwrap();
    }
    let raw = std::fs::read_to_string(s.dir.join("ops.jsonl")).unwrap();
    let lines: Vec<&str> = raw.lines().filter(|l| !l.trim().is_empty()).collect();

    // (a) NO op carries a timeline snapshot (the edit._set_timeline signature).
    for l in &lines {
        assert!(
            !l.contains("edit._set_timeline"),
            "no op may carry a full-timeline snapshot under recompute: {}",
            &l[..l.len().min(120)]
        );
    }
    // (b) Insert op lines are flat in N: the LAST insert is within a small
    // constant of the FIRST (only at_ms digits differ). Under the old snapshot
    // model the last would carry a 60-clip timeline and dwarf the first.
    let inserts: Vec<usize> = lines
        .iter()
        .filter(|l| l.contains("\"edit.insert\""))
        .map(|l| l.len())
        .collect();
    assert_eq!(inserts.len(), 60);
    let (first, last) = (inserts[0], *inserts.last().unwrap());
    assert!(
        last <= first + 64,
        "insert op line grew with clip count (first {first}B, last {last}B) — a snapshot leaked back in"
    );

    // And the whole thing still replays to the live state (undo trail intact).
    assert_eq!(
        rebuild_from_log(
            &lines
                .iter()
                .map(|l| serde_json::from_str(l).unwrap())
                .collect::<Vec<_>>()
        )
        .unwrap(),
        s.project
    );
}

/// Recompute restore both directions (the undo-correctness gate for the
/// snapshot-free model — a wrong recompute = silent data loss): a tip restore
/// UNDOES the last edit by recomputing its pre-op timeline from the log; a tip
/// restore of THAT undo op REDOES it (recomputes the pre-undo timeline). Built
/// on a non-trivial timeline so the recompute reconstructs real structure.
/// Replay reproduces every step.
#[test]
fn recompute_tip_restore_toggles_undo_redo() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = ProjectStore::create(dir.path(), "toggle", None).unwrap();
    s.record_import(None, asset(), actor(), None).unwrap();
    s.apply(
        "edit.insert",
        json!({"asset":"a1","track":"v1","at_ms":0,"src_range_ms":[0,5000]}),
        actor(),
        None,
    )
    .unwrap();
    s.apply(
        "edit.split",
        json!({"track":"v1","at_ms":1000}),
        actor(),
        None,
    )
    .unwrap();
    let before_split = s.project.tracks.clone(); // 2 clips
    let split = s
        .apply(
            "edit.split",
            json!({"track":"v1","at_ms":3000}),
            actor(),
            None,
        )
        .unwrap();
    let after_split = s.project.tracks.clone(); // 3 clips
    assert_ne!(before_split, after_split);

    // UNDO: restore the split (the tip) → recompute the pre-split timeline.
    let undo = s
        .apply("edit.restore", json!({"op_id": split.op_id}), actor(), None)
        .unwrap();
    assert_eq!(
        s.project.tracks, before_split,
        "tip restore recomputes the pre-split timeline"
    );

    // REDO: restore the undo op (now the tip) → recompute the pre-undo timeline,
    // which is the split state. (A restore op is itself a timeline op, so
    // restoring it toggles back — same semantics the snapshot model had.)
    s.apply("edit.restore", json!({"op_id": undo.op_id}), actor(), None)
        .unwrap();
    assert_eq!(
        s.project.tracks, after_split,
        "restoring the undo op redoes the split"
    );

    // Every step replays to the live state (append-only log, undo trail intact).
    assert_eq!(
        rebuild_from_log(&s.log.read_all().unwrap()).unwrap(),
        s.project
    );
}

/// review comments add/list/resolve round-trip, replay
/// byte-identical, are NOT timeline ops, and SURVIVE a timeline undo (they are
/// review metadata, not the edit — a tip-restore of an edit must not touch them).
#[test]
fn comments_roundtrip_and_survive_timeline_undo() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = ProjectStore::create(dir.path(), "cm", None).unwrap();
    s.record_import(None, asset(), actor(), None).unwrap();
    s.apply(
        "edit.insert",
        json!({"asset":"a1","track":"v1","at_ms":0,"src_range_ms":[0,5000]}),
        actor(),
        None,
    )
    .unwrap();

    // Add a point comment + a range comment.
    let (cm1, _) = s
        .add_comment(4300, None, "remove the um here", "client", actor(), None)
        .unwrap();
    let (cm2, _) = s
        .add_comment(
            1000,
            Some(2000),
            "this stretch is slow",
            "client",
            actor(),
            None,
        )
        .unwrap();
    assert_eq!((cm1.id.as_str(), cm1.status.as_str()), ("cm1", "open"));
    assert_eq!((cm2.id.as_str(), cm2.end_ms), ("cm2", Some(2000)));
    assert_eq!(s.project.comments.len(), 2);

    // Resolve one → addressed; the op is metadata (no inverse, not a timeline op).
    let (cm1b, rec) = s
        .resolve_comment("cm1", "addressed", actor(), None)
        .unwrap();
    assert_eq!(cm1b.status, "addressed");
    assert!(
        !rec.mutates_timeline(),
        "comment.resolve must not be a timeline op"
    );
    assert!(rec.inverse.is_none(), "comment ops carry no inverse");

    // Replay byte-identical.
    let ops = s.log.read_all().unwrap();
    assert_eq!(
        rebuild_from_log(&ops).unwrap(),
        s.project,
        "comments must replay byte-identical"
    );

    // A tip-restore undoes the latest TIMELINE op (the insert — the comments
    // after it are NOT timeline ops, so the insert is still the tip) and leaves
    // the comments untouched.
    let insert_op = ops
        .iter()
        .find(|o| o.verb == "edit.insert")
        .unwrap()
        .op_id
        .clone();
    s.apply("edit.restore", json!({"op_id": insert_op}), actor(), None)
        .unwrap();
    assert!(
        s.project.tracks.iter().all(|t| t.clips.is_empty()),
        "the insert was undone"
    );
    assert_eq!(
        s.project.comments.len(),
        2,
        "comments survive a timeline undo (they're metadata)"
    );
    assert_eq!(
        rebuild_from_log(&s.log.read_all().unwrap()).unwrap(),
        s.project
    );
}

#[test]
fn comment_anchor_resolves_after_upstream_ripple() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = ProjectStore::create(dir.path(), "cm-anchor", None).unwrap();
    s.record_import(None, asset(), actor(), None).unwrap();
    s.apply(
        "edit.insert",
        json!({"asset":"a1","track":"v1","at_ms":0,"src_range_ms":[0,5000]}),
        actor(),
        None,
    )
    .unwrap();
    s.apply(
        "edit.insert",
        json!({"asset":"a1","track":"v1","at_ms":5000,"src_range_ms":[0,5000]}),
        actor(),
        None,
    )
    .unwrap();

    let (cm, _) = s
        .add_comment(6000, Some(7000), "tighten this", "client", actor(), None)
        .unwrap();
    let anchor = cm
        .anchor
        .as_ref()
        .expect("comment should anchor to the clip under the playhead");
    assert_eq!(anchor.track_id, "v1");
    assert_eq!(anchor.clip_id, "c2");
    assert_eq!(anchor.offset_ms, 1000);

    s.apply(
        "edit.ripple_delete",
        json!({"track":"v1","range_ms":[1000,2000]}),
        actor(),
        None,
    )
    .unwrap();

    let live = &s.project.comments[0];
    assert_eq!(
        live.at_ms, 6000,
        "stored absolute time remains replay-stable"
    );
    assert_eq!(
        s.project.resolve_comment_anchor_ms(live),
        Some(5000),
        "resolved anchor follows c2 after content before it is removed"
    );
    assert_eq!(
        rebuild_from_log(&s.log.read_all().unwrap()).unwrap(),
        s.project,
        "anchored comments still replay byte-identically"
    );
}

/// Replay-durability regression for derived asset metadata: media.import records
/// the asset with all derived fields None (enrichment — probe/proxy/transcript/
/// perception/filmstrip — is a later project.json CACHE write, not an op). A pure
/// log rebuild therefore drops the pointers, but the files persist on disk under
/// deterministic names. ProjectStore::open's reconcile pass must RE-POINT every
/// derived field whose file exists — and must NOT fabricate a pointer for a field
/// whose file is absent.
#[test]
fn rebuild_reconciles_derived_asset_pointers_from_disk() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = ProjectStore::create(dir.path(), "rec", None).unwrap();
    // Import like PRODUCTION: the import op carries an asset with ALL derived
    // fields None (the test `asset()` helper sets probe=Some, which would mask
    // the bug — so build a bare one here).
    let bare = |path: &str| Asset {
        path: path.into(),
        hash: "sha256:deadbeef".into(),
        probe: None,
        transcript: None,
        perception: None,
        proxy: None,
        filmstrip: None,
    };
    let (a1, _) = s
        .record_import(None, bare("/testdata/a1.mp4"), actor(), None)
        .unwrap();
    let (a2, _) = s
        .record_import(None, bare("/testdata/a2.mp4"), actor(), None)
        .unwrap();
    assert_eq!((a1.as_str(), a2.as_str()), ("a1", "a2"));

    // a1: lay down all the enrichment artifacts the chain would write (the exact
    // deterministic names reconcile expects). a2: NOTHING on disk.
    std::fs::create_dir_all(s.dir.join("proxies")).unwrap();
    std::fs::create_dir_all(s.dir.join("filmstrip")).unwrap();
    std::fs::create_dir_all(s.dir.join("receipts")).unwrap();
    std::fs::write(s.dir.join("proxies/a1.mp4"), b"x").unwrap();
    std::fs::write(s.dir.join("filmstrip/a1.jpg"), b"x").unwrap();
    std::fs::write(s.dir.join("receipts/a1.words.json"), b"[]").unwrap();
    std::fs::write(s.dir.join("receipts/a1.perception.json"), b"{}").unwrap();
    std::fs::write(
        s.dir.join("receipts/a1.probe.json"),
        br#"{"kind":"video","duration_ms":10000}"#,
    )
    .unwrap();

    // Delete the cache → open() rebuilds from the log, then reconciles from disk.
    std::fs::remove_file(s.dir.join("project.json")).unwrap();
    let reopened = ProjectStore::open(&s.dir).expect("reopen cold");

    // a1: every derived pointer re-pointed; probe recovered from its file.
    let a = reopened
        .project
        .assets
        .get("a1")
        .expect("a1 survives rebuild");
    assert_eq!(
        a.proxy.as_deref(),
        Some("proxies/a1.mp4"),
        "proxy re-pointed"
    );
    assert_eq!(
        a.filmstrip.as_deref(),
        Some("filmstrip/a1.jpg"),
        "filmstrip re-pointed"
    );
    assert_eq!(
        a.transcript.as_deref(),
        Some("receipts/a1.words.json"),
        "transcript re-pointed"
    );
    assert_eq!(
        a.perception.as_deref(),
        Some("receipts/a1.perception.json"),
        "perception re-pointed"
    );
    assert_eq!(
        a.probe
            .as_ref()
            .and_then(|p| p.get("duration_ms"))
            .and_then(|d| d.as_u64()),
        Some(10_000),
        "probe recovered from receipts/a1.probe.json"
    );

    // a2: no files on disk → reconcile must NOT fabricate pointers (stays None).
    let b = reopened
        .project
        .assets
        .get("a2")
        .expect("a2 survives rebuild");
    assert!(
        b.proxy.is_none()
            && b.filmstrip.is_none()
            && b.transcript.is_none()
            && b.perception.is_none()
            && b.probe.is_none(),
        "an asset with no on-disk artifacts must keep None derived fields, not a dangling pointer"
    );
}

/// REGRESSION (caught by scripts/verb-walkthrough.mjs: a
/// `project.format` op MUST replay. Before apply_record gained its arm, the op
/// fell through to the "verb is not a core verb" escape, so ANY project that
/// changed output format became unreplayable — breaking three live paths:
///   1. recompute-by-replay undo (edit.restore of a later op replays the prefix,
///      which now contains the format op),
///   2. cold rebuild_from_log (reopening the project rebuilds from the log),
///   3. rebase across the format op (rebuild_skipping replays every other op).
/// This pins all three.
#[test]
fn project_format_op_replays_and_survives_restore_and_rebase() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = ProjectStore::create(dir.path(), "fmt", None).expect("create");
    s.record_import(None, asset(), actor(), None)
        .expect("import");
    s.apply(
        "edit.insert",
        json!({"asset":"a1","track":"v1","at_ms":0}),
        actor(),
        None,
    )
    .expect("insert");

    // Change output format (resolution + fps) — the op that was unreplayable.
    s.set_format(
        Some(1280),
        Some(720),
        Some(24.0),
        actor(),
        Some("downscale for speed".into()),
    )
    .expect("set_format");
    assert_eq!(s.project.settings.width, 1280);
    assert_eq!(s.project.settings.height, 720);
    assert_eq!(s.project.settings.fps, 24.0);

    // A further edit AFTER the format op (so a tip-restore of it must replay the
    // prefix that now contains the format op — path #1).
    let rec = s
        .apply(
            "edit.transform",
            json!({"clip":"c1","scale":0.8}),
            actor(),
            None,
        )
        .expect("transform");

    // PATH #2 — the whole log (incl. the format op) replays to byte-identical state.
    let all = s.log.read_all().unwrap();
    let rebuilt = rebuild_from_log(&all).expect("project.format must replay (regression)");
    assert_eq!(rebuilt, s.project, "replay diverged after project.format");
    assert_eq!(rebuilt.settings.width, 1280);
    assert_eq!(rebuilt.settings.fps, 24.0);

    // PATH #3 — rebase: skip the dependency-free transform op (no later op references
    // anything it created) and replay the rest, which still includes the format op.
    // The replay loop must not choke on project.format. Run BEFORE the restore so
    // skipping it leaves no dangling op_id reference.
    let ops = s.log.read_all().unwrap();
    let transform_idx = ops.iter().position(|o| o.verb == "edit.transform").unwrap();
    let rebased = rebuild_skipping(&ops, transform_idx)
        .expect("rebuild_skipping must replay across project.format");
    assert_eq!(
        rebased.settings.width, 1280,
        "format survives a rebase that skips another op"
    );

    // PATH #1 — recompute-by-replay undo of the later edit, across the format op.
    s.apply(
        "edit.restore",
        json!({"op_id": rec.op_id}),
        actor(),
        Some("undo transform".into()),
    )
    .expect("restore must replay the prefix across project.format");
    // Format settings are untouched by the restore (it only undid the transform).
    assert_eq!(s.project.settings.width, 1280);
    assert_eq!(s.project.settings.fps, 24.0);
}

/// Gate — edit.speed_ramp is a SINGLE replay-safe op: it sets ONE clip field and
/// allocates NO ids (unlike a real split, which needs per-split PinnedIds), so a
/// rebuild from the op log reproduces the ramp and its remapped timeline length
/// byte-identically. Also proves the variable-speed timeline_duration_ms remap is
/// the same on the live path and on replay.
#[test]
fn speed_ramp_replays_deterministically() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = ProjectStore::create(dir.path(), "demo", None).expect("create");
    let (aid, _) = s
        .record_import(None, asset(), actor(), None)
        .expect("import");
    assert_eq!(aid, "a1");
    // One 10s clip on v1 (asset duration 10_000 ms → clip [0,10000)).
    s.apply(
        "edit.insert",
        json!({"asset":"a1","track":"v1","at_ms":0}),
        actor(),
        None,
    )
    .expect("insert");
    let clip_id = match &s
        .project
        .tracks
        .iter()
        .find(|t| t.id == "v1")
        .unwrap()
        .clips[0]
    {
        cut_core::Clip::Media(c) => c.id.clone(),
        _ => panic!("expected a media clip"),
    };
    // A speed ramp: normal → 4× at the middle → normal, over the 10s source.
    s.apply(
        "edit.speed_ramp",
        json!({
            "clip": clip_id,
            "points": [
                {"at_ms": 0, "factor": 1.0},
                {"at_ms": 5000, "factor": 4.0},
                {"at_ms": 10000, "factor": 1.0}
            ],
            "segments": 40
        }),
        actor(),
        Some("dramatic mid-clip speed ramp".into()),
    )
    .expect("speed_ramp");

    // The clip carries the ramp, and the timeline shortened (fast middle nets less).
    let live_clip = match &s
        .project
        .tracks
        .iter()
        .find(|t| t.id == "v1")
        .unwrap()
        .clips[0]
    {
        cut_core::Clip::Media(c) => c.clone(),
        _ => panic!(),
    };
    assert!(live_clip.has_speed_ramp(), "ramp must be set on the clip");
    let dur = cut_core::Clip::Media(live_clip).timeline_duration_ms();
    assert!(
        dur < 10_000,
        "the fast middle nets a shorter clip ({dur} ms)"
    );

    // Replay reproduces the exact project (struct + bytes) and is self-identical.
    let ops = s.log.read_all().unwrap();
    let r1 = rebuild_from_log(&ops).expect("replay 1");
    let r2 = rebuild_from_log(&ops).expect("replay 2");
    assert_eq!(r1, s.project, "replayed state != live state");
    assert_eq!(
        serde_json::to_string(&r1).unwrap(),
        serde_json::to_string(&s.project).unwrap(),
        "replayed project.json bytes differ from live"
    );
    assert_eq!(
        serde_json::to_string(&r1).unwrap(),
        serde_json::to_string(&r2).unwrap(),
        "two replays diverged"
    );

    // Clearing the ramp (points:[]) is also replay-safe and restores constant speed.
    s.apply(
        "edit.speed_ramp",
        json!({"clip": clip_id, "points": []}),
        actor(),
        None,
    )
    .expect("clear ramp");
    let cleared = match &s
        .project
        .tracks
        .iter()
        .find(|t| t.id == "v1")
        .unwrap()
        .clips[0]
    {
        cut_core::Clip::Media(c) => c.clone(),
        _ => panic!(),
    };
    assert!(!cleared.has_speed_ramp(), "ramp must be cleared");
    let ops2 = s.log.read_all().unwrap();
    assert_eq!(
        rebuild_from_log(&ops2).unwrap(),
        s.project,
        "cleared-ramp replay drift"
    );
}
