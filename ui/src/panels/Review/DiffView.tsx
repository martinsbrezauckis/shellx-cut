// panels/Review/DiffView.tsx — the DIFF tab: two checkpoint
// selectors → project.diff{from,to} → summary chips (clips added/removed/
// moved, Δduration, tracks touched) + the op list between, tinted add/del
// per office-suite .diff-line semantics (removal verbs red, insert green).
// Callers: Review/index.tsx. Deps: ./shared, lib/client types.

import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import type { OpRecord, Project } from '../../lib/client'
import { dispatchVerb, effectsSummary, fmtDur, opSeekMs } from './shared'

export interface DiffViewProps {
  project: Project | null
  /** Live op log — supplies the HEAD option (diff up to the latest op). */
  ops: OpRecord[]
  onSeek: (atMs: number) => void
  request?: { from: string; to: string; nonce: number } | null
}

/** project.diff result shape (timeline/op-log contract: ops + computed summary). The server
 * emits clip-id ARRAYS for added/removed/moved and per-track ranges for
 * tracks_touched ({track, ranges_ms}); plain counts/names are also accepted
 * defensively. */
interface DiffSummary {
  from_op?: string
  to_op?: string
  ops?: OpRecord[]
  clips_added?: string[] | number
  clips_removed?: string[] | number
  clips_moved?: string[] | number
  duration_delta_ms?: number
  tracks_touched?: Array<string | { track?: string; ranges_ms?: [number, number][] }>
}

/** Clip-set size whichever shape the field arrived in (id array or count). */
function clipCount(v: string[] | number | undefined): number {
  return Array.isArray(v) ? v.length : typeof v === 'number' ? v : 0
}

/** Removal-ish verbs tint red, additive verbs green — glanceable op polarity. */
function opTint(verb: string): 'del' | 'add' | '' {
  if (/cut_words|remove_silences|remove_fillers|ripple_delete|remove_marker/.test(verb)) return 'del'
  if (/insert|import|generate|add_marker/.test(verb)) return 'add'
  return ''
}

export default function DiffView({ project, ops, onSeek, request }: DiffViewProps) {
  const checkpoints = project?.checkpoints ?? []
  const headOp = ops.length > 0 ? ops[ops.length - 1].op_id : null
  const [from, setFrom] = useState('')
  const [to, setTo] = useState('')
  const [diff, setDiff] = useState<DiffSummary | null>(null)
  const [error, setError] = useState('')
  const defaultsSeededRef = useRef(false)
  const defaultsSeedKey = `${project?.name ?? ''}|${checkpoints.map((cp) => cp.id).join(',')}`

  // Sensible defaults once per project/checkpoint set: oldest checkpoint → HEAD.
  useEffect(() => {
    defaultsSeededRef.current = false
  }, [defaultsSeedKey])

  useEffect(() => {
    if (!request?.from || !request.to) return
    defaultsSeededRef.current = true
    setFrom(request.from)
    setTo(request.to)
  }, [request?.nonce, request?.from, request?.to])

  useEffect(() => {
    if (defaultsSeededRef.current) return
    if (checkpoints.length === 0 && !headOp) return
    defaultsSeededRef.current = true
    if (!from && checkpoints.length > 0) setFrom(checkpoints[0].id)
    if (!to && (checkpoints.length > 1 || headOp)) setTo(checkpoints.length > 1 ? checkpoints[checkpoints.length - 1].id : headOp ?? '')
  }, [checkpoints, headOp, from, to])

  const run = useCallback(async () => {
    if (!from || !to || from === to) {
      setDiff(null)
      setError(from === to ? 'Pick two different comparison points.' : '')
      return
    }
    setError('')
    const r = await dispatchVerb('project.diff', { from, to })
    if (r.ok) setDiff((r.result ?? {}) as DiffSummary)
    else {
      setDiff(null)
      setError(r.error?.message ?? 'diff failed')
    }
  }, [from, to])

  useEffect(() => {
    void run()
  }, [run])

  const chips = useMemo(() => {
    if (!diff) return []
    const out: Array<{ label: string; cls: string }> = []
    const added = clipCount(diff.clips_added)
    const removed = clipCount(diff.clips_removed)
    const moved = clipCount(diff.clips_moved)
    if (added) out.push({ label: `+${added} clips`, cls: 'rr-chip--add' })
    if (removed) out.push({ label: `−${removed} clips`, cls: 'rr-chip--del' })
    if (moved) out.push({ label: `~${moved} moved`, cls: '' })
    if (typeof diff.duration_delta_ms === 'number' && diff.duration_delta_ms !== 0) {
      const d = diff.duration_delta_ms
      out.push({ label: `Δ ${d > 0 ? '+' : '−'}${fmtDur(Math.abs(d))}`, cls: d < 0 ? 'rr-chip--del' : 'rr-chip--add' })
    }
    // tracks_touched entries are {track, ranges_ms} (timeline/op-log contract per-track ranges).
    const tracks = (diff.tracks_touched ?? [])
      .map((t) => (typeof t === 'string' ? t : (t?.track ?? '')))
      .filter(Boolean)
    if (tracks.length) out.push({ label: tracks.join(' '), cls: '' })
    return out
  }, [diff])

  if (checkpoints.length === 0 && !headOp) {
    return (
      <div className="rr__empty">
        No comparison point yet — run a guided workflow or ask Agent Chat to save one, then return here to compare it with the current edit.
      </div>
    )
  }

  const options = (
    <>
      {checkpoints.map((cp) => (
        <option key={cp.id} value={cp.id}>
          {cp.name} ({cp.at_op})
        </option>
      ))}
      {headOp && <option value={headOp}>HEAD ({headOp})</option>}
      {[from, to].filter((ref, index, refs) =>
        ref
        && refs.indexOf(ref) === index
        && ref !== headOp
        && !checkpoints.some((checkpoint) => checkpoint.id === ref),
      ).map((ref) => <option key={ref} value={ref}>TURN ({ref})</option>)}
    </>
  )

  return (
    <div className="rr-diff" data-cut-diff="">
      <div className="rr-diff__selectors">
        <select className="rr-diff__sel" data-cut-diff-from="" aria-label="Compare from" value={from} onChange={(e) => setFrom(e.target.value)}>
          <option value="">from…</option>
          {options}
        </select>
        <span className="rr-diff__arrow">→</span>
        <select className="rr-diff__sel" data-cut-diff-to="" aria-label="Compare to" value={to} onChange={(e) => setTo(e.target.value)}>
          <option value="">to…</option>
          {options}
        </select>
      </div>
      {error && <div className="rr-diff__error">{error}</div>}
      {diff && (
        <>
          <div className="rr-diff__chips">
            {chips.map((c, i) => (
              <span key={i} className={`rr-chip ${c.cls}`}>
                {c.label}
              </span>
            ))}
            {chips.length === 0 && <span className="rr-chip">no changes</span>}
          </div>
          <div className="rr-diff__ops">
            {(diff.ops ?? []).map((op, i) => (
              <button
                key={`${op.op_id}-${i}`}
                className={`rr-diff__op rr-diff__op--${opTint(op.verb)}`}
                data-cut-diff-op={op.op_id}
                onClick={() => {
                  const ms = opSeekMs(op)
                  if (ms !== null) onSeek(ms)
                }}
                title={op.rationale ?? op.op_id}
              >
                <span className="rr-diff__op-verb">{op.verb}</span>
                <span className="rr-diff__op-fx">{effectsSummary(op) || op.op_id}</span>
              </button>
            ))}
          </div>
        </>
      )}
    </div>
  )
}
