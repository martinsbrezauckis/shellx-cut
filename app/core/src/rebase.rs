//! rebase.rs — selective non-tip undo ("op rebase"): skip ONE older op from
//! the log without discarding the ops after it.
//!
//! ## Why this module exists (the blocker, restated)
//!
//! ShellX Cut's spine is an append-only op log replayed in full to rebuild
//! `project.json` (store::rebuild_from_log). `edit.restore{mode:"tip"}` is
//! TIP-ONLY: it recomputes the pre-target log prefix, which would roll the
//! whole timeline back and silently discard every later edit for a non-tip
//! target. The store guardrail refuses that and names the blockers.
//!
//! "Rebase out op N" is fundamentally different from "restore op N": it must
//! reproduce the timeline AS IF N NEVER HAPPENED, while KEEPING every later op
//! and its allocated ids stable. Two facts make a naive replay-skip corrupting:
//!
//!   1. Current records have no per-op inverse — undo is recompute-by-replay,
//!      so we must REPLAY the log with N's effects omitted.
//!   2. Clip / marker / track ids are POSITIONALLY allocated (`max(cN)+1`) and
//!      REFERENCED BY LATER OPS (`edit.gain{clip:c3}`, `edit.trim{clip:c5}`...).
//!      Drop the op that allocated `c3` and a naive re-replay renumbers every
//!      later allocation → every later id reference is dangling or aimed at the
//!      WRONG clip. Silent corruption, not an error.
//!
//! ## The two safety mechanisms this module provides
//!
//! - **Id-pinning replay** (`pinned_ids_from_effects`, consumed by
//!   store::apply_edit_verb). On replay, allocating verbs consume the id they
//!   RECORDED in their effects instead of re-deriving it positionally. This
//!   makes replay independent of allocation ORDER — a strictly stronger
//!   determinism guarantee than today (and the prerequisite that lets a
//!   skip-replay keep later ids stable). The LIVE path is unchanged: it still
//!   allocates positionally and records what it allocated.
//!
//! - **Dependency gate** (`op_outputs` / `op_inputs` / `can_rebase_out`). Before
//!   skipping N we scan every later op for a reference to any id N CREATED. If
//!   ANY later op depends on N, we REFUSE with a structured guardrail error that
//!   names the dependents — the same honest-refusal pattern the tip guardrail
//!   uses. Only PROVABLY-INDEPENDENT ops are ever rebased out. A missed
//!   reference class would be silent corruption, so the analysis is exhaustive
//!   over every recorded verb, including the lowered (transcript.*, captions.*,
//!   audio.add_music) ops whose real edits live in their `lowered` effects.
//!
//! ## Append-only invariant (the append-only operation-log contract / timeline/op-log contract)
//!
//! A rebase NEVER rewrites history. It computes the post-skip timeline, then
//! APPENDS a fresh op with its materialized recomputed timeline in an effect.
//! It is itself tip-undoable by recomputing its pre-op prefix. Replay applies
//! the recorded result WITHOUT re-running dependency analysis — determinism
//! over the analysis is not required, only over the recorded result.
//!
//! Dependencies: types.rs, ops.rs, edit.rs, error.rs, store.rs. Primary caller:
//! store::ProjectStore::rebase_out (the live verb path) + store::apply_record
//! (replay of a recorded rebase op).

use crate::error::{codes, CutError};
use crate::ops::{OpEffect, OpRecord};
use std::collections::BTreeSet;

/// The ids an op CREATED (its "outputs") and the ids it REFERENCED ("inputs"),
/// split by namespace. Clip ids are `cN`, marker ids `mN`, track ids `vN`/`aNt`
/// (and the fixed defaults v1/a1t). Asset ids (`aN`) are NOT tracked here:
/// media.import is non-undoable and assets are never rebased out, so an op
/// referencing an asset can never depend on a rebased op for it.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct IdSet {
    /// Clip ids (`cN`).
    pub clips: BTreeSet<String>,
    /// Marker ids (`mN`).
    pub markers: BTreeSet<String>,
    /// Track ids (`vN`, `aNt`, or an explicit id from edit.add_track).
    pub tracks: BTreeSet<String>,
}

