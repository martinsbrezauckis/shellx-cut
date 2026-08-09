// panels/Review — the review rail: OPS · RECEIPTS · DIFF.
// Role: tab container + the RAIL keyboard scope. Focus model:
// clicking the rail (or pressing R anywhere) focuses it; skim keys j/k/a/x/
// Enter act on the OPS feed; Esc blurs back to GLOBAL. Focused scope is shown
// by a 2px --cut top edge. Accept (`a`) is a LOCAL review marker (no verb —
// accepting confirmed truth changes nothing); reject (`x`) dispatches
// edit.restore via onReject. Every row/element carries data-cut-*.
// Callers: App.tsx. Deps: lib/client types./shared./OpsFeed./Receipts,
// ./DiffView.

import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { callVerb } from '../../lib/client'
import type { OpRecord, Project, RenderReceipt } from '../../lib/client'
import { runUserVerb } from '../../lib/userActionFeedback'
import DiffView from './DiffView'
import './mock' // deterministic offline demo; no-op unless ?mock=1 is present
import OpsFeed from './OpsFeed'
import QC from './QC'
import Receipts from './Receipts'
import Scopes from './Scopes'
import { Icon } from '../../icons'
import './review.css'
import {
  opSeekMs,
  restoreOp,
  restoredOpIds,
  revertTo,
  seekPlayhead,
  tipUndoOp,
  type Reviewed,
  type RestoreGuidance,
} from './shared'
import {
  loadReviewMarkers,
  REVIEW_MARKERS_EVENT,
  saveReviewMarkers,
  type ReviewMarkersDetail,
} from './reviewMarkers'

/** Typed props — contract between App.tsx and the Review panel. */
export interface ReviewProps {
  project: Project | null
  /** Current editor playhead; Scopes measures this composed frame by default. */
  playheadMs: number
  /** Op-log, newest last (App seeds via project.ops + appends op_applied events). */
  ops: OpRecord[]
  /** Receipts, newest last (receipt_ready events). */
  receipts: RenderReceipt[]
  /** Deferred tab request from the rail wrapper; survives collapsed/unmounted rails. */
  reviewTabRequest?: ReviewTabRequest | null
  /** Reject an op → App dispatches edit.restore. */
  onReject: (opId: string) => void
  /** Seek request (optional — integration wires; fallback = ui.playhead verb). */
  onSeek?: (atMs: number) => void
  /** Collapse the rail (the header button; mirrors the `\` key + left sidebar). */
  onCollapse?: () => void
  /** Linear undo/redo — step the engine's history cursor (project.undo/redo).
   * The top Undo bar uses these; per-op reject still uses edit.restore. */
  onUndo?: () => void
  onRedo?: () => void
}

export type ReviewTab = 'ops' | 'receipts' | 'qc' | 'scopes' | 'diff'
export interface ReviewTabRequest {
  tab: ReviewTab
  nonce: number
  diff?: { from: string; to: string }
}

