// panels/Review/OpsFeed.tsx — the OPS tab: live op-log feed.
// Role: rows newest at BOTTOM with auto-follow (pinned to bottom unless the
// user scrolled up); 3-line row anatomy (actor badge + verb + age / rationale
// / effects summary); focused/accepted/rejected states; hover highlights the
// linked transcript span via the shared op-id bridge. Every actor is badged —
// no un-attributed ops, ever (rule 1 of the feel).
//
// OP-REBASE (engine 4408dbd/211f810): a non-tip mutating op gets a SECOND
// undo affordance — "Undo (keep later edits)" — that calls edit.restore{mode:"rebase"}
// to selectively undo it while keeping later ops. It is visually distinct from
// the plain tip reject (x) and gated behind an inline confirm step (history
// surgery = one extra click, never silent). Ops the engine re-based OVER get a
// transient "kept" indicator (KEPT badge + green edge), driven by `keptOps`.
// Callers: Review/index.tsx. Deps: ./shared, lib/client types.

import { useEffect, useMemo, useRef, useState } from 'react'
import type { OpRecord } from '../../lib/client'
import { Icon } from '../../icons'
import { mutatesTimeline } from '../../lib/ops'
import {
  groupOperations,
  operationGroupHeading,
  type IndexedOperation,
} from './opGroupModel'
import { effectsSummary, highlightOpTargets, opSeekMs, timeAgo, type Reviewed } from './shared'

export interface OpsFeedProps {
  ops: OpRecord[]
  cursor: number
  reviewed: Reviewed
  /** Op ids undone by a later edit.restore (derived server truth). */
  restored: Set<string>
  /** Op ids the newest applied op references but cannot be rebased OUT of yet
   * (the dependents named by a refused rebase) — highlighted + scrolled to. */
  highlightedDeps: Set<string>
  /** Op ids transiently flashing a "kept" indicator after a successful rebase
   * (the rebased_over set) — they were re-based OVER and kept intact. */
  keptOps: Set<string>
  /** The current tip op id (newest applied mutating op). Non-tip mutating ops
   * are the ones that get the rebase affordance; the tip uses plain reject. */
  tipOpId: string | null
  onCursor: (idx: number) => void
  onAccept: (opId: string) => void
  onReject: (opId: string) => void
  /** Rebase-reject a non-tip op → edit.restore{mode:"rebase"}. */
  onRebaseReject: (opId: string) => void
  onSeek: (atMs: number) => void
}

// Timeline-mutating classification = lib/ops::mutatesTimeline (the BLOCKLIST
// mirror of the engine), shared with Review/shared.ts + Timeline. Only mutating
// ops are restorable history, so only they get a reject/rebase affordance. Was a
// stale local allowlist that missed edit.grade/title.add/captions.kinetic/….