impl IdSet {
    /// True when this set shares at least one id (in any namespace) with `other`
    /// — i.e. some op consumed something this op produced.
    pub fn intersects(&self, other: &IdSet) -> bool {
        !self.clips.is_disjoint(&other.clips)
            || !self.markers.is_disjoint(&other.markers)
            || !self.tracks.is_disjoint(&other.tracks)
    }

    /// The shared ids, flattened to a sorted Vec for human-readable error text.
    pub fn shared_with(&self, other: &IdSet) -> Vec<String> {
        let mut v: Vec<String> = self
            .clips
            .intersection(&other.clips)
            .chain(self.markers.intersection(&other.markers))
            .chain(self.tracks.intersection(&other.tracks))
            .cloned()
            .collect();
        v.sort();
        v
    }
}

/// Pull a string id out of an op-effect detail key (e.g. `"added_clip"`).
fn eff_str<'a>(e: &'a OpEffect, key: &str) -> Option<&'a str> {
    e.detail.get(key).and_then(|v| v.as_str())
}

/// Walk a verb+args+effects triple, accumulating the clip ids it ALLOCATED.
/// Lives as a free fn so both the top-level op walk and the lowered-step walk
/// share one source of truth for "what clip ids did this primitive mint".
fn collect_clip_outputs(verb: &str, effects: &[OpEffect], out: &mut BTreeSet<String>) {
    match verb {
        // edit.split → effect {split_at_ms, left, right}; the RIGHT half is the
        // freshly allocated clip (the left half keeps the original id).
        "edit.split" => {
            for e in effects {
                if let Some(r) = eff_str(e, "right") {
                    out.insert(r.to_string());
                }
            }
        }
        // edit.insert → effect {added_clip, ...}; the splice that placed it may
        // ALSO have minted a `split_clip` id (the right half of a clip the
        // insert cut through). Both are outputs.
        "edit.insert" => {
            for e in effects {
                if let Some(c) = eff_str(e, "added_clip") {
                    out.insert(c.to_string());
                }
                if let Some(c) = eff_str(e, "split_clip") {
                    out.insert(c.to_string());
                }
            }
        }
        // edit.move → the clip keeps its id (no new clip), but the destination
        // splice may have minted a `split_clip` id (right half of a clip the
        // move landed inside). That split clip is a NEW output.
        "edit.move" => {
            for e in effects {
                if let Some(c) = eff_str(e, "split_clip") {
                    out.insert(c.to_string());
                }
            }
        }
        // edit.ripple_delete → effect {removed_ms, clips_removed, ripple, and
        // (new) split_clip when the range edge cut a clip whose BOTH halves
        // survive}. The split half is a fresh output; clips_removed are gone,
        // not outputs.
        "edit.ripple_delete" => {
            for e in effects {
                if let Some(c) = eff_str(e, "split_clip") {
                    out.insert(c.to_string());
                }
            }
        }
        // edit.duplicate / edit.nest both mint a replacement/clone clip recorded
        // as `added_clip`; dependency checks need to see that id so later edits
        // against the new clip produce a precise blocker instead of falling
        // through to a generic skip-replay failure.
        "edit.duplicate" | "edit.nest" => {
            for e in effects {
                if let Some(c) = eff_str(e, "added_clip") {
                    out.insert(c.to_string());
                }
            }
        }
        _ => {}
    }
}

