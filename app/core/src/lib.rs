//! cut-core — ShellX Cut core crate (timeline/op-log contract).
//!
//! Role: the shared-types + timeline-logic crate every other crate depends
//! on. Owns the project model, the append-only op-log (source of truth),
//! EDL derivation, checkpoints/diff, edit primitives, and the universal
//! verb/error/receipt types. NO I/O beyond the project dir; NO ffmpeg; NO
//! network — those live in cut-media / cut-perception / server.
//! Primary callers: cut-media, cut-perception, server (cutd), tests.
//!
//! Module map:
//! - types     — Project/Track/Clip/Marker/CaptionStyle/Checkpoint (project.json)
//! - ops       — OpRecord/Actor/OpEffect/OpLog (ops.jsonl)
//! - edl       — Edl/EdlSegment + edl_from_project (render + checks input)
//! - edit      — the only timeline mutators (split/ripple_delete/trim/...)
//! - diff      — DiffSummary + diff(a,b) over checkpoints
//! - store     — ProjectStore: .cutproj dir lifecycle, replay, hashing
//! - receipt   — CheckResult + RenderReceipt ("done requires evidence")
//! - error     — CutError (actionable) + VerbResult (universal envelope)

pub mod auto_zoom;
pub mod beatsync;
pub mod detach;
pub mod diff;
pub mod edit;
pub mod edl;
pub mod error;
mod journal;
pub mod multicam;
mod mutation_request;
pub mod ops;
pub mod rebase;
pub mod receipt;
mod speed_ramp_timing;
pub mod split_edit;
pub mod store;
pub mod trim_edit;
pub mod types;
pub mod verb_contract;

// Re-export the shared contract types at crate root (build-contract: parallel
// agents import `cut_core::{Project, OpRecord, Receipt types, VerbResult, ...}`).
pub use detach::{find_safe_detach_audio_track, plan_detach_audio, DetachPlan, DetachReject};
pub use diff::{diff, DiffSummary, TrackTouch};
pub use edit::{FadeTarget, GainTarget};
pub use edl::{edl_from_project, Edl, EdlAdjustment, EdlSegment};
pub use error::{codes as error_codes, CutError, VerbResult, VerbWarning};
pub use mutation_request::MutationRequest;
pub use ops::{Actor, ActorKind, InverseOp, JournalRecovery, OpEffect, OpLog, OpRecord, OpStatus};
pub use rebase::{
    can_rebase_out, op_inputs, op_outputs, rebase_blockers, Dependent, IdSet, PinnedIds,
};
pub use receipt::{check_names, fix_action, CheckResult, FixAction, FixTarget, RenderReceipt};
pub use speed_ramp_timing::{
    timeline_duration_ms_for_frames, timeline_frame_count, timeline_sample_count_for_frames,
};
pub use split_edit::{plan_split_edit, SplitEditKind, SplitEditPlan, SplitEditReject};
pub use store::{
    apply_edit_verb, apply_record, hash_file, rebuild_from_log, timeline_snapshot,
    AtomicMediaInsertPlanResult, ProjectCacheHealth, ProjectOpenHealth, ProjectSnapshotHealth,
    ProjectStore,
};
pub use types::{
    default_speed, effect_specs, is_unit_speed, is_valid_chroma_color, is_valid_transition,
    speed_ramp_factor_at, speed_ramp_segments, src_off_to_tl, tl_off_to_src, transition_specs,
    Adjustment, AnimState, Asset, BrandKit, CaptionClip, CaptionStyle, CaptionStylePreset,
    Checkpoint, Clip, ClipAnimation, ClipCrop, ClipEffect, ClipEq, ClipFade, ClipFreeze, ClipGrade,
    ClipMask, ClipMatte, ClipStabilize, ClipTransform, ColorConfig, ColorSpace, Comment,
    CommentAnchor, CommentReviewSource, EffectParam, EffectSpec, EqBand, FadeKind, GainWindow,
    GapClip, GradePreset, GradeWindow, Keyframe, KfInterp, KfParam, KfPoint, Marker, MaskEffect,
    MaskShape, MaskTrackPoint, MatteBg, MatteMode, MatteModel, MatteQuality, MatteSeed, MediaClip,
    Nest, Project, ProjectSettings, RampSeg, ReviewFeedbackNote, Sequence, SmartBin, SpeedRamp,
    SpeedRampPoint, Track, TrackKind, TranscriptIgnore, TransitionSpec, WindowShape,
    DEFAULT_RAMP_SEGMENTS, DEFAULT_SEQUENCE_ID, MAX_RAMP_SEGMENTS, MIN_FRAMES_PER_SUBSEG,
    MIN_RAMP_SEGMENTS, PROJECT_SCHEMA, TRANSITIONS,
};
pub use verb_contract::{
    contract_for_verb, AgentChatCapability, AsyncJobType, Idempotency, MutationClass, ProjectState,
    Replayability, SideEffects, UiExposure, VerbContract, VerbFacet, VerbRisk,
};

