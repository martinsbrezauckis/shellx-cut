// lib/ops — op-classification shared across the Review rail + Timeline undo.
//
// SINGLE SOURCE OF TRUTH for "does this op mutate the timeline?" — a verbatim
// mirror of the engine's `cut_core::OpRecord::mutates_timeline()`
// (app/core/src/ops.rs). The engine uses a BLOCKLIST: EVERY op mutates the
// timeline (and is therefore undoable / rebasable) EXCEPT a small fixed set of
// structural / metadata verbs.
//
// Why this exists: the UI previously kept an allowlist
// (`TIMELINE_MUTATING`) that had to enumerate every mutating verb. New verbs
// (edit.grade, title.add, captions.kinetic, edit.speed, the scene/trim verbs,
// audio.add_music, transcript.assemble, captions.reflow/shift/set_range, …) were
// silently excluded, so the Undo bar pointed at the wrong (older) op — most
// visibly, `captions.kinetic { replace_static }` appends a semantic
// captions.kinetic op the old allowlist skipped. Mirroring the engine's
// blocklist makes the UI correct for ALL current and FUTURE verbs by default.
//
// Keep this list in lockstep with app/core/src/ops.rs::mutates_timeline().

/** Verbs that do NOT mutate the timeline (structural / metadata only). Mirror of
 *  the excluded set in cut_core::OpRecord::mutates_timeline(). */
export const NON_TIMELINE_VERBS = new Set<string>([
  'project.create',
  'project.checkpoint',
  'media.import',
  // Review-comment metadata ops (comment.apply is NOT here — it executes
  // timeline edits and IS a timeline op).
  'comment.add',
  'comment.import',
  'comment.resolve',
  'comment.draft',
])

/** True when an op mutated the timeline (tracks/markers/caption_styles) and is
 *  therefore undoable / rebasable — the inverse-bearing predicate the engine
 *  uses for restore tip/rebase selection. */
export function mutatesTimeline(verb: string): boolean {
  return !NON_TIMELINE_VERBS.has(verb)
}