export default function Review({ project, playheadMs, ops, receipts, reviewTabRequest, onReject, onSeek, onCollapse, onUndo, onRedo }: ReviewProps) {
  const [tab, setTab] = useState<ReviewTab>('ops')
  const [cursor, setCursor] = useState(-1) // index into ops; -1 = none
  const [reviewed, setReviewed] = useState<Reviewed>({})
  const [railFocused, setRailFocused] = useState(false)
  // Undo surface: the engine's verbatim guidance after a refused non-tip
  // tip-undo OR a rebase blocked by dependents) — shown until the next
  // successful undo or dismissal. NEVER faked.
  const [undoGuidance, setUndoGuidance] = useState<RestoreGuidance | null>(null)
  // Op-rebase success: the ids the engine re-based OVER (rebased_over) flash a
  // transient "kept" indicator. Cleared on a timer (presentational only).
  const [keptFlash, setKeptFlash] = useState<Set<string>>(new Set())
  const keptTimer = useRef<number | null>(null)
  const rootRef = useRef<HTMLElement>(null)
  const skipReviewedSaveRef = useRef(false)

  const restored = useMemo(() => restoredOpIds(ops), [ops])
  // The tip op edit.restore can undo (newest applied, not-restored, mutating).
  const tipOp = useMemo(() => tipUndoOp(ops), [ops])

  // Linear undo/redo availability — the engine's in-memory cursor state, not
  // derivable from the op list alone (the log doesn't encode cursor position).
  // Re-fetched whenever the op log changes (every undo/redo/edit publishes
  // op_applied → App refreshes `ops` → this re-runs), so the Undo/Redo buttons'
  // enabled-state always tracks the live cursor.
  const [avail, setAvail] = useState({ undo: false, redo: false })
  const readAvail = useCallback(async () => {
    const r = await callVerb('project.ops', {})
    if (!r.ok || !r.result) return null
    const res = r.result as { undo_available?: boolean; redo_available?: boolean }
    return { undo: !!res.undo_available, redo: !!res.redo_available }
  }, [project?.name])
  const refreshAvail = useCallback(async () => {
    const next = await readAvail()
    if (next) setAvail(next)
  }, [readAvail])
  useEffect(() => {
    let alive = true
    void readAvail().then((next) => {
      if (alive && next) setAvail(next)
    })
    return () => {
      alive = false
    }
  }, [ops, project?.name, tab, readAvail])
  const undoTipOp = avail.undo ? tipOp : null

  // Dependents named by a refused rebase (parsed from the verbatim cause) →
  // highlighted + scrolled to in the feed. Only while the guidance is showing.
  const highlightedDeps = useMemo(
    () => new Set(undoGuidance?.mode === 'rebase' ? undoGuidance.dependents ?? [] : []),
    [undoGuidance],
  )

  useEffect(() => {
    skipReviewedSaveRef.current = true
    if (!project) {
      setReviewed({})
      return
    }
    try {
      setReviewed(loadReviewMarkers(project.name))
    } catch {
      setReviewed({})
    }
  }, [project?.name])

  useEffect(() => {
    if (!project) return
    if (skipReviewedSaveRef.current) {
      skipReviewedSaveRef.current = false
      return
    }
    saveReviewMarkers(project.name, reviewed)
  }, [project?.name, project, reviewed])

  // Agent Chat accepts/reverts the same op records shown by this rail. Keep one
  // persisted marker model and mirror changes live when both surfaces are open.
  useEffect(() => {
    const onMarkers = (event: Event) => {
      const detail = (event as CustomEvent<ReviewMarkersDetail>).detail
      if (!project || detail?.projectName !== project.name || !Array.isArray(detail.opIds)) return
      setReviewed((prev) => {
        const next = { ...prev }
        for (const opId of detail.opIds) next[opId] = detail.verdict
        return next
      })
    }
    document.addEventListener(REVIEW_MARKERS_EVENT, onMarkers)
    return () => document.removeEventListener(REVIEW_MARKERS_EVENT, onMarkers)
  }, [project?.name])

  // Pending review = applied, not locally reviewed, not undone, and not a
  // restore op itself (a reject's own restore op needs no second review).
  const pendingCount = useMemo(
    () =>
      ops.filter(
        (o) => o.status === 'applied' && o.verb !== 'edit.restore' && !reviewed[o.op_id] && !restored.has(o.op_id),
      ).length,
    [ops, reviewed, restored],
  )

  const seek = useCallback(
    (atMs: number) => {
      if (onSeek) onSeek(atMs)
      else void seekPlayhead(atMs)
    },
    [onSeek],
  )

  const accept = useCallback((opId: string) => {
    setReviewed((prev) => ({ ...prev, [opId]: 'accepted' }))
  }, [])

  // Flash the "kept" indicator on the rebased_over ops for ~2.4s. Presentational
  // only — the rejected/kept truth lives in the op log; this is a glance cue.
  const flashKept = useCallback((ids: string[]) => {
    if (ids.length === 0) return
    if (keptTimer.current) window.clearTimeout(keptTimer.current)
    setKeptFlash(new Set(ids))
    keptTimer.current = window.setTimeout(() => setKeptFlash(new Set()), 2400)
  }, [])
  useEffect(() => () => { if (keptTimer.current) window.clearTimeout(keptTimer.current) }, [])

  // Reject / undo path — owns the edit.restore dispatch so it can read the
  // envelope and surface the engine's guidance VERBATIM. The spawned
  // restore op arrives as a new row via op_applied. `reason` flavors the
  // rationale; `mode` selects tip (default) vs rebase (selective non-tip undo).
  const restore = useCallback(
    async (opId: string, reason: string, mode: 'tip' | 'rebase' = 'tip') => {
      const res = await restoreOp(opId, reason, mode)
      if (res.ok) {
        setReviewed((prev) => ({ ...prev, [opId]: 'rejected' }))
        setUndoGuidance(null) // a successful undo clears any stale guidance
        flashKept(res.rebasedOver) // rebase: glow the ops it kept
      } else {
        // Refusal (tip-only OR rebase-with-dependents): show the engine's words
        // verbatim + (for rebase) highlight the named dependents. Change nothing.
        setUndoGuidance(res.guidance)
      }
    },
    [flashKept],
  )

  const reject = useCallback((opId: string) => void restore(opId, 'rail reject'), [restore])

  // Reject (rebase): selectively undo a NON-TIP op while keeping later ops.
  // Rationale names the op + its verb so the op log reads honestly (the lowered
  // verb is looked up from the live ops list). The confirm-step lives in the row.
  const rebaseReject = useCallback(
    (opId: string) => {
      const target = ops.find((o) => o.op_id === opId)
      const verb = target?.verb ?? '?'
      void restore(opId, `user rebase-reject: ${opId} (${verb}) from history`, 'rebase')
    },
    [ops, restore],
  )

  // project.revert{to} — the engine's escape hatch named by a guardrail's
  // suggested_action. Rolls back to a point (appends restore ops, never rewrites).
  const revert = useCallback(
    async (toOpId: string) => {
      const r = await revertTo(toOpId, `revert to ${toOpId} (from rebase guidance)`)
      if (r.ok) setUndoGuidance(null)
    },
    [],
  )

  // Top Undo/Redo bar — the linear history cursor (project.undo / redo), the
  // same primitive as Ctrl+Z / Ctrl+Shift+Z. Undoing twice steps strictly
  // further back (no oscillation). Falls back to the App-provided callbacks; the
  // per-op reject/rebase below still uses edit.restore (a different operation).
  const doUndo = useCallback(() => {
    if (onUndo) onUndo()
    else if (undoTipOp) void restore(undoTipOp.op_id, 'undo (tip)') // compatibility fallback
    window.setTimeout(() => void refreshAvail(), 350)
  }, [onUndo, undoTipOp, restore, refreshAvail])
  const doRedo = useCallback(() => {
    if (onRedo) onRedo()
    else void runUserVerb('project.redo', {}, 'Could not redo the edit.')
    window.setTimeout(() => void refreshAvail(), 350)
  }, [onRedo, refreshAvail])

  // onReject kept in the contract for App-level hooks, but Review now owns the
  // restore dispatch (single source) so it can read the guidance envelope.
  void onReject

  // --- RAIL keyboard scope: j/k cursor, a accept, x reject, Enter seek --
  const onKeyDown = useCallback(
    (e: React.KeyboardEvent) => {
      // Let the DIFF selects / any form control keep their native keys.
      const tag = (e.target as HTMLElement).tagName
      if (tag === 'SELECT' || tag === 'INPUT' || tag === 'TEXTAREA') return
      if (e.key === 'Escape') {
        ;(e.target as HTMLElement).blur()
        rootRef.current?.blur()
        return
      }
      if (tab !== 'ops') return // skim keys act on the OPS feed
      const move = (d: number) => {
        e.preventDefault()
        setCursor((c) => Math.min(ops.length - 1, Math.max(0, (c === -1 ? (d > 0 ? -1 : ops.length) : c) + d)))
      }
      switch (e.key) {
        case 'j':
          move(1)
          break
        case 'k':
          move(-1)
          break
        case 'a':
          if (cursor >= 0 && ops[cursor]) {
            e.preventDefault()
            accept(ops[cursor].op_id)
            setCursor((c) => Math.min(ops.length - 1, c + 1)) // skim flows downward
          }
          break
        case 'x':
          if (cursor >= 0 && ops[cursor]) {
            e.preventDefault()
            reject(ops[cursor].op_id)
            setCursor((c) => Math.min(ops.length - 1, c + 1))
          }
          break
        case 'Enter':
          if (cursor >= 0 && ops[cursor]) {
            const ms = opSeekMs(ops[cursor])
            if (ms !== null) seek(ms)
          }
          break
      }
    },
    [tab, ops, cursor, accept, reject, seek],
  )

  // GLOBAL↔RAIL scope flag (focus model): Preview and Timeline skip their
  // global transport keys while `document.documentElement.dataset.cutKbscope
  // === 'rail'` — THE cross-panel contract both check (Preview:140,
  // Timeline:549). Nobody set it, so rail skimming (j/k) also started the
  // J/K/L shuttle, whose 100ms clock then overwrote any seek — including
  // agent-driven ui.playhead (found by the installed walkthrough).
  useEffect(() => {
    if (railFocused) document.documentElement.dataset.cutKbscope = 'rail'
    else delete document.documentElement.dataset.cutKbscope
    return () => {
      delete document.documentElement.dataset.cutKbscope
    }
  }, [railFocused])

  // The status-bar last-receipt chip dispatches `cut:open-receipts`.
  // (loose-coupled join — the chip lives in another panel). Switch this rail
  // to the RECEIPTS tab and focus it so the receipt is immediately visible.
  // App separately re-expands a collapsed rail on the same event (display:none
  // can't take focus until React unhides it), so the focus call is deferred a
  // tick to land after that re-render — harmless when the rail is already open.
  useEffect(() => {
    const switchTab = (next: ReviewTab) => {
      setTab(next)
      setTimeout(() => rootRef.current?.focus(), 0)
    }
    const onOpenReceipts = () => {
      switchTab('receipts')
    }
    const onOpenReviewTab = (event: Event) => {
      const detail = (event as CustomEvent<ReviewTab | { tab: ReviewTab }>).detail
      const next = typeof detail === 'string' ? detail : detail?.tab
      if (next === 'ops' || next === 'receipts' || next === 'qc' || next === 'scopes' || next === 'diff') switchTab(next)
    }
    document.addEventListener('cut:open-receipts', onOpenReceipts)
    document.addEventListener('cut:open-review-tab', onOpenReviewTab)
    return () => {
      document.removeEventListener('cut:open-receipts', onOpenReceipts)
      document.removeEventListener('cut:open-review-tab', onOpenReviewTab)
    }
  }, [])

  useEffect(() => {
    if (!reviewTabRequest) return
    setTab(reviewTabRequest.tab)
    setTimeout(() => rootRef.current?.focus(), 0)
  }, [reviewTabRequest?.nonce, reviewTabRequest?.tab])

  // Global `R` focuses the rail. Guarded so a future shell-owned keymap
  // can preempt via preventDefault, and typing in inputs is never hijacked.
  useEffect(() => {
    const onR = (e: KeyboardEvent) => {
      if (e.key !== 'r' && e.key !== 'R') return
      if (e.defaultPrevented || e.ctrlKey || e.metaKey || e.altKey) return
      const t = document.activeElement as HTMLElement | null
      if (t && (t.tagName === 'INPUT' || t.tagName === 'SELECT' || t.tagName === 'TEXTAREA')) return
      rootRef.current?.focus()
    }
    window.addEventListener('keydown', onR)
    return () => window.removeEventListener('keydown', onR)
  }, [])

  return (
    <section
      ref={rootRef}
      className={`panel rr ${railFocused ? 'rr--focused' : ''}`}
      data-panel="review"
      data-cut-panel="review"
      tabIndex={0}
      onKeyDown={onKeyDown}
      onFocus={() => setRailFocused(true)}
      onBlur={(e) => {
        // Only unfocus the SCOPE when focus leaves the rail subtree entirely.
        if (!rootRef.current?.contains(e.relatedTarget as Node)) setRailFocused(false)
      }}
    >
      <div className="panel__header rr__header">
        <span>Review</span>
        {pendingCount > 0 && (
          <span className="rr__pending-chip" data-cut-pending={pendingCount}>
            {pendingCount} pending review
          </span>
        )}
        {onCollapse && (
          <button
            className="rr__collapse"
            data-cut-action="collapse-rail"
            onClick={onCollapse}
            title="Collapse panel (\)"
            aria-label="Collapse review panel"
          >
            {/* chevrons-right — points toward the edge it folds to */}
            <Icon name="collapseRight" size={14} />
          </button>
        )}
      </div>
      <div className="rr__tabs" role="tablist">
        {(['ops', 'receipts', 'qc', 'scopes', 'diff'] as const).map((t) => (
          <button
            key={t}
            role="tab"
            aria-selected={tab === t}
            className={`rr__tab ${tab === t ? 'rr__tab--active' : ''}`}
            data-cut-tab={t}
            data-cut-review-tab={t}
            onClick={() => setTab(t)}
          >
            {t.toUpperCase()}
            {t === 'receipts' && receipts.length > 0 && <span className="rr__tab-count">{receipts.length}</span>}
          </button>
        ))}
      </div>
      {/* The OPS feed IS the history list; this bar adds the
          tip-undo button + the engine's verbatim guidance when a deeper undo
          is refused. Both wired to EXISTING verbs only (edit.restore /
          project.revert). Only on the OPS tab — history is the op log. */}
      {tab === 'ops' && (
        <div className="rr__undo" data-cut-undo-bar>
          <button
            className="rr__undo-btn"
            data-cut-action="undo"
            disabled={!avail.undo}
            title={avail.undo ? 'Undo (Ctrl+Z) — step back one edit' : 'nothing to undo'}
            onClick={doUndo}
          >
            {/* undo arrow */}
            <Icon name="undo" size={14} />
            Undo
          </button>
          <button
            className="rr__undo-btn"
            data-cut-action="redo"
            disabled={!avail.redo}
            title={avail.redo ? 'Redo (Ctrl+Shift+Z / Ctrl+Y) — step forward one edit' : 'nothing to redo'}
            onClick={doRedo}
          >
            <Icon name="redo" size={14} />
            Redo
          </button>
          <span className="rr__undo-tip" data-cut-undo-tip>
            {undoTipOp ? (
              <>
                tip: <span className="rr__undo-verb">{undoTipOp.verb}</span>
              </>
            ) : (
              <span className="rr__undo-empty">nothing to undo</span>
            )}
          </span>
        </div>
      )}
      {/* Engine guidance verbatim — only the tip is restorable. We show
          the engine's own message/cause/suggested_action; we do NOT paraphrase
          and we do NOT fake the deeper restore. */}
      {tab === 'ops' && undoGuidance && (
        <div className="rr__undo-guidance" data-cut-undo-guidance role="status">
          <div className="rr__undo-guidance-head">
            <span className="rr__undo-guidance-tag">restore refused</span>
            <button className="rr__undo-guidance-x" data-cut-action="dismiss-guidance" title="dismiss" onClick={() => setUndoGuidance(null)}><Icon name="close" size={14} label="dismiss" /></button>
          </div>
          <div className="rr__undo-guidance-msg">{undoGuidance.message}</div>
          {undoGuidance.cause && <div className="rr__undo-guidance-cause">{undoGuidance.cause}</div>}
          {undoGuidance.suggested_action && (
            <div className="rr__undo-guidance-action">{undoGuidance.suggested_action}</div>
          )}
          {/* The one suggested action the rail can fire directly: the engine's
              project.revert escape hatch. Target = the op BEFORE the refused
              one (revert{to} restores state AS OF `to`, inclusive), and the
              label says plainly that later ops are dropped — this is the
              blunt alternative the guardrail offers, not a hidden rebase. */}
          {undoGuidance.mode === 'rebase' && undoGuidance.targetOpId && (() => {
            const i = ops.findIndex((o) => o.op_id === undoGuidance.targetOpId)
            const prev = i > 0 ? ops[i - 1] : null
            return prev ? (
              <button
                className="rr__undo-guidance-revert"
                data-cut-action="guidance-revert"
                title="Restore this point in the edit and drop every later change"
                onClick={() => void revert(prev.op_id)}
              >
                Restore this point (drops later edits)
              </button>
            ) : null
          })()}
        </div>
      )}
      <div className="panel__body rr__body">
        {tab === 'ops' && (
          <OpsFeed
            ops={ops}
            cursor={cursor}
            reviewed={reviewed}
            restored={restored}
            highlightedDeps={highlightedDeps}
            keptOps={keptFlash}
            tipOpId={tipOp?.op_id ?? null}
            onCursor={setCursor}
            onAccept={accept}
            onReject={reject}
            onRebaseReject={rebaseReject}
            onSeek={seek}
          />
        )}
        {tab === 'receipts' && <Receipts receipts={receipts} onSeek={seek} />}
        {tab === 'qc' && <QC project={project} />}
        {tab === 'scopes' && <Scopes playheadMs={playheadMs} />}
        {tab === 'diff' && <DiffView project={project} ops={ops} onSeek={seek} request={reviewTabRequest?.diff ? { ...reviewTabRequest.diff, nonce: reviewTabRequest.nonce } : null} />}
      </div>
      <div className="rr__keyhint">
        <kbd>j</kbd>/<kbd>k</kbd> skim · <kbd>a</kbd> accept · <kbd>x</kbd> reject · <kbd>Enter</kbd> seek ·{' '}
        <kbd>Esc</kbd> leave
      </div>
    </section>
  )
}
