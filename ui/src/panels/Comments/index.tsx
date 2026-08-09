// panels/Comments/index.tsx — review-comment rail (left side).
//
// The client comment → agent change loop made visual: leave a timecoded review
// note, the agent DRAFTS the edit (concrete verbs), you review the proposal and
// APPLY (revertibly) or DISMISS. Reads project.comments (server truth, refreshed
// on every op_applied — no polling); all mutations go through verbs
// (comment.add / draft / apply / resolve). Mirrors the right Review rail's
// collapse and the Canvas LayersRail row pattern (house style, --cut accent).
//
// Zero-local-mutation contract: the UI is an API client — every action is a verb; the
// panel never mutates project state locally (the op_applied snapshot is truth).
// Deps: lib/client (verbs + Comment type), Timeline/layout (timecode). Caller:
// App.tsx (left rail).

import { useCallback, useEffect, useMemo, useState } from 'react'
import { callVerb, exportUrl, type Comment, type Project } from '../../lib/client'
import { resolveCommentTime, type ResolvedCommentTime } from '../../lib/commentAnchors'
import { isTauri, pickReviewFeedback } from '../../lib/tauri'
import { timecode } from '../Timeline/layout'
import { Icon } from '../../icons'
import './comments.css'

const FILTERS = ['all', 'open', 'addressed', 'dismissed'] as const
type Filter = (typeof FILTERS)[number]
const FILTER_LABEL: Record<Filter, string> = { all: 'All', open: 'Open', addressed: 'Done', dismissed: 'Dismissed' }

function proposedEditLabel(verb: string, args: Record<string, unknown>): string {
  const rationale = typeof args.rationale === 'string' ? args.rationale.trim() : ''
  if (rationale) return rationale.charAt(0).toUpperCase() + rationale.slice(1)
  switch (verb) {
    case 'edit.ripple_delete': return 'Remove a section and close the gap'
    case 'edit.trim': return 'Trim a clip'
    case 'edit.move': return 'Move a clip'
    case 'edit.split': return 'Split a clip'
    case 'edit.restore': return 'Restore an earlier edit'
    default:
      if (verb.startsWith('captions.')) return 'Update captions'
      if (verb.startsWith('transcript.')) return 'Edit spoken words'
      if (verb.startsWith('audio.')) return 'Adjust audio'
      if (verb.startsWith('edit.')) return 'Adjust the timeline'
      if (verb.startsWith('project.')) return 'Update the project'
      if (verb.startsWith('render.')) return 'Render a new version'
      return 'Apply the proposed edit'
  }
}

interface Props {
  project: Project | null
  playheadMs: number
  /** Jump the playhead to a comment's timecode. */
  onSeek: (ms: number) => void
  /** Collapse the rail (the `]` toggle / topbar button mirror). */
  onCollapse: () => void
  /** A comment to select + scroll to (set by a timeline pin click). The `n`
   *  counter re-fires the effect even when the same id is clicked twice. */
  focus?: { id: string; n: number } | null
}