export default function OpsFeed({
  ops, cursor, reviewed, restored, highlightedDeps, keptOps, tipOpId,
  onCursor, onAccept, onReject, onRebaseReject, onSeek,
}: OpsFeedProps) {
  const listRef = useRef<HTMLDivElement>(null)
  const pinnedRef = useRef(true) // auto-follow while the user stays at bottom
  const [, setTick] = useState(0)
  const lastOpId = ops.length > 0 ? ops[ops.length - 1].op_id : ''
  const groups = useMemo(() => groupOperations(ops), [ops])

  // Re-render every 30s so the `time ago` column stays honest.
  useEffect(() => {
    const t = setInterval(() => setTick((n) => n + 1), 30_000)
    return () => clearInterval(t)
  }, [])

  // Auto-follow: new ops keep the feed pinned to bottom unless user scrolled up.
  useEffect(() => {
    if (pinnedRef.current && listRef.current) listRef.current.scrollTop = listRef.current.scrollHeight
  }, [lastOpId])

  // Keyboard cursor stays visible while skimming.
  useEffect(() => {
    if (cursor < 0) return
    const row = listRef.current?.querySelector(`[data-cut-op-row="${cursor}"]`)
    const group = row?.closest('details')
    if (group instanceof HTMLDetailsElement) group.open = true
    row?.scrollIntoView({ block: 'nearest' })
  }, [cursor])

  // When a selective undo is refused, the named dependents are highlighted;
  // scroll the first one into view so the user sees which later edit is in the
  // way. Pinned-to-bottom is paused for it.
  useEffect(() => {
    if (highlightedDeps.size === 0) return
    const first = [...highlightedDeps][0]
    const el = listRef.current?.querySelector(`[data-cut-op="${CSS.escape(first)}"]`)
    const group = el?.closest('details')
    if (group instanceof HTMLDetailsElement) group.open = true
    el?.scrollIntoView({ block: 'center', behavior: 'smooth' })
  }, [highlightedDeps])

  const onScroll = () => {
    const el = listRef.current
    if (el) pinnedRef.current = el.scrollHeight - el.scrollTop - el.clientHeight < 40
  }

  if (ops.length === 0) {
    return <div className="rr__empty">No edits yet — timeline changes and automated actions appear here.</div>
  }

  const renderRow = ({ op, index }: IndexedOperation) => {
    const isRejected = restored.has(op.op_id)
    const canRebase =
      op.status === 'applied' &&
      op.verb !== 'edit.restore' &&
      mutatesTimeline(op.verb) &&
      op.op_id !== tipOpId &&
      !isRejected
    return (
      <OpRow
        key={op.op_id}
        op={op}
        idx={index}
        focused={index === cursor}
        verdict={isRejected ? 'rejected' : reviewed[op.op_id]}
        canRebase={canRebase}
        isDependent={highlightedDeps.has(op.op_id)}
        isKept={keptOps.has(op.op_id)}
        onCursor={onCursor}
        onAccept={onAccept}
        onReject={onReject}
        onRebaseReject={onRebaseReject}
        onSeek={onSeek}
      />
    )
  }

  return (
    <div className="rr-ops" ref={listRef} onScroll={onScroll} data-cut-ops-feed="">
      {groups.map((group) => {
        if (!group.groupId || group.entries.length < 2) return renderRow(group.entries[0])
        const first = group.entries[0].op
        const last = group.entries[group.entries.length - 1].op
        return (
          <details className="rr-op-group" key={group.key} data-cut-op-group={first.op_id}>
            <summary data-cut-action="op-group-toggle" data-cut-op-group-toggle={first.op_id}>
              <span className={`rr-badge rr-badge--${first.actor?.kind ?? 'system'}`}>
                {(first.actor?.kind ?? 'system').toUpperCase()}
              </span>
              <span className="rr-op-group__heading">{operationGroupHeading(group)}</span>
              <span className="rr-op-group__count">{group.entries.length} ops</span>
              <span className="rr-op-group__age">{timeAgo(last.ts)}</span>
            </summary>
            <div className="rr-op-group__rows">{group.entries.map(renderRow)}</div>
          </details>
        )
      })}
    </div>
  )
}

// ---------------------------------------------------------------------------
// One op row. Hover toggles the cross-panel op-id highlight (transcript span
// flashes blue); click focuses the keyboard cursor; double-click seeks.
// ---------------------------------------------------------------------------

interface OpRowProps {
  op: OpRecord
  idx: number
  focused: boolean
  verdict?: 'accepted' | 'rejected'
  /** This op can be rebased out (applied, mutating, non-tip, not rejected). */
  canRebase: boolean
  /** This op blocks a refused rebase (a named dependent) — highlight it. */
  isDependent: boolean
  /** This op was just re-based OVER and kept — flash a transient KEPT badge. */
  isKept: boolean
  onCursor: (idx: number) => void
  onAccept: (opId: string) => void
  onReject: (opId: string) => void
  onRebaseReject: (opId: string) => void
  onSeek: (atMs: number) => void
}

