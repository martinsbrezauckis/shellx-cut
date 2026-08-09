// panels/Review/shared.ts — helpers shared by Review + Transcript.
// Role: timecode/duration/time-ago formatting, op-log classification (which
// transcript word ranges are currently cut, which ops were rejected via
// edit.restore), effects-line summaries, the cross-panel op-id highlight
// bridge, and a widened verb dispatcher for contract-compliant args the
// scaffold client map doesn't carry yet.
// Callers: panels/Review/*, panels/Transcript/*. Deps: lib/client only.

import { API_BASE, type OpRecord, type VerbResult } from '../../lib/client'
import { VERB_BEHAVIOR } from '../../lib/generatedVerbBehavior'
import { mutatesTimeline } from '../../lib/ops'

// ---------------------------------------------------------------------------
// Formatting: mono = fact; effects line style `v1 −1.7s @ 01:03.2`.
// ---------------------------------------------------------------------------

/** ms → `MM:SS.t` (tenths) — the compact timecode used in effects lines / chips. */
export function fmtTc(ms: number): string {
  const t = Math.max(0, ms)
  const totalS = t / 1000
  const m = Math.floor(totalS / 60)
  const s = totalS - m * 60
  return `${String(m).padStart(2, '0')}:${s < 10 ? '0' : ''}${s.toFixed(1)}`
}

/** ms → human duration: `1.7s`, `41.2s`, `2:05`. */
export function fmtDur(ms: number): string {
  const s = Math.abs(ms) / 1000
  if (s < 90) return `${s.toFixed(1)}s`
  const m = Math.floor(s / 60)
  return `${m}:${String(Math.round(s - m * 60)).padStart(2, '0')}`
}

/** ISO ts → short relative age: `4s`, `2m`, `1h`, `3d`. Re-render on a timer. */
export function timeAgo(ts: string): string {
  const dt = Date.now() - Date.parse(ts)
  if (!Number.isFinite(dt) || dt < 0) return 'now'
  const s = Math.floor(dt / 1000)
  if (s < 60) return `${s}s`
  const m = Math.floor(s / 60)
  if (m < 60) return `${m}m`
  const h = Math.floor(m / 60)
  if (h < 24) return `${h}h`
  return `${Math.floor(h / 24)}d`
}

// ---------------------------------------------------------------------------
// Op-log classification
// ---------------------------------------------------------------------------

/** Local review verdicts (accepted is UI-only; rejected also derives from ops). */
export type Reviewed = Record<string, 'accepted' | 'rejected'>

/** A transcript word range removed by an op (cut_words / remove_fillers run). */
export interface CutSpan {
  opId: string
  asset: string
  /** [start_idx, end_idx] inclusive — transcript.get indices. */
  wordRange: [number, number]
  rationale?: string
}

const isTranscriptCutVerb = (verb: string) =>
  VERB_BEHAVIOR[verb]?.facets.includes('transcript_cut') ?? false

function wordRangeFrom(v: unknown): [number, number] | null {
  return Array.isArray(v) && v.length === 2 && typeof v[0] === 'number' && typeof v[1] === 'number'
    ? [v[0], v[1]]
    : null
}

const objectFrom = (v: unknown): object | null => (v !== null && typeof v === 'object' ? v : null)

function stringField(v: object, name: string): string | undefined {
  const value = Reflect.get(v, name)
  return typeof value === 'string' ? value : undefined
}

function numberField(v: object, name: string): number | undefined {
  const value = Reflect.get(v, name)
  return typeof value === 'number' ? value : undefined
}

function arrayField(v: object, name: string): unknown[] | undefined {
  const value = Reflect.get(v, name)
  return Array.isArray(value) ? value : undefined
}

function cutErrorFrom(v: unknown): VerbResult['error'] | undefined {
  const obj = objectFrom(v)
  if (!obj) return undefined
  const code = stringField(obj, 'code')
  const message = stringField(obj, 'message')
  if (!code || !message) return undefined
  return {
    code,
    message,
    cause: stringField(obj, 'cause') ?? '',
    clip_id: stringField(obj, 'clip_id'),
    at_ms: numberField(obj, 'at_ms'),
    suggested_action: stringField(obj, 'suggested_action'),
  }
}