/** The review-comment rail. */
export default function Comments({ project, playheadMs, onSeek, onCollapse, focus }: Props) {
  const comments = useMemo(() => project?.comments ?? [], [project])
  const [filter, setFilter] = useState<Filter>('all')
  const [text, setText] = useState('')
  const [selected, setSelected] = useState<string | null>(null)
  // Per-comment in-flight action ('drafting' | 'applying') — drives the spinner.
  const [busy, setBusy] = useState<Record<string, string>>({})
  const [handoffBusy, setHandoffBusy] = useState<'export' | 'import' | null>(null)
  const [reviewPackage, setReviewPackage] = useState<string | null>(null)
  const [note, setNote] = useState<string | null>(null)
  const flash = (t: string) => { setNote(t); window.setTimeout(() => setNote(null), 4500) }

  const counts = useMemo(
    () => ({
      all: comments.length,
      open: comments.filter((c) => c.status === 'open').length,
      addressed: comments.filter((c) => c.status === 'addressed').length,
      dismissed: comments.filter((c) => c.status === 'dismissed').length,
    }),
    [comments],
  )
  const shown = useMemo(
    () =>
      comments
        .filter((c) => filter === 'all' || c.status === filter)
        .map((comment) => ({ comment, time: resolveCommentTime(project, comment) }))
        .sort((a, b) => a.time.atMs - b.time.atMs),
    [comments, filter, project],
  )

  const add = useCallback(async () => {
    const t = text.trim()
    if (!t || !project) return
    const r = await callVerb('comment.add', { at_ms: Math.max(0, Math.round(playheadMs)), text: t, author: 'me' })
    if (r.ok) setText('')
    else flash(`add: ${r.error?.code ?? 'failed'}`)
  }, [text, playheadMs, project])

  const draft = useCallback(async (id: string) => {
    setBusy((b) => ({ ...b, [id]: 'drafting' }))
    const r = await callVerb('comment.draft', { comment_id: id })
    setBusy((b) => { const n = { ...b }; delete n[id]; return n })
    const res = r.result as { status?: string; reason?: unknown } | undefined
    if (!r.ok) flash(`draft: ${r.error?.code ?? 'failed'}`)
    else if (res?.status !== 'completed') flash(`draft ${res?.status}${res?.reason ? `: ${String(res.reason).slice(0, 80)}` : ''}`)
  }, [])

  const apply = useCallback(async (id: string) => {
    setBusy((b) => ({ ...b, [id]: 'applying' }))
    const r = await callVerb('comment.apply', { comment_id: id })
    setBusy((b) => { const n = { ...b }; delete n[id]; return n })
    const res = r.result as { status?: string; failed_verb?: string; checkpoint?: string } | undefined
    if (!r.ok) flash(`apply: ${r.error?.code ?? 'failed'}`)
    else if (res?.status === 'failed') {
      const step = res.failed_verb ? ` at ${res.failed_verb}` : ''
      const recovery = res.checkpoint ? ` — revert point ${res.checkpoint}` : ''
      flash(`apply stopped${step}${recovery}`)
    } else {
      const checkpoint = res && res.checkpoint ? res.checkpoint : ''
      flash(checkpoint ? `applied · undo point ${checkpoint}` : 'applied')
    }
  }, [])

  const resolve = useCallback(async (id: string, status: Comment['status']) => {
    const r = await callVerb('comment.resolve', { comment_id: id, status })
    if (!r.ok) flash(`resolve: ${r.error?.code ?? 'failed'}`)
  }, [])

  const exportReview = useCallback(async () => {
    if (!project || handoffBusy) return
    setHandoffBusy('export')
    const response = await callVerb('comment.export', {})
    setHandoffBusy(null)
    if (!response.ok) {
      flash(`export review: ${response.error?.message ?? response.error?.code ?? 'failed'}`)
      return
    }
    const path = (response.result as { path?: string } | undefined)?.path
    if (!path) {
      flash('export review: missing package path')
      return
    }
    setReviewPackage(exportUrl(path))
    flash('review package ready')
  }, [handoffBusy, project])

  const importReview = useCallback(async () => {
    if (!project || handoffBusy) return
    if (!isTauri()) {
      flash('feedback import is available in the desktop app')
      return
    }
    const path = await pickReviewFeedback()
    if (!path) return
    setHandoffBusy('import')
    const response = await callVerb('comment.import', { path })
    setHandoffBusy(null)
    if (!response.ok) {
      flash(`import feedback: ${response.error?.message ?? response.error?.code ?? 'failed'}`)
      return
    }
    const count = (response.result as { count?: number } | undefined)?.count ?? 0
    flash(`imported ${count} review ${count === 1 ? 'note' : 'notes'}`)
  }, [handoffBusy, project])

  // A timeline comment-pin click focuses the comment here: App opens the rail
  // and passes `focus` (the panel mounts after the rail opens, so a prop beats
  // an event that would fire before this listener attaches). Select + reveal it.
  useEffect(() => {
    if (!focus?.id) return
    setFilter('all') // ensure it's visible regardless of the active filter
    setSelected(focus.id)
    window.setTimeout(() => document.querySelector(`[data-cut-comment="${focus.id}"]`)?.scrollIntoView({ block: 'nearest' }), 0)
  }, [focus])

  useEffect(() => {
    setReviewPackage(null)
  }, [project?.name])

  return (
    <section className="panel cm" data-cut-panel="comments">
      <header className="cm__head">
        <span className="cm__title">
          Comments{comments.length > 0 && <span className="cm__count">{comments.length}</span>}
        </span>
        <div className="cm__head-actions">
          <button
            className="cm__icon"
            data-cut-action="comment-export-review"
            title="Export review package"
            disabled={!project || !!handoffBusy}
            onClick={() => void exportReview()}
            aria-label="Export review package"
          >
            <Icon name={handoffBusy === 'export' ? 'spinner' : 'share'} size={14} />
          </button>
          <button
            className="cm__icon"
            data-cut-action="comment-import-feedback"
            title={isTauri() ? 'Import review feedback' : 'Import review feedback in the desktop app'}
            disabled={!project || !!handoffBusy}
            onClick={() => void importReview()}
            aria-label="Import review feedback"
          >
            <Icon name={handoffBusy === 'import' ? 'spinner' : 'import'} size={14} />
          </button>
          <button className="cm__icon" data-cut-action="comments-collapse" title="Hide comments (Ctrl/Cmd+Shift+C)" onClick={onCollapse} aria-label="Hide comments">
            <Chevron />
          </button>
        </div>
      </header>

      {reviewPackage && (
        <a className="cm__package" data-cut-review-package href={reviewPackage} target="_blank" rel="noreferrer">
          <Icon name="link" size={14} />
          Open review page
        </a>
      )}

      <div className="cm__filters" role="tablist" aria-label="Comment status filter">
        {FILTERS.map((f) => (
          <button
            key={f}
            role="tab"
            aria-selected={filter === f}
            className={`cm__filter ${filter === f ? 'cm__filter--on' : ''}`}
            data-cut-comment-filter={f}
            onClick={() => setFilter(f)}
          >
            {FILTER_LABEL[f]}
            {counts[f] > 0 && <span className="cm__fcount">{counts[f]}</span>}
          </button>
        ))}
      </div>

      <div className="cm__add">
        <span className="cm__add-tc" title="The comment lands at the playhead">{timecode(playheadMs)}</span>
        <input
          className="cm__add-input"
          data-cut-comment-input
          placeholder={project ? 'Add a review note…' : 'Open a project first'}
          value={text}
          disabled={!project}
          onChange={(e) => setText(e.target.value)}
          onKeyDown={(e) => { if (e.key === 'Enter') void add() }}
        />
        <button
          className="cm__add-btn"
          data-cut-action="comment-add"
          disabled={!project || !text.trim()}
          title="Add comment at the playhead (Enter)"
          onClick={() => void add()}
          aria-label="Add comment"
        >
          <Enter />
        </button>
      </div>

      {note && <div className="cm__note" data-cut-comment-note>{note}</div>}

      <div className="cm__list">
        {shown.length === 0 ? (
          <div className="cm__empty" data-cut-comment-empty>
            <div className="cm__empty-t">No {filter === 'all' ? '' : `${FILTER_LABEL[filter].toLowerCase()} `}comments</div>
            <div className="cm__empty-b">
              Seek to a moment and add a review note. The agent drafts the edit — you review, apply (revertibly), or dismiss.
            </div>
          </div>
        ) : (
          shown.map(({ comment: c, time }) => (
            <CommentRow
              key={c.id}
              c={c}
              time={time}
              selected={selected === c.id}
              busy={busy[c.id]}
              onToggle={() => setSelected((s) => (s === c.id ? null : c.id))}
              onSeek={() => onSeek(time.atMs)}
              onDraft={() => void draft(c.id)}
              onApply={() => void apply(c.id)}
              onDone={() => void resolve(c.id, c.status === 'addressed' ? 'open' : 'addressed')}
              onDismiss={() => void resolve(c.id, c.status === 'dismissed' ? 'open' : 'dismissed')}
            />
          ))
        )}
      </div>
    </section>
  )
}