function OpRow({
  op, idx, focused, verdict, canRebase, isDependent, isKept,
  onCursor, onAccept, onReject, onRebaseReject, onSeek,
}: OpRowProps) {
  const effects = useMemo(() => effectsSummary(op), [op])
  const actorKind = op.actor?.kind ?? 'system'
  const rejected = verdict === 'rejected' || op.status === 'rejected'
  const isRestore = op.verb === 'edit.restore'
  // Confirm-step for the rebase reject: one extra click, no modal. The button
  // flips into a "Rebase out? [yes] [no]" inline confirm before firing — this
  // is history surgery, so it never goes off on a single stray click.
  const [confirming, setConfirming] = useState(false)

  const cls = [
    'rr-op',
    focused ? 'rr-op--focused' : '',
    verdict === 'accepted' ? 'rr-op--accepted' : '',
    rejected ? 'rr-op--rejected' : '',
    isDependent ? 'rr-op--dependent' : '',
    isKept ? 'rr-op--kept' : '',
  ]
    .filter(Boolean)
    .join(' ')

  return (
    <div
      className={cls}
      data-cut-op={op.op_id}
      data-cut-op-row={idx}
      data-cut-dependent={isDependent ? '' : undefined}
      data-cut-kept={isKept ? '' : undefined}
      tabIndex={0}
      onClick={() => onCursor(idx)}
      onFocus={() => onCursor(idx)}
      onDoubleClick={() => {
        const ms = opSeekMs(op)
        if (ms !== null) onSeek(ms)
      }}
      onMouseEnter={() => highlightOpTargets(op.op_id, true)}
      onMouseLeave={() => highlightOpTargets(op.op_id, false)}
    >
      <div className="rr-op__l1">
        <span className={`rr-badge rr-badge--${actorKind}`}>{actorKind.toUpperCase()}</span>
        <span className="rr-op__via">via {op.actor?.via ?? '?'}</span>
        <span className="rr-op__verb">{op.verb}</span>
        {/* transient KEPT badge — this op was re-based OVER and kept intact */}
        {isKept && <span className="rr-op__kept-badge" data-cut-kept-badge={op.op_id}>kept</span>}
        {/* dependent marker — this later edit blocked the requested selective undo */}
        {isDependent && <span className="rr-op__dep-badge" data-cut-dep-badge={op.op_id}>blocked by later edits</span>}
        <span className="rr-op__age">{timeAgo(op.ts)}</span>
      </div>
      <div className={`rr-op__rationale ${op.rationale ? '' : 'rr-op__rationale--none'}`}>
        {op.rationale ?? 'no rationale'}
      </div>
      {effects && <div className="rr-op__effects">{effects}</div>}
      {/* verdict marks + inline actions; restore ops are outcomes, not reviewables */}
      <div className="rr-op__edge">
        {verdict === 'accepted' && <span className="rr-op__check"><Icon name="check" size={14} tone="success" label="accepted" /></span>}
        {rejected && <span className="rr-op__cross"><Icon name="close" size={14} tone="danger" label="rejected" /></span>}
      </div>
      {!verdict && !rejected && !isRestore && (
        <div className="rr-op__actions">
          {confirming ? (
            // Confirm step — distinct, deliberate, one extra click. The label
            // names the consequence so the user knows exactly what the
            // selective undo keeps and removes.
            <div className="rr-op__confirm" data-cut-rebase-confirm={op.op_id}>
              <span className="rr-op__confirm-q">Undo this edit?</span>
              <button
                className="rr-op__act rr-op__act--rebase-go"
                data-cut-action="rebase-confirm"
                title="Undo this change while keeping later edits"
                onClick={(e) => {
                  e.stopPropagation()
                  setConfirming(false)
                  onRebaseReject(op.op_id)
                }}
              >
                Undo (keep later edits)
              </button>
              <button
                className="rr-op__act rr-op__act--cancel"
                data-cut-action="rebase-cancel"
                title="cancel"
                onClick={(e) => {
                  e.stopPropagation()
                  setConfirming(false)
                }}
              >
                ✕
              </button>
            </div>
          ) : (
            <>
              <button
                className="rr-op__act"
                data-cut-action="accept-op"
                title="accept (a)"
                onClick={(e) => {
                  e.stopPropagation()
                  onAccept(op.op_id)
                }}
              >
                ✓
              </button>
              <button
                className="rr-op__act rr-op__act--reject"
                data-cut-action="reject-op"
                title="Reject and restore this edit (X)"
                onClick={(e) => {
                  e.stopPropagation()
                  onReject(op.op_id)
                }}
              >
                ✕
              </button>
              {/* Reject (rebase): non-tip only — selectively undo THIS op,
                  keeping later ops. Distinct glyph + label; opens the confirm. */}
              {canRebase && (
                <button
                  className="rr-op__act rr-op__act--rebase"
                  data-cut-action="rebase-reject-op"
                  title="Undo this change while keeping later edits"
                  onClick={(e) => {
                    e.stopPropagation()
                    setConfirming(true)
                  }}
                >
                  Undo (keep later edits)
                </button>
              )}
            </>
          )}
        </div>
      )}
    </div>
  )
}