#[cfg(test)]
mod tests {
    use super::*;

    /// Smoke: the timeline/op-log contract example timeline JSON round-trips through our types.
    #[test]
    fn spec_timeline_roundtrip() {
        let json = serde_json::json!({
            "schema": "shellx-cut/1",
            "settings": {"width":1920, "height":1080, "fps":30, "audio_rate":48000},
            "assets": {"a1": {"path":"/x.mp4", "hash":"sha256:abc"}},
            "tracks": [
                {"id":"v1","kind":"video","clips":[
                    {"id":"c1","asset":"a1","src_in_ms":0,"src_out_ms":5000,"effects":[],"gain_db":0.0}
                ]},
                {"id":"a1t","kind":"audio","clips":[{"kind":"gap","duration_ms":250}]},
                {"id":"cap1","kind":"caption","clips":[
                    {"id":"s1","text":"hello","style_ref":"brand1","range_ms":[0,1200]}
                ]}
            ],
            "markers": [{"id":"m1","at_ms":1234,"label":"intro"}],
            "caption_styles": {"brand1": {"font":"Inter","size":42,"color":"#fff","bg":"#000a","pos":"bottom"}},
            "checkpoints": []
        });
        let p: Project = serde_json::from_value(json).expect("spec example must parse");
        assert_eq!(p.tracks.len(), 3);
        assert!(matches!(p.tracks[0].clips[0], Clip::Media(_)));
        assert!(matches!(p.tracks[1].clips[0], Clip::Gap(_)));
        assert!(matches!(p.tracks[2].clips[0], Clip::Caption(_)));
        assert_eq!(p.duration_ms(), 5000);
        // And back out without loss of clip variants.
        let v = serde_json::to_value(&p).unwrap();
        let p2: Project = serde_json::from_value(v).unwrap();
        assert_eq!(p, p2);
    }

    /// Smoke: EDL derivation flattens cumulative positions + caption absolutes.
    #[test]
    fn edl_smoke() {
        let mut p = Project::new("t", ProjectSettings::default());
        p.track_mut("v1")
            .unwrap()
            .clips
            .push(Clip::Media(edit::make_media_clip("c1", "a1", 0, 3000)));
        p.track_mut("v1")
            .unwrap()
            .clips
            .push(Clip::Media(edit::make_media_clip("c2", "a1", 5000, 6000)));
        let edl = edl_from_project(&p);
        let segs: Vec<_> = edl.track_segments("v1").collect();
        assert_eq!(segs.len(), 2);
        assert_eq!(
            (segs[1].timeline_in_ms, segs[1].timeline_out_ms),
            (3000, 4000)
        );
        assert_eq!(edl.cut_points_ms(), vec![0, 3000, 4000]);
    }