interface RowProps {
  c: Comment
  time: ResolvedCommentTime
  selected: boolean
  busy?: string
  onToggle: () => void
  onSeek: () => void
  onDraft: () => void
  onApply: () => void
  onDone: () => void
  onDismiss: () => void
}

function CommentRow({ c, time, selected, busy, onToggle, onSeek, onDraft, onApply, onDone, onDismiss }: RowProps) {
  const draft = c.draft
  const hasVerbs = !!draft && (draft.verbs?.length ?? 0) > 0
  const invalid = !!draft?.validation && !draft.validation.ok
  const bodyId = `comment-body-${c.id}`
  return (
    <div className={`cm__row cm__row--${c.status} ${selected ? 'cm__row--sel' : ''}`} data-cut-comment={c.id} data-cut-comment-status={c.status} data-cut-comment-anchor={time.status}>
      <div className="cm__row-head" onClick={onToggle}>
        <button
          type="button"
          className="cm__row-caret"
          data-cut-comment-disclosure
          aria-label={selected ? 'Hide comment actions' : 'Show comment actions'}
          aria-expanded={selected}
          aria-controls={bodyId}
          onClick={(e) => { e.stopPropagation(); onToggle() }}
        >
          <Icon name={selected ? 'chevronDown' : 'chevronRight'} size={14} />
        </button>
        <button
          className="cm__tc"
          data-cut-action="comment-seek"
          title="Jump to this moment"
          onClick={(e) => { e.stopPropagation(); onSeek() }}
        >
          {timecode(time.atMs)}{time.endMs != null ? `–${timecode(time.endMs)}` : ''}
        </button>
        <span className={`cm__dot cm__dot--${c.status}`} title={c.status} aria-hidden="true" />
        {time.status === 'stale' && <span className="cm__anchor cm__anchor--stale" title="Original clip was removed; showing the saved timeline time">Stale</span>}
        {draft && <span className="cm__drafted" title={`agent drafted ${draft.verbs?.length ?? 0} edit(s)`}><Wand /></span>}
        <span className="cm__text">{c.text}</span>
      </div>

      {selected && (
        <div className="cm__body" id={bodyId}>
          <div className="cm__meta">
            {c.author} · {c.status}
            {c.review_source && <span className="cm__source" data-cut-comment-source={c.review_source.render_id}> · External · {c.review_source.render_id}</span>}
            {draft?.backend?.provider && <span className="cm__meta-b"> · {draft.backend.provider}</span>}
          </div>

          {hasVerbs && (
            <div className="cm__draft">
              <div className="cm__draft-h">
                Proposed {draft!.verbs.length === 1 ? 'edit' : `${draft!.verbs.length} edits`}
                {draft!.confidence != null && <span className="cm__conf">{Math.round(draft!.confidence * 100)}%</span>}
              </div>
              <ul className="cm__verbs">
                {draft!.verbs.map((v, i) => (
                  <li key={i}>
                    <span className="cm__verb">{proposedEditLabel(v.verb, v.args)}</span>
                  </li>
                ))}
              </ul>
              {draft!.rationale && <div className="cm__rationale">{draft!.rationale}</div>}
              {invalid && (
                <div className="cm__invalid"><Icon name="warning" size={14} tone="warn" /> This proposal contains an unsupported edit and cannot be applied.</div>
              )}
            </div>
          )}
          {draft && !hasVerbs && (
            <div className="cm__draft cm__draft--noop">Agent found no actionable edit{draft.rationale ? ` — ${draft.rationale}` : ''}</div>
          )}

          <div className="cm__actions">
            <button
              className="cm__act cm__act--draft"
              data-cut-action="comment-draft"
              disabled={!!busy}
              title="Ask the agent to draft the edit for this comment"
              onClick={onDraft}
            >
              {busy === 'drafting' ? <Spinner /> : <Wand />} {draft ? 'Re-draft' : 'Draft'}
            </button>
            {hasVerbs && (
              <button
                className="cm__act cm__act--apply"
                data-cut-action="comment-apply"
                disabled={!!busy || invalid}
                title={invalid ? 'the draft has unrecognized verbs — re-draft' : 'Apply the drafted edit (one-click revertible via the auto-checkpoint)'}
                onClick={onApply}
              >
                {busy === 'applying' ? <Spinner /> : <Check />} Apply
              </button>
            )}
            <button
              className="cm__act cm__act--done"
              data-cut-action="comment-done"
              disabled={!!busy}
              title={c.status === 'addressed' ? 'Reopen this comment' : 'Mark this comment done'}
              onClick={onDone}
            >
              {c.status === 'addressed' ? <Reopen /> : <Check />} {c.status === 'addressed' ? 'Reopen' : 'Done'}
            </button>
            <button
              className="cm__act cm__act--dismiss"
              data-cut-action="comment-dismiss"
              title={c.status === 'dismissed' ? 'Reopen this comment' : 'Dismiss — won’t act on it'}
              onClick={onDismiss}
            >
              {c.status === 'dismissed' ? <Reopen /> : <Cross />} {c.status === 'dismissed' ? 'Reopen' : 'Dismiss'}
            </button>
          </div>
        </div>
      )}
    </div>
  )
}

// ── inline icons → normalized <Icon> (size 14, the system's smallest tier).
// Named wrappers keep the call sites (<Chevron/> …) unchanged; intent maps:
// collapse=chevronLeft, submit=return, agent-draft=autopilot, apply=check,
// dismiss=close, reopen=rotateCw, busy=spinner (keeps the cm__spin animation).
const Chevron = () => <Icon name="chevronLeft" size={14} />
const Enter = () => <Icon name="return" size={14} />
const Wand = () => <Icon name="autopilot" size={14} />
const Check = () => <Icon name="check" size={14} />
const Cross = () => <Icon name="close" size={14} />
const Reopen = () => <Icon name="rotateCw" size={14} />
const Spinner = () => <Icon name="spinner" size={14} className="cm__spin" />