/// The ids an op PRODUCES — clips/markers/tracks it allocated. Pure function of
/// the recorded verb + effects (never the live project), so it is stable under
/// replay. For lowered higher-layer ops (transcript.*, captions.generate,
/// audio.add_music) the real primitives live in the `lowered` effects entry —
/// we walk each lowered step's verb against the SAME op effects (the lowered
/// steps' allocations are recorded in the op's top-level effects, since
/// apply_lowered flattens every step's effects onto the record).
pub fn op_outputs(op: &OpRecord) -> IdSet {
    let mut out = IdSet::default();

    // Clip outputs — from this op's own verb AND from any lowered steps.
    collect_clip_outputs(&op.verb, &op.effects, &mut out.clips);
    if let Some(lowered) = op.effects.iter().find_map(|e| e.detail.get("lowered")) {
        if let Ok(steps) = serde_json::from_value::<Vec<crate::ops::InverseOp>>(lowered.clone()) {
            for s in &steps {
                // The lowered step's allocations were flattened onto the op's
                // top-level effects by apply_lowered, so we walk the SAME
                // effects under each step verb. (A lowered insert/split records
                // added_clip/right on the op exactly like a direct one.)
                collect_clip_outputs(&s.verb, &op.effects, &mut out.clips);
            }
        }
    }

    // Marker output — edit.add_marker effect {added_marker:{id,...}}.
    for e in &op.effects {
        if let Some(m) = e
            .detail
            .get("added_marker")
            .and_then(|v| v.get("id"))
            .and_then(|v| v.as_str())
        {
            out.markers.insert(m.to_string());
        }
        // Track output — edit.add_track effect {added_track, kind}.
        if let Some(t) = eff_str(e, "added_track") {
            out.tracks.insert(t.to_string());
        }
    }

    out
}

/// Pull a clip-id arg out of an args object (covers the `clip` arg used by
/// trim/gain/transform/crop/fade/captions.set_range, and `clips` arrays where a
/// verb names several).
fn args_clip_refs(args: &serde_json::Value, out: &mut BTreeSet<String>) {
    if let Some(c) = args.get("clip").and_then(|v| v.as_str()) {
        out.insert(c.to_string());
    }
    if let Some(arr) = args.get("clips").and_then(|v| v.as_array()) {
        for c in arr.iter().filter_map(|v| v.as_str()) {
            out.insert(c.to_string());
        }
    }
}

/// Pull a marker-id arg out of an args object (move_marker/remove_marker `id`).
fn args_marker_refs(verb: &str, args: &serde_json::Value, out: &mut BTreeSet<String>) {
    if matches!(verb, "edit.move_marker" | "edit.remove_marker") {
        if let Some(m) = args.get("id").and_then(|v| v.as_str()) {
            out.insert(m.to_string());
        }
    }
}

/// Pull track-id args out of an args object (`track`, `to_track`, `music_track`,
/// `against_track`). Default tracks (v1/a1t) are real references too, but they
/// are never rebased out (add_track can't allocate them), so naming them costs
/// nothing and keeps the analysis total.
fn args_track_refs(args: &serde_json::Value, out: &mut BTreeSet<String>) {
    for key in ["track", "to_track", "music_track", "against_track"] {
        if let Some(t) = args.get(key).and_then(|v| v.as_str()) {
            out.insert(t.to_string());
        }
    }
}

/// Walk one verb+args, accumulating the clip/marker/track ids it REFERENCES.
/// Shared by the top-level op walk and the lowered-step walk.
fn collect_inputs(verb: &str, args: &serde_json::Value, out: &mut IdSet) {
    args_clip_refs(args, &mut out.clips);
    args_marker_refs(verb, args, &mut out.markers);
    args_track_refs(args, &mut out.tracks);
}

/// Clip references some verbs record in EFFECTS rather than args, because they
/// bind their operands POSITIONALLY (by timeline position) at apply time, not
/// by clip id in the args. `edit.crossfade{track,at_ms,duration_ms}` names its
/// two neighbour clips as `left_clip`/`right_clip` in its effect — those are
/// genuine inputs the rebase dependency gate MUST see. Without this, a crossfade
/// that consumes a clip an EARLIER op created (e.g. the right-half clip a split
/// minted at that boundary) escapes the gate, so rebasing the split out is
/// wrongly allowed and the orphaned crossfade leaks a misleading replay error
/// instead of the honest guardrail refusal. Conservative direction
/// (op_inputs doc): include MORE references, never fewer.
fn collect_effect_inputs(verb: &str, effects: &[OpEffect], out: &mut BTreeSet<String>) {
    if verb == "edit.crossfade" {
        for e in effects {
            if let Some(c) = eff_str(e, "left_clip") {
                out.insert(c.to_string());
            }
            if let Some(c) = eff_str(e, "right_clip") {
                out.insert(c.to_string());
            }
        }
    }
}