function stringArrayFrom(v: unknown): string[] {
  return Array.isArray(v) ? v.filter((x) => typeof x === 'string') : []
}

function verbResultFrom(v: unknown): VerbResult | null {
  const obj = objectFrom(v)
  if (!obj) return null
  const okValue = Reflect.get(obj, 'ok')
  if (typeof okValue !== 'boolean') return null
  const opIds = stringArrayFrom(Reflect.get(obj, 'op_ids'))
  const result: VerbResult = { ok: okValue, result: Reflect.get(obj, 'result') }
  if (opIds.length > 0) result.op_ids = opIds
  const error = cutErrorFrom(Reflect.get(obj, 'error'))
  if (error) result.error = error
  return result
}

/** Op ids undone by a later applied `edit.restore` op — covers BOTH modes
 * (tip undo + rebase-out both record `args.op_id` as the restored target). The
 * rejected/struck-through row state derives from this set. */
export function restoredOpIds(ops: OpRecord[]): Set<string> {
  const out = new Set<string>()
  for (const op of ops) {
    if (op.verb === 'edit.restore' && op.status === 'applied') {
      const args = objectFrom(op.args)
      if (args) {
        const target = stringField(args, 'op_id')
        if (target) out.add(target)
      }
    }
  }
  return out
}

/** True when an op record IS a rebase-mode restore (vs a tip restore). The
 * mode rides on the op's args (`mode:"rebase"`) — server truth, set by
 * rebase_out. Tip restores omit it. */
export function isRebaseOp(op: OpRecord): boolean {
  const args = objectFrom(op.args)
  return op.verb === 'edit.restore' && args !== null && stringField(args, 'mode') === 'rebase'
}

/** The `rebased_over` op ids recorded on a rebase op (the later ops it
 * re-based OVER and KEPT intact). Lives on the op's effects detail
 * (`rebased_over`), recorded by store::rebase_out — server truth, stable under
 * replay. Empty for a tip restore or a non-restore op. */
export function rebasedOverOf(op: OpRecord): string[] {
  if (!isRebaseOp(op)) return []
  for (const eff of op.effects ?? []) {
    const v = eff.rebased_over
    if (Array.isArray(v)) return v.filter((x) => typeof x === 'string')
  }
  return []
}

/**
 * Word ranges currently CUT (struck-through in the transcript): every applied
 * transcript-domain op that addresses words (args.word_range, or word_range on
 * an effect entry — remove_fillers emits one op per run) minus restored ones.
 */
export function activeCutSpans(ops: OpRecord[]): CutSpan[] {
  const restored = restoredOpIds(ops)
  const spans: CutSpan[] = []
  for (const op of ops) {
    if (op.status !== 'applied' || restored.has(op.op_id)) continue
    if (!isTranscriptCutVerb(op.verb)) continue
    const args = objectFrom(op.args)
    if (args) {
      const asset = stringField(args, 'asset')
      const wordRange = wordRangeFrom(Reflect.get(args, 'word_range'))
      if (asset && wordRange) {
        spans.push({ opId: op.op_id, asset, wordRange, rationale: op.rationale })
        continue
      }
    }
    // remove_fillers / remove_silences: per-span ops carry the range on effects.
    for (const eff of op.effects ?? []) {
      const wordRange = wordRangeFrom(eff.word_range)
      if (typeof eff.asset === 'string' && wordRange) {
        spans.push({ opId: op.op_id, asset: eff.asset, wordRange, rationale: op.rationale })
      }
    }
  }
  return spans
}

/** Effects → one summary line: `v1 −1.7s @ 01:03.2 · a1t −1.7s @ 01:03.2`. */
export function effectsSummary(op: OpRecord): string {
  const parts: string[] = []
  for (const eff of op.effects ?? []) {
    const track = typeof eff.track === 'string' ? eff.track : ''
    const removed = wordRangeFrom(eff.removed_ms)
    const range = wordRangeFrom(eff.range_ms)
    if (removed) {
      parts.push(`${track} −${fmtDur(removed[1] - removed[0])} @ ${fmtTc(removed[0])}`.trim())
    } else if (range) {
      parts.push(`${track} ${fmtDur(range[1] - range[0])} @ ${fmtTc(range[0])}`.trim())
    } else if (typeof eff.at_ms === 'number') {
      parts.push(`${track} @ ${fmtTc(eff.at_ms)}`.trim())
    } else if (track) {
      parts.push(track)
    }
  }
  return parts.join(' · ')
}

