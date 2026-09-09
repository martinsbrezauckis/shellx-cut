// panels/Comments/index.tsx — timecoded review notes and Agent Chat handoff.
// A comment is a durable editing target. “Make changes” uses the normal Agent
// Chat conversation; it never opens a parallel Draft/Apply provider workflow.

import { useCallback, useEffect, useMemo, useState } from 'react'
import { callVerb, exportUrl, type Comment, type Project } from '../../lib/client'
import { createCommentChatTimelineTarget } from '../../lib/chatTimelineTarget'
import { resolveCommentTime, type ResolvedCommentTime } from '../../lib/commentAnchors'
import { isTauri, pickReviewFeedback } from '../../lib/tauri'
import { timecode } from '../Timeline/layout'
import { Icon } from '../../icons'
import { TemporalRange } from '../../components/TemporalNavigation'
import './comments.css'

const FILTERS = ['all', 'open', 'addressed', 'dismissed'] as const
type Filter = (typeof FILTERS)[number]
const FILTER_LABEL: Record<Filter, string> = { all: 'All', open: 'Open', addressed: 'Done', dismissed: 'Dismissed' }

interface Props {
  project: Project | null
  playheadMs: number
  selectedClipIds: string[]
  selectedRange: [number, number] | null
  onSeek: (ms: number) => void
  onCollapse: () => void
  focus?: { id: string; n: number } | null
}