/// The ids an op REFERENCES — clips/markers/tracks it reads from its args. Pure
/// function of the recorded verb + args. For lowered higher-layer ops we ALSO
/// walk each lowered step's verb+args (a transcript.cut_words lowered to an
/// `edit.ripple_delete{track}` references that track; a lowered
/// `edit.gain{clip}` references that clip).
///
/// NOTE the conservative direction: when in doubt we include MORE references,
/// never fewer. An over-inclusive input set can only make `can_rebase_out`
/// REFUSE a rebase that might actually be safe — the safe-side failure. An
/// under-inclusive set would let a corrupting rebase through, which is the one
/// outcome the product forbids ("a half-working rebase is worse than none").
pub fn op_inputs(op: &OpRecord) -> IdSet {
    let mut out = IdSet::default();
    collect_inputs(&op.verb, &op.args, &mut out);
    collect_effect_inputs(&op.verb, &op.effects, &mut out.clips);
    if let Some(lowered) = op.effects.iter().find_map(|e| e.detail.get("lowered")) {
        if let Ok(steps) = serde_json::from_value::<Vec<crate::ops::InverseOp>>(lowered.clone()) {
            for s in &steps {
                collect_inputs(&s.verb, &s.args, &mut out);
                collect_effect_inputs(&s.verb, &op.effects, &mut out.clips);
            }
        }
    }
    out
}

/// A later op that DEPENDS on the op we want to rebase out, with the shared ids
/// that create the dependency — the payload of an honest refusal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dependent {
    /// The dependent op's id.
    pub op_id: String,
    /// The dependent op's verb (for the human-readable refusal).
    pub verb: String,
    /// The ids the dependent op consumes that the target op produced.
    pub via_ids: Vec<String>,
}

/// Can op at index `n` be rebased out of `ops` without corrupting later ops?
/// Returns the list of later ops that DEPEND on n's outputs (empty ⇒ safe to
/// rebase). A dependency is "a later op references an id that n created."
///
/// `ops` must be the FULL log in order; `n` is the 0-based index of the target.
/// Non-timeline ops (for example project.create, project.checkpoint, and
/// media.import) are neither rebase targets nor dependents — but
/// we still scan their args for completeness (they carry no clip/marker/track
/// references, so they never register as dependents).
pub fn rebase_blockers(ops: &[OpRecord], n: usize) -> Vec<Dependent> {
    let outputs = op_outputs(&ops[n]);
    // A target that produced NOTHING (e.g. an old edit.gain, edit.trim,
    // edit.move with no split) can never be depended upon by id — it is always
    // independent, so the loop below naturally returns empty.
    let mut blockers = Vec::new();
    for later in &ops[n + 1..] {
        let inputs = op_inputs(later);
        if outputs.intersects(&inputs) {
            blockers.push(Dependent {
                op_id: later.op_id.clone(),
                verb: later.verb.clone(),
                via_ids: outputs.shared_with(&inputs),
            });
        }
    }
    blockers
}

/// True iff op `n` is provably independent of every later op (no dependents).
pub fn can_rebase_out(ops: &[OpRecord], n: usize) -> bool {
    rebase_blockers(ops, n).is_empty()
}

/// Build the structured guardrail error returned when a rebase target HAS
/// dependents. Names every blocking op + the ids that bind them, and points at
/// the existing escape hatches (project.revert / edit.restore) — the same
/// honest-refusal contract that the tip guardrail already honors.
pub fn rebase_refusal(target_op_id: &str, blockers: &[Dependent]) -> CutError {
    let names: Vec<String> = blockers
        .iter()
        .map(|d| format!("{} ({} via {})", d.op_id, d.verb, d.via_ids.join(",")))
        .collect();
    CutError::new(
        codes::GUARDRAIL,
        format!(
            "op '{target_op_id}' cannot be rebased out — {} later op(s) depend on ids it created",
            blockers.len()
        ),
        format!(
            "rebasing out '{target_op_id}' would leave these later op(s) referencing ids that \
             would no longer exist (silent timeline corruption): {}",
            names.join("; ")
        ),
    )
    .with_suggested_action(
        "only ops whose created ids no LATER op references can be rebased out; reject the \
         dependent op(s) first, or use project.revert{to} to roll back to a point, or \
         edit.restore the tip op",
    )
}