/** First timeline position an op touched — the `Enter`-to-seek target. */
export function opSeekMs(op: OpRecord): number | null {
  for (const eff of op.effects ?? []) {
    const removed = wordRangeFrom(eff.removed_ms)
    const range = wordRangeFrom(eff.range_ms)
    if (removed) return removed[0]
    if (range) return range[0]
    if (typeof eff.at_ms === 'number') return eff.at_ms
  }
  const args = objectFrom(op.args)
  if (args) {
    const atMs = numberField(args, 'at_ms')
    if (atMs != null) return atMs
  }
  return null
}

// ---------------------------------------------------------------------------
// Cross-panel highlight bridge: hovering a cut_words op row
// highlights its transcript span). The join key is the op id — both panels
// stamp `data-op-id` on the linked DOM; shared op-id is the
// join key"). DOM class toggle, not React state: the panels have no common
// parent owns it, and the highlight is purely presentational.
// ---------------------------------------------------------------------------

/** Toggle the hover-highlight on every element linked to `opId`. */
export function highlightOpTargets(opId: string, on: boolean): void {
  document.querySelectorAll(`[data-op-id="${CSS.escape(opId)}"]`).forEach((el) => {
    el.classList.toggle('op-link-hot', on)
  })
}

// ---------------------------------------------------------------------------
// Widened verb dispatch — schema/verbs.json is the public verb contract;
// the scaffold's VerbArgs map lags it on the scope contract/12 args (asset/track scope,
// REQUIRED aggressiveness). Integration re-syncs client.ts; until then this UI
// dispatches contract-compliant wire shapes through this single widening seam.
// ---------------------------------------------------------------------------

/** Call a verb by wire name with args validated by the server, not the map. */
export async function dispatchVerb(name: string, args: unknown): Promise<VerbResult> {
  const res = await fetch(`${API_BASE}/api/verb/${encodeURIComponent(name)}`, {
    method: 'POST',
    headers: { 'content-type': 'application/json', 'x-cut-actor': 'human:ui:ui' },
    body: JSON.stringify(args ?? {}),
  })
  const body = await res.json()
  return verbResultFrom(body)
    ?? { ok: false, error: { code: 'bad_response', message: 'server returned an invalid verb envelope', cause: 'invalid JSON envelope' } }
}

/** Seek the shared playhead (ui.playhead). Returns confirmed ms or null. */
export async function seekPlayhead(atMs: number): Promise<number | null> {
  const r = await dispatchVerb('ui.playhead', { at_ms: Math.max(0, Math.round(atMs)) })
  if (!r.ok) return null
  const result = objectFrom(r.result)
  const playheadMs = result ? numberField(result, 'playhead_ms') : undefined
  return playheadMs ?? Math.max(0, Math.round(atMs))
}

// ---------------------------------------------------------------------------
// Undo surface — wired to EXISTING verbs only. edit.restore is TIP-ONLY by
// design: it recomputes the target's pre-op journal prefix, so only the LATEST
// timeline-mutating op can be restored. A non-tip target returns a guardrail
// error whose message/cause/suggested_action name the deeper path
// (project.revert{to}). We surface that engine guidance VERBATIM — the UI
// never fakes a restore it can't perform, and never invents its own message.
// ---------------------------------------------------------------------------

// Timeline-mutating classification now lives in lib/ops (mutatesTimeline),
// derived from the generated schema behavior contract. Unknown verbs stay
// conservative, so a stale UI artifact cannot hide a timeline edit from undo.

/** The newest applied, not-yet-restored timeline op — the only op
 * edit.restore{mode:"tip"} will accept (the "tip"). null = nothing to undo.
 *
 * `edit.restore` ops COUNT because they are timeline mutations. The engine
 * selects by generated mutation class, never `inverse.is_some()`. So after any
 * restore/rebase the restore op itself is the tip; restoring it recomputes the
 * pre-restore state. The rebase-rail walk
 * caught the old behavior skipping restores: the Undo button then
 * pointed at an op the engine refused as non-tip (graceful, but wrong). */