export default function Comments({
  project,
  playheadMs,
  selectedClipIds,
  selectedRange,
  onSeek,
  onCollapse,
  focus,
}: Props) {
  const comments = useMemo(() => project?.comments ?? [], [project])
  const [filter, setFilter] = useState<Filter>('all')
  const [text, setText] = useState('')
  const [selected, setSelected] = useState<string | null>(null)
  const [handoffBusy, setHandoffBusy] = useState<'export' | 'import' | null>(null)
  const [reviewPackage, setReviewPackage] = useState<string | null>(null)
  const [note, setNote] = useState<string | null>(null)
  const flash = (message: string) => { setNote(message); window.setTimeout(() => setNote(null), 4500) }

  const counts = useMemo(() => ({
    all: comments.length,
    open: comments.filter((comment) => comment.status === 'open').length,
    addressed: comments.filter((comment) => comment.status === 'addressed').length,
    dismissed: comments.filter((comment) => comment.status === 'dismissed').length,
  }), [comments])
  const shown = useMemo(() => comments
    .filter((comment) => filter === 'all' || comment.status === filter)
    .map((comment) => ({
      comment,
      time: resolveCommentTime(project, comment),
      target: createCommentChatTimelineTarget({ project, comment, selectedClipIds, selectedRange }),
    }))
    .sort((a, b) => a.time.atMs - b.time.atMs), [comments, filter, project, selectedClipIds, selectedRange])

  const add = useCallback(async () => {
    const value = text.trim()
    if (!value || !project) return
    const response = await callVerb('comment.add', {
      at_ms: Math.max(0, Math.round(playheadMs)),
      text: value,
      author: 'me',
    })
    if (response.ok) setText('')
    else flash(`add: ${response.error?.code ?? 'failed'}`)
  }, [playheadMs, project, text])

  const resolve = useCallback(async (id: string, status: Comment['status']) => {
    const response = await callVerb('comment.resolve', { comment_id: id, status })
    if (!response.ok) flash(`resolve: ${response.error?.code ?? 'failed'}`)
  }, [])

  const exportReview = useCallback(async () => {
    if (!project || handoffBusy) return
    setHandoffBusy('export')
    const response = await callVerb('comment.export', {})
    setHandoffBusy(null)
    if (!response.ok) return flash(`export review: ${response.error?.message ?? response.error?.code ?? 'failed'}`)
    const path = (response.result as { path?: string } | undefined)?.path
    if (!path) return flash('export review: missing package path')
    setReviewPackage(exportUrl(path))
    flash('review package ready')
  }, [handoffBusy, project])

  const importReview = useCallback(async () => {
    if (!project || handoffBusy) return
    if (!isTauri()) return flash('feedback import is available in the desktop app')
    const path = await pickReviewFeedback()
    if (!path) return
    setHandoffBusy('import')
    const response = await callVerb('comment.import', { path })
    setHandoffBusy(null)
    if (!response.ok) return flash(`import feedback: ${response.error?.message ?? response.error?.code ?? 'failed'}`)
    const count = (response.result as { count?: number } | undefined)?.count ?? 0
    flash(`imported ${count} review ${count === 1 ? 'note' : 'notes'}`)
  }, [handoffBusy, project])

  const askAgent = useCallback((comment: Comment) => {
    const target = createCommentChatTimelineTarget({ project, comment, selectedClipIds, selectedRange })
    if (!target) return flash('open the current project before asking the agent')
    document.dispatchEvent(new CustomEvent('cut:open-chat', {
      detail: {
        prompt: `Make this change: ${comment.text}`,
        target,
        submit: true,
      },
    }))
  }, [project, selectedClipIds, selectedRange])

  useEffect(() => {
    if (!focus?.id) return
    setFilter('all')
    setSelected(focus.id)
    window.setTimeout(() => document.querySelector(`[data-cut-comment="${focus.id}"]`)?.scrollIntoView({ block: 'nearest' }), 0)
  }, [focus])

  useEffect(() => { setReviewPackage(null) }, [project?.name])

  return (
    <section className="panel cm" data-cut-panel="comments">
      <header className="cm__head">
        <span className="cm__title">Comments{comments.length > 0 && <span className="cm__count">{comments.length}</span>}</span>
        <div className="cm__head-actions">
          <button className="cm__icon" data-cut-action="comment-export-review" title="Export review package" disabled={!project || !!handoffBusy} onClick={() => void exportReview()} aria-label="Export review package">
            <Icon name={handoffBusy === 'export' ? 'spinner' : 'share'} size={14} />
          </button>
          <button className="cm__icon" data-cut-action="comment-import-feedback" title={isTauri() ? 'Import review feedback' : 'Import review feedback in the desktop app'} disabled={!project || !!handoffBusy} onClick={() => void importReview()} aria-label="Import review feedback">
            <Icon name={handoffBusy === 'import' ? 'spinner' : 'import'} size={14} />
          </button>
          <button className="cm__icon" data-cut-action="comments-collapse" title="Hide comments (Ctrl/Cmd+Shift+C)" onClick={onCollapse} aria-label="Hide comments"><Icon name="chevronLeft" size={14} /></button>
        </div>
      </header>

      {reviewPackage && <a className="cm__package" data-cut-review-package href={reviewPackage} target="_blank" rel="noreferrer"><Icon name="link" size={14} />Open review page</a>}

      <div className="cm__filters" role="tablist" aria-label="Comment status filter">
        {FILTERS.map((value) => <button key={value} role="tab" aria-selected={filter === value} className={`cm__filter ${filter === value ? 'cm__filter--on' : ''}`} data-cut-comment-filter={value} onClick={() => setFilter(value)}>{FILTER_LABEL[value]}{counts[value] > 0 && <span className="cm__fcount">{counts[value]}</span>}</button>)}
      </div>

      <div className="cm__add">
        <span className="cm__add-tc" title="The comment lands at the playhead">{timecode(playheadMs)}</span>
        <input className="cm__add-input" data-cut-comment-input placeholder={project ? 'Add a review note…' : 'Open a project first'} value={text} disabled={!project} onChange={(event) => setText(event.target.value)} onKeyDown={(event) => { if (event.key === 'Enter') void add() }} />
        <button className="cm__add-btn" data-cut-action="comment-add" disabled={!project || !text.trim()} title="Add comment at the playhead (Enter)" onClick={() => void add()} aria-label="Add comment"><Icon name="return" size={14} /></button>
      </div>
      {note && <div className="cm__note" data-cut-comment-note>{note}</div>}

      <div className="cm__list">
        {shown.length === 0 ? (
          <div className="cm__empty" data-cut-comment-empty><div className="cm__empty-t">No {filter === 'all' ? '' : `${FILTER_LABEL[filter].toLowerCase()} `}comments</div><div className="cm__empty-b">Seek to a moment and add a review note. Make changes sends its exact target to Agent Chat.</div></div>
        ) : shown.map(({ comment, time, target }) => (
          <CommentRow key={comment.id} comment={comment} time={time} targetLabel={target?.label ?? null} selected={selected === comment.id} onToggle={() => setSelected((current) => current === comment.id ? null : comment.id)} onSeek={() => onSeek(time.atMs)} onAskAgent={() => askAgent(comment)} onDone={() => void resolve(comment.id, comment.status === 'addressed' ? 'open' : 'addressed')} onDismiss={() => void resolve(comment.id, comment.status === 'dismissed' ? 'open' : 'dismissed')} />
        ))}
      </div>
    </section>
  )
}