/// Wrap a raw skip-replay failure into an honest guardrail refusal.
/// Reached only when the dependency gate (`rebase_blockers`) missed a real
/// dependency and `rebuild_skipping` then failed re-running the orphaned op.
/// Rather than leak that op's own error (e.g. "no cut between two clips at
/// 4000ms") — which names neither the rebase target nor the true blocker — we
/// report that the rebase could not be reproduced and point at the escape
/// hatches, never committing a half-rebased timeline.
pub fn rebase_unreproducible(target_op_id: &str, cause: &CutError) -> CutError {
    CutError::new(
        codes::GUARDRAIL,
        format!("op '{target_op_id}' cannot be rebased out — replaying the log without it failed"),
        format!(
            "a later op could not be reproduced once '{target_op_id}' was skipped (an \
             unanticipated dependency the gate did not catch); refusing rather than committing a \
             half-rebased timeline. underlying cause: {}",
            cause.message
        ),
    )
    .with_suggested_action(
        "reject the dependent op first, or use project.revert{to} to roll back to a checkpoint",
    )
}

/// Extract the ids an op RECORDED as allocated, keyed by the allocation ROLE
/// the replay path needs to pin. Returned as a small struct the edit-verb table
/// consults on the replay path so it consumes the recorded id instead of
/// re-deriving it positionally. The LIVE path never uses this (it allocates and
/// records); only replay (store::apply_record) and skip-replay
/// (store::rebuild_skipping) pin.
///
/// Fields are `Option` because not every op of a verb records every role
/// (an insert that did NOT split records `added_clip` but no `split_clip`), and
/// because OLD logs predate the `split_clip` effect — a missing pin falls back
/// to positional allocation, which in the no-skip case is byte-identical to the
/// recorded id (that is exactly why old logs still replay unchanged).
/// IMPORTANT — multi-allocation: a single LOWERED op (audio.add_music) can run
/// MANY allocating primitive steps (one edit.add_marker per beat → m1, m2, …).
/// So the marker pin is a QUEUE consumed IN ORDER across the lowered steps, not
/// a single value. Same for ripple range-edge split clips (one per track). The
/// Direct edit ops allocate added/split clips once, but an atomic Motion import
/// lowers several insert steps into one op. The clip roles therefore also keep
/// ordered queues while retaining their first-value fields for legacy callers.
///
/// Because the queue roles are consumed positionally, the replay caller passes
/// `&mut PinnedIds` and each allocating step POPS the next id. The live path
/// (apply_edit_verb with pinned=None) never touches this.
#[derive(Debug, Default, Clone)]
pub struct PinnedIds {
    /// edit.split right-half id (effect `right`). One per op.
    pub split_right: Option<String>,
    /// edit.insert added-clip id (effect `added_clip`). One per op.
    pub added_clip: Option<String>,
    /// All insert added-clip ids in lowered-step order.
    pub added_clips: Vec<String>,
    added_clip_cursor: usize,
    /// splice-split right-half id minted by insert/move (the FIRST `split_clip`
    /// effect — insert/move splice at most one clip, so one value suffices).
    pub split_clip: Option<String>,
    /// All insert splice-split ids in lowered-step order.
    pub split_clips: Vec<String>,
    split_clip_cursor: usize,
    /// edit.ripple_delete range-edge split ids — ALL `split_clip` effect values
    /// IN TRACK ORDER (a track=None ripple can split one clip per track).
    /// Consumed via `next_ripple_split` (positional).
    pub ripple_split_clips: Vec<String>,
    /// Read cursor into `ripple_split_clips`.
    ripple_cursor: usize,
    /// edit.add_track id (effect `added_track`). One per op.
    pub added_track: Option<String>,
    /// edit.add_marker ids (effect `added_marker.id`) — ALL of them IN ORDER (a
    /// lowered audio.add_music records one per beat). Consumed via
    /// `next_marker` (positional).
    pub added_markers: Vec<String>,
    /// Read cursor into `added_markers`.
    marker_cursor: usize,
}