    /// Regression (compositing flag: OVERLAY video-track
    /// boundaries are NOT editorial cut points — PiP compositing never cuts
    /// program audio, so cut_points_ms keeps audio tracks + the base video
    /// track only (matching the renderer's base/overlay split).
    #[test]
    fn cut_points_exclude_overlay_tracks() {
        let mut p = Project::new("t", ProjectSettings::default());
        p.track_mut("v1")
            .unwrap()
            .clips
            .push(Clip::Media(edit::make_media_clip("c1", "a1", 0, 3000)));
        p.track_mut("a1t")
            .unwrap()
            .clips
            .push(Clip::Media(edit::make_media_clip("ac1", "a1", 0, 3000)));
        // Overlay video track with boundaries (timeline 0..400) that exist
        // on no audio-bearing track.
        p.tracks.push(Track {
            id: "v2".into(),
            kind: TrackKind::Video,
            clips: vec![Clip::Media(edit::make_media_clip("o1", "a1", 700, 1100))],
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
        assert_eq!(edl.base_video_track(), Some("v1"));
        assert!(edl.is_audio_bearing_track("v1"));
        assert!(edl.is_audio_bearing_track("a1t"));
        assert!(
            !edl.is_audio_bearing_track("v2"),
            "overlay is not audio-bearing"
        );
        assert!(
            !edl.is_audio_bearing_track("nope"),
            "unknown track is not audio-bearing"
        );
        // 400 (overlay clip end) must NOT appear; v1/a1t boundaries remain.
        assert_eq!(edl.cut_points_ms(), vec![0, 3000]);
    }

    /// Smoke: op-log append + bounded cursor page round-trip on disk.
    #[test]
    fn oplog_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let log = OpLog::open(&dir.path().join("ops.jsonl")).unwrap();
        let op = OpRecord {
            op_id: OpRecord::format_id(0),
            ts: OpRecord::now_ts(),
            actor: Actor {
                kind: ActorKind::Agent,
                name: "claude".into(),
                via: "mcp".into(),
                request: None,
            },
            verb: "edit.split".into(),
            args: serde_json::json!({"track":"v1","at_ms":100}),
            rationale: None,
            effects: vec![],
            inverse: None,
            status: OpStatus::Applied,
        };
        log.append(&op).unwrap();
        assert_eq!(log.read_all().unwrap().len(), 1);
        assert_eq!(log.next_id().unwrap(), "op_000002");
        assert!(log
            .page_after(Some("op_000001"), 1, 1024)
            .unwrap()
            .ops
            .is_empty());
    }

    /// Caption-duration regression: caption clips carry absolute `range_ms`,
    /// so a caption track's timeline end is max(range end) — never the SUM of
    /// caption durations. The old sum inflated Project::duration_ms() (and
    /// with it edl.duration_ms) past the real composition end, failing the
    /// duration_matches_edl check on every captioned render.
    #[test]
    fn caption_track_duration_is_max_range_end_not_sum() {
        let mut p = Project::new("t", ProjectSettings::default());
        p.track_mut("v1")
            .unwrap()
            .clips
            .push(Clip::Media(edit::make_media_clip("c1", "a1", 0, 5000)));
        p.tracks.push(Track {
            id: "cap1".into(),
            kind: TrackKind::Caption,
            clips: vec![
                Clip::Caption(CaptionClip {
                    id: "s1".into(),
                    text: "a".into(),
                    style_ref: None,
                    range_ms: [0, 2000],
                }),
                Clip::Caption(CaptionClip {
                    id: "s2".into(),
                    text: "b".into(),
                    style_ref: None,
                    range_ms: [2500, 4800],
                }),
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
        // Sum of caption durations = 4300; max range end = 4800; media = 5000.
        assert_eq!(p.tracks.last().unwrap().duration_ms(), 4800);
        assert_eq!(p.duration_ms(), 5000);
        assert_eq!(edl_from_project(&p).duration_ms, 5000);
        // Captions covering MORE total duration than the media (overlap-free
        // but dense) must still not push the composition past the media end.
        p.tracks
            .last_mut()
            .unwrap()
            .clips
            .push(Clip::Caption(CaptionClip {
                id: "s3".into(),
                text: "c".into(),
                style_ref: None,
                range_ms: [100, 4900],
            }));
        assert_eq!(p.duration_ms(), 5000);
    }
}