interface RowProps {
  comment: Comment
  time: ResolvedCommentTime
  targetLabel: string | null
  selected: boolean
  onToggle: () => void
  onSeek: () => void
  onAskAgent: () => void
  onDone: () => void
  onDismiss: () => void
}

function CommentRow({ comment, time, targetLabel, selected, onToggle, onSeek, onAskAgent, onDone, onDismiss }: RowProps) {
  const bodyId = `comment-body-${comment.id}`
  const fallbackTargetLabel = time.endMs != null && time.endMs > time.atMs
    ? `Comment target: ${timecode(time.atMs)}–${timecode(time.endMs)}`
    : `Comment target: ${timecode(time.atMs)}`
  return (
    <div className={`cm__row cm__row--${comment.status} ${selected ? 'cm__row--sel' : ''}`} data-cut-comment={comment.id} data-cut-comment-status={comment.status} data-cut-comment-anchor={time.status}>
      <div className="cm__row-head" onClick={onToggle}>
        <button type="button" className="cm__row-caret" data-cut-comment-disclosure aria-label={selected ? 'Hide comment actions' : 'Show comment actions'} aria-expanded={selected} aria-controls={bodyId} onClick={(event) => { event.stopPropagation(); onToggle() }}><Icon name={selected ? 'chevronDown' : 'chevronRight'} size={14} /></button>
        <TemporalRange rangeMs={[time.atMs, time.endMs ?? time.atMs]} format={timecode} className="cm__tc" data={{ 'data-cut-action': 'comment-seek' }} title="Jump to this moment" onClick={(event) => event.stopPropagation()} onActivate={onSeek}>{timecode(time.atMs)}{time.endMs != null ? `–${timecode(time.endMs)}` : ''}</TemporalRange>
        <span className={`cm__dot cm__dot--${comment.status}`} title={comment.status} aria-hidden="true" />
        {time.status === 'stale' && <span className="cm__anchor cm__anchor--stale" title="Original clip was removed; showing the saved timeline time">Stale</span>}
        <span className="cm__text">{comment.text}</span>
      </div>
      {selected && <div className="cm__body" id={bodyId}>
        <div className="cm__meta">{comment.author} · {comment.status}{comment.review_source && <span className="cm__source" data-cut-comment-source={comment.review_source.render_id}> · External · {comment.review_source.render_id}</span>}</div>
        <div className="cm__target" data-cut-comment-target>{targetLabel ?? fallbackTargetLabel}</div>
        <div className="cm__actions">
          <button className="cm__act cm__act--draft" data-cut-action="comment-make-changes" title="Send this comment and its exact timeline target to Agent Chat" onClick={onAskAgent}><Icon name="agent" size={14} /> Make changes</button>
          <button className="cm__act cm__act--done" data-cut-action="comment-done" title={comment.status === 'addressed' ? 'Reopen this comment' : 'Mark this comment done'} onClick={onDone}><Icon name={comment.status === 'addressed' ? 'rotateCw' : 'check'} size={14} /> {comment.status === 'addressed' ? 'Reopen' : 'Done'}</button>
          <button className="cm__act cm__act--dismiss" data-cut-action="comment-dismiss" title={comment.status === 'dismissed' ? 'Reopen this comment' : 'Dismiss — won’t act on it'} onClick={onDismiss}><Icon name={comment.status === 'dismissed' ? 'rotateCw' : 'close'} size={14} /> {comment.status === 'dismissed' ? 'Reopen' : 'Dismiss'}</button>
        </div>
      </div>}
    </div>
  )
}