impl PinnedIds {
    /// Read the recorded allocation ids out of an op's effects. Cheap, pure,
    /// total (absent keys ⇒ empty ⇒ the verb falls back to positional alloc).
    pub fn from_effects(effects: &[OpEffect]) -> Self {
        let mut p = PinnedIds::default();
        for e in effects {
            if let Some(r) = eff_str(e, "right") {
                p.split_right.get_or_insert_with(|| r.to_string());
            }
            if let Some(c) = eff_str(e, "added_clip") {
                p.added_clip.get_or_insert_with(|| c.to_string());
                p.added_clips.push(c.to_string());
            }
            if let Some(c) = eff_str(e, "split_clip") {
                // split_clip is recorded by BOTH insert/move (splice split, at
                // most one per op → first wins) AND ripple_delete (range-edge
                // split, one PER TRACK → collect ALL in track order). An op runs
                // exactly one of those verbs, so populating both shapes is safe.
                p.split_clip.get_or_insert_with(|| c.to_string());
                p.split_clips.push(c.to_string());
                p.ripple_split_clips.push(c.to_string());
            }
            if let Some(t) = eff_str(e, "added_track") {
                p.added_track.get_or_insert_with(|| t.to_string());
            }
            if let Some(m) = e
                .detail
                .get("added_marker")
                .and_then(|v| v.get("id"))
                .and_then(|v| v.as_str())
            {
                p.added_markers.push(m.to_string());
            }
        }
        p
    }

    /// Pop the next recorded marker id (None once the queue is exhausted ⇒ the
    /// step allocates positionally). Advances the cursor.
    pub fn next_marker(&mut self) -> Option<String> {
        let id = self.added_markers.get(self.marker_cursor).cloned();
        if id.is_some() {
            self.marker_cursor += 1;
        }
        id
    }

    pub fn next_added_clip(&mut self) -> Option<String> {
        if self.added_clips.is_empty() {
            return self.added_clip.clone();
        }
        let id = self.added_clips.get(self.added_clip_cursor).cloned();
        if id.is_some() {
            self.added_clip_cursor += 1;
        }
        id
    }

    pub fn next_split_clip(&mut self) -> Option<String> {
        if self.split_clips.is_empty() {
            return self.split_clip.clone();
        }
        let id = self.split_clips.get(self.split_clip_cursor).cloned();
        if id.is_some() {
            self.split_clip_cursor += 1;
        }
        id
    }

    /// Pop the next recorded ripple range-edge split id (None once exhausted).
    /// Advances the cursor.
    pub fn next_ripple_split(&mut self) -> Option<String> {
        let id = self.ripple_split_clips.get(self.ripple_cursor).cloned();
        if id.is_some() {
            self.ripple_cursor += 1;
        }
        id
    }