export function tipUndoOp(ops: OpRecord[]): OpRecord | null {
  const restored = restoredOpIds(ops)
  for (let i = ops.length - 1; i >= 0; i--) {
    const op = ops[i]
    if (op.status !== 'applied') continue
    if (op.verb !== 'edit.restore' && !mutatesTimeline(op.verb)) continue
    if (restored.has(op.op_id)) continue
    return op
  }
  return null
}

/** The engine's structured guidance after a refused restore — surfaced
 * VERBATIM in the UI (never paraphrased). For a rebase guardrail the engine's
 * `cause` NAMES the dependent op ids (e.g. "op_000006 (edit.gain via c3)");
 * `dependents` are those ids PARSED OUT of the verbatim cause purely for
 * highlight/scroll affordances — the displayed text always stays the engine's
 * own words. `mode` records which restore mode was refused. */
export interface RestoreGuidance {
  code: string
  message: string
  cause?: string
  suggested_action?: string
  /** "tip" | "rebase" — which mode the refused dispatch used. */
  mode?: 'tip' | 'rebase'
  /** The op id whose restore was refused (the rebase/tip target). */
  targetOpId?: string
  /** Dependent op ids parsed from a rebase guardrail cause (for highlight +
   * scroll-to). Display still uses the verbatim cause; this is navigation only. */
  dependents?: string[]
}

/** Pull `op_NNNNNN` ids out of a guardrail cause string. The engine writes
 * dependents as "op_000006 (edit.gain via c3); op_000008 (...)" — we extract
 * the ids ONLY to drive highlight/scroll, never to rewrite the message. */
export function parseDependentOpIds(cause?: string): string[] {
  if (!cause) return []
  const ids = cause.match(/op_\d{6,}/g) ?? []
  return Array.from(new Set(ids))
}

/** What restoreOp returns on success. A rebase carries the kept op ids so the
 * caller can flash the transient "kept" indicators (tip restore → empty). */
export interface RestoreOk {
  ok: true
  /** rebased_over op ids (the later ops kept intact) — empty for a tip undo. */
  rebasedOver: string[]
}

/** Dispatch edit.restore for an op. `mode` selects tip (default) vs rebase
 * (selective non-tip undo). On success returns {ok:true, rebasedOver}; on a
 * guardrail (tip-only OR rebase-dependents) or any failure returns the engine's
 * guidance VERBATIM so the caller shows it without inventing copy — plus the
 * parsed dependent ids for navigation. */
export async function restoreOp(
  opId: string,
  rationale?: string,
  mode: 'tip' | 'rebase' = 'tip',
): Promise<RestoreOk | { ok: false; guidance: RestoreGuidance }> {
  // tip is the default wire shape; only send mode when rebasing (keeps tip
  // calls byte-identical to the pre-rebase contract).
  const args: { op_id: string; rationale?: string; mode?: 'rebase' } = { op_id: opId, rationale }
  if (mode === 'rebase') args.mode = 'rebase'
  const r = await dispatchVerb('edit.restore', args)
  if (r.ok) {
    const result = objectFrom(r.result)
    const rebasedOverValue = result ? arrayField(result, 'rebased_over') : undefined
    const rebasedOver = rebasedOverValue
      ? rebasedOverValue.filter((x) => typeof x === 'string')
      : []
    return { ok: true, rebasedOver }
  }
  const e = r.error
  return {
    ok: false,
    guidance: {
      code: e?.code ?? 'error',
      message: e?.message ?? 'restore failed',
      cause: e?.cause,
      suggested_action: e?.suggested_action,
      mode,
      targetOpId: opId,
      dependents: parseDependentOpIds(e?.cause),
    },
  }
}

/** Deliberate rollback to a point — project.revert{to}. The engine's escape
 * hatch named by the tip-only guidance; appends restore ops, never rewrites. */
export async function revertTo(to: string, rationale?: string): Promise<VerbResult> {
  return dispatchVerb('project.revert', { to, rationale })
}