    /// True when no role is pinned — the op allocated nothing (or is an old log
    /// with no recorded ids). The verb table can then take the plain live path.
    pub fn is_empty(&self) -> bool {
        self.split_right.is_none()
            && self.added_clip.is_none()
            && self.added_clips.is_empty()
            && self.split_clip.is_none()
            && self.split_clips.is_empty()
            && self.ripple_split_clips.is_empty()
            && self.added_track.is_none()
            && self.added_markers.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ops::{Actor, ActorKind, OpRecord, OpStatus};
    use serde_json::json;

    /// Build a minimal OpRecord for analysis tests (only verb/args/effects
    /// matter to op_inputs/op_outputs).
    fn op(verb: &str, args: serde_json::Value, effects: Vec<OpEffect>) -> OpRecord {
        OpRecord {
            op_id: "op_test".into(),
            ts: "2026-06-11T00:00:00.000Z".into(),
            actor: Actor {
                kind: ActorKind::Agent,
                name: "t".into(),
                via: "test".into(),
                request: None,
            },
            verb: verb.into(),
            args,
            rationale: None,
            effects,
            inverse: None,
            status: OpStatus::Applied,
        }
    }

    fn fx(detail: serde_json::Value) -> OpEffect {
        crate::edit::fx(None, detail)
    }

    #[test]
    fn split_outputs_right_half_only() {
        let o = op(
            "edit.split",
            json!({"track":"v1","at_ms":4000}),
            vec![fx(json!({"split_at_ms":4000,"left":"c1","right":"c3"}))],
        );
        let out = op_outputs(&o);
        assert!(out.clips.contains("c3"));
        assert!(
            !out.clips.contains("c1"),
            "left half keeps its id — not an output"
        );
    }

    #[test]
    fn insert_outputs_added_and_split_clip() {
        let o = op(
            "edit.insert",
            json!({"asset":"a1","track":"v1","at_ms":1000}),
            vec![fx(json!({"added_clip":"c5","split_clip":"c6"}))],
        );
        let out = op_outputs(&o);
        assert!(out.clips.contains("c5"));
        assert!(
            out.clips.contains("c6"),
            "the splice-split right half is also an output"
        );
    }

    #[test]
    fn duplicate_and_nest_outputs_added_clip() {
        for verb in ["edit.duplicate", "edit.nest"] {
            let o = op(
                verb,
                json!({"clip":"c1"}),
                vec![fx(json!({"added_clip":"c7"}))],
            );
            let out = op_outputs(&o);
            assert!(
                out.clips.contains("c7"),
                "{verb} output should include added_clip"
            );
        }
    }

    #[test]
    fn trim_inputs_reference_clip() {
        let o = op("edit.trim", json!({"clip":"c3","src_out_ms":2000}), vec![]);
        let inp = op_inputs(&o);
        assert!(inp.clips.contains("c3"));
    }

    #[test]
    fn dependency_gate_blocks_when_later_op_uses_created_id() {
        // op0: split makes c3. op1: gain on c3 → c3 depends on op0.
        let ops = vec![
            op(
                "edit.split",
                json!({"track":"v1","at_ms":4000}),
                vec![fx(json!({"left":"c1","right":"c3"}))],
            ),
            op("edit.gain", json!({"clip":"c3","db":-3.0}), vec![]),
        ];
        let blockers = rebase_blockers(&ops, 0);
        assert_eq!(blockers.len(), 1);
        assert_eq!(blockers[0].verb, "edit.gain");
        assert_eq!(blockers[0].via_ids, vec!["c3".to_string()]);
        assert!(!can_rebase_out(&ops, 0));
    }

    #[test]
    fn crossfade_effect_clip_refs_register_as_inputs() {
        // op_inputs must see the neighbour clips a crossfade records in EFFECTS
        // (left_clip/right_clip), not just args — the args carry no clip id, so
        // the recorded effect references are authoritative.
        let o = op(
            "edit.crossfade",
            json!({"track":"v1","at_ms":4000,"duration_ms":500}),
            vec![fx(
                json!({"at_ms":4000,"left_clip":"c1","right_clip":"c3","xfade_ms":500}),
            )],
        );
        let inp = op_inputs(&o);
        assert!(inp.clips.contains("c1"), "left_clip is an input");
        assert!(inp.clips.contains("c3"), "right_clip is an input");
    }

    #[test]
    fn dependency_gate_blocks_crossfade_on_split_created_clip() {
        // Regression case: a split mints c3; a later crossfade consumes c3 as
        // its right neighbour. Rebasing the split out must be REFUSED (the
        // crossfade depends on c3) with the honest guardrail — before the fix the
        // gate was blind to the crossfade and the rebase leaked a replay error.
        let ops = vec![
            op(
                "edit.split",
                json!({"track":"v1","at_ms":4000}),
                vec![fx(json!({"left":"c1","right":"c3"}))],
            ),
            op(
                "edit.crossfade",
                json!({"track":"v1","at_ms":4000,"duration_ms":500}),
                vec![fx(
                    json!({"at_ms":4000,"left_clip":"c1","right_clip":"c3","xfade_ms":500}),
                )],
            ),
        ];
        let blockers = rebase_blockers(&ops, 0);
        assert_eq!(
            blockers.len(),
            1,
            "crossfade depends on the split-created clip"
        );
        assert_eq!(blockers[0].verb, "edit.crossfade");
        assert!(blockers[0].via_ids.contains(&"c3".to_string()));
        assert!(!can_rebase_out(&ops, 0));
    }

    #[test]
    fn dependency_gate_allows_independent_op() {
        // op0: gain on a1t (creates nothing). op1: trim c1 (unrelated). op0 is
        // independent — nothing it created is referenced (it created nothing).
        let ops = vec![
            op("edit.gain", json!({"track":"a1t","db":-3.0}), vec![]),
            op("edit.trim", json!({"clip":"c1","src_out_ms":2000}), vec![]),
        ];
        assert!(can_rebase_out(&ops, 0));
        assert!(rebase_blockers(&ops, 0).is_empty());
    }

    #[test]
    fn lowered_op_outputs_and_inputs_are_walked() {
        // A transcript.cut_words lowered to a ripple_delete on v1 that split a
        // clip (recorded split_clip c9). Its output set must include c9; its
        // input set must include the track it referenced.
        let o = op(
            "transcript.cut_words",
            json!({"asset":"a1"}),
            vec![
                fx(json!({"removed_ms":[1000,2000],"split_clip":"c9","ripple":true})),
                fx(
                    json!({"lowered":[{"verb":"edit.ripple_delete","args":{"track":"v1","range_ms":[1000,2000]}}]}),
                ),
            ],
        );
        let out = op_outputs(&o);
        assert!(
            out.clips.contains("c9"),
            "lowered ripple split clip is an output"
        );
        let inp = op_inputs(&o);
        assert!(
            inp.tracks.contains("v1"),
            "lowered step's track is an input"
        );
    }

    #[test]
    fn pinned_ids_extract_every_role() {
        let effects = vec![
            fx(json!({"right":"c3"})),
            fx(json!({"added_clip":"c5","split_clip":"c6"})),
            fx(json!({"added_track":"v2","kind":"video"})),
            fx(json!({"added_marker":{"id":"m4","at_ms":0,"label":"x"}})),
        ];
        let mut p = PinnedIds::from_effects(&effects);
        assert_eq!(p.split_right.as_deref(), Some("c3"));
        assert_eq!(p.added_clip.as_deref(), Some("c5"));
        assert_eq!(p.split_clip.as_deref(), Some("c6"));
        assert_eq!(p.added_track.as_deref(), Some("v2"));
        assert_eq!(p.next_marker().as_deref(), Some("m4"));
        assert_eq!(
            p.next_marker(),
            None,
            "queue exhausted after the one marker"
        );
        assert!(!p.is_empty());
    }

    #[test]
    fn pinned_ids_empty_for_non_allocating_op() {
        let effects = vec![fx(
            json!({"clip":"c1","old_src_ms":[0,3000],"new_src_ms":[0,2000]}),
        )];
        assert!(PinnedIds::from_effects(&effects).is_empty());
    }

    /// Regression: a lowered audio.add_music records MANY add_marker effects
    /// (one per beat). The marker pin MUST be a queue popped in order — pinning
    /// every beat to m1 corrupted replay (caught by the server add_music test).
    #[test]
    fn pinned_marker_queue_pops_in_order() {
        let effects = vec![
            fx(json!({"added_marker":{"id":"m1","at_ms":0,"label":"beat"}})),
            fx(json!({"added_marker":{"id":"m2","at_ms":500,"label":"beat"}})),
            fx(json!({"added_marker":{"id":"m3","at_ms":1000,"label":"beat"}})),
        ];
        let mut p = PinnedIds::from_effects(&effects);
        assert_eq!(p.next_marker().as_deref(), Some("m1"));
        assert_eq!(p.next_marker().as_deref(), Some("m2"));
        assert_eq!(p.next_marker().as_deref(), Some("m3"));
        assert_eq!(p.next_marker(), None, "exhausted ⇒ positional fallback");
    }

    /// Regression: a track=None ripple can split one clip per track; the split
    /// ids queue pops in track order.
    #[test]
    fn pinned_ripple_split_queue_pops_in_order() {
        let effects = vec![
            crate::edit::fx(
                Some("v1"),
                json!({"removed_ms":[1000,2000],"split_clip":"c5"}),
            ),
            crate::edit::fx(
                Some("a1t"),
                json!({"removed_ms":[1000,2000],"split_clip":"c6"}),
            ),
        ];
        let mut p = PinnedIds::from_effects(&effects);
        assert_eq!(p.next_ripple_split().as_deref(), Some("c5"));
        assert_eq!(p.next_ripple_split().as_deref(), Some("c6"));
        assert_eq!(p.next_ripple_split(), None);
    }
}
