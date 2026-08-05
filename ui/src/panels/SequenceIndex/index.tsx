import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import {
  callVerb,
  type Project,
  type SequenceIndexResult,
  type SequenceIndexClipRow,
  type SequenceIndexMarkerRow,
} from '../../lib/client'
import { Icon } from '../../icons'
import { writeClipboardText } from '../../lib/clipboard'
import { sequenceIndexCsv } from './csv'
import './sequenceIndex.css'

export { sequenceIndexCsv } from './csv'

type IndexKind = 'all' | 'clip' | 'marker'
type TrackFilter = '' | 'video' | 'audio' | 'caption'
type StatusFilter = 'all' | 'issues' | 'offline' | 'gaps' | 'effects' | 'hidden' | 'locked' | 'muted'

export interface SequenceIndexProps {
  project: Project | null
  onProjectChanged: () => void | Promise<void>
}

function formatTime(ms: number): string {
  const totalSeconds = Math.max(0, ms) / 1000
  const minutes = Math.floor(totalSeconds / 60)
  const seconds = totalSeconds - minutes * 60
  return `${String(minutes).padStart(2, '0')}:${seconds.toFixed(3).padStart(6, '0')}`
}

function rowKey(row: SequenceIndexClipRow | SequenceIndexMarkerRow): string {
  return `${row.sequence_id}:${row.kind}:${row.id}:${row.at_ms}`
}

function clipBadges(row: SequenceIndexClipRow): Array<{ label: string; kind: string }> {
  const badges: Array<{ label: string; kind: string }> = []
  const effects = row.effects ?? []
  if (row.clip_kind === 'gap') badges.push({ label: 'Gap', kind: 'issue' })
  if (row.offline) badges.push({ label: 'Offline', kind: 'issue' })
  if (effects.length > 0) badges.push({
    label: effects.length === 1 ? effects[0].replaceAll('_', ' ') : `${effects.length} effects`,
    kind: 'effect',
  })
  if (row.track_visible === false) badges.push({ label: 'Hidden', kind: 'track' })
  if (row.track_locked === true) badges.push({ label: 'Locked', kind: 'track' })
  if (row.track_muted === true) badges.push({ label: 'Muted', kind: 'track' })
  return badges
}

export default function SequenceIndex({ project, onProjectChanged }: SequenceIndexProps) {
  const [q, setQ] = useState('')
  const [appliedQuery, setAppliedQuery] = useState('')
  const [kind, setKind] = useState<IndexKind>('all')
  const [sequence, setSequence] = useState('')
  const [trackKind, setTrackKind] = useState<TrackFilter>('')
  const [status, setStatus] = useState<StatusFilter>('all')
  const [result, setResult] = useState<SequenceIndexResult | null>(null)
  const [busy, setBusy] = useState(false)
  const [opening, setOpening] = useState<string | null>(null)
  const [copyState, setCopyState] = useState<'idle' | 'copied' | 'error'>('idle')
  const [err, setErr] = useState<string | null>(null)
  const loadGeneration = useRef(0)

  const sequenceOptions = useMemo(() => {
    if (!project) return []
    if (project.sequences && project.sequences.length > 0) {
      return project.sequences.map((item) => ({ id: item.id, name: item.name }))
    }
    return [{ id: project.active_sequence ?? 'seq1', name: 'Main' }]
  }, [project])
  const projectKey = `${project?.name ?? ''}:${project?.active_sequence ?? ''}:${sequenceOptions.map((item) => `${item.id}:${item.name}`).join('|')}`

  useEffect(() => {
    if (sequence && !sequenceOptions.some((item) => item.id === sequence)) setSequence('')
  }, [sequence, sequenceOptions])

  const load = useCallback(async () => {
    const generation = ++loadGeneration.current
    if (!project) {
      setResult(null)
      setBusy(false)
      setErr(null)
      return
    }
    setBusy(true)
    setErr(null)
    try {
      const response = await callVerb('project.sequence_index', {
        query: appliedQuery,
        kind,
        sequence: sequence || undefined,
        track_kind: trackKind || undefined,
        status: status === 'all' ? undefined : status,
        limit: 200,
      })
      if (generation !== loadGeneration.current) return
      if (response.ok && response.result) setResult(response.result)
      else {
        setResult(null)
        setErr(response.error?.message ?? 'Sequence Index search failed')
      }
    } catch {
      if (generation !== loadGeneration.current) return
      setResult(null)
      setErr('server unreachable')
    } finally {
      if (generation === loadGeneration.current) setBusy(false)
    }
  }, [appliedQuery, kind, project, sequence, status, trackKind])

  useEffect(() => {
    void load()
  }, [load, projectKey])

  const submit = () => {
    const next = q.trim()
    if (next === appliedQuery) void load()
    else setAppliedQuery(next)
  }

  const openRow = async (row: SequenceIndexClipRow | SequenceIndexMarkerRow) => {
    if (!project) return
    const key = rowKey(row)
    setOpening(key)
    setErr(null)
    try {
      if (row.sequence_id !== (project.active_sequence ?? 'seq1')) {
        const switched = await callVerb('project.sequence_switch', {
          id: row.sequence_id,
          rationale: 'human: open Sequence Index result',
        })
        if (!switched.ok) {
          setErr(switched.error?.message ?? 'Could not open that sequence')
          return
        }
        await onProjectChanged()
      }
      const jumped = await callVerb('ui.playhead', { at_ms: Math.round(row.at_ms) })
      if (!jumped.ok) setErr(jumped.error?.message ?? 'Could not move the playhead')
      else document.dispatchEvent(new CustomEvent('cut:focus-preview'))
    } catch {
      setErr('server unreachable')
    } finally {
      setOpening(null)
    }
  }

  const openSource = (row: SequenceIndexClipRow) => {
    if (!row.asset) return
    document.dispatchEvent(new CustomEvent('cut:open-source-monitor', {
      detail: { asset: row.asset, at_ms: Math.round(row.src_in_ms ?? 0) },
    }))
  }

  const copyCsv = async () => {
    if (!result || result.results.length === 0) return
    setCopyState('idle')
    try {
      await writeClipboardText(sequenceIndexCsv(result.results))
      setCopyState('copied')
    } catch {
      setCopyState('error')
    }
    window.setTimeout(() => setCopyState('idle'), 2200)
  }

  return (
    <section className="si" data-cut-sequence-index aria-label="Sequence Index">
      {!project ? (
        <div className="si__empty" data-cut-sequence-index-empty>Open a project to browse its sequences.</div>
      ) : (
        <>
          <div className="si__search">
            <input
              data-cut-sequence-index-query
              value={q}
              onChange={(event) => setQ(event.target.value)}
              onKeyDown={(event) => { if (event.key === 'Enter') submit() }}
              placeholder="Search clips and markers"
            />
            <button type="button" data-cut-sequence-index-search onClick={submit} disabled={busy} title="Search Sequence Index">
              <Icon name="search" size={14} label="Search" />
            </button>
          </div>

          <div className="si__toolbar">
            <div className="si__kinds" role="group" aria-label="Result kind">
              {(['all', 'clip', 'marker'] as IndexKind[]).map((value) => (
                <button
                  type="button"
                  key={value}
                  data-cut-sequence-index-kind={value}
                  aria-pressed={kind === value}
                  onClick={() => {
                    setKind(value)
                    if (value === 'marker') {
                      setStatus('all')
                      setTrackKind('')
                    }
                  }}
                >
                  {value === 'all' ? 'All' : value === 'clip' ? 'Clips' : 'Markers'}
                </button>
              ))}
            </div>

            <div className="si__filters">
              <select aria-label="Sequence" data-cut-sequence-index-sequence value={sequence} onChange={(event) => setSequence(event.target.value)}>
                <option value="">All sequences</option>
                {sequenceOptions.map((item) => <option key={item.id} value={item.id}>{item.name}</option>)}
              </select>
              <select
                aria-label="Track"
                data-cut-sequence-index-track
                value={trackKind}
                onChange={(event) => {
                  const next = event.target.value as TrackFilter
                  setTrackKind(next)
                  if (next && kind === 'marker') setKind('clip')
                }}
              >
                <option value="">All tracks</option>
                <option value="video">Video</option>
                <option value="audio">Audio</option>
                <option value="caption">Captions</option>
              </select>
              <select
                aria-label="Status"
                data-cut-sequence-index-status
                value={status}
                onChange={(event) => {
                  const next = event.target.value as StatusFilter
                  setStatus(next)
                  if (next !== 'all' && kind === 'marker') setKind('clip')
                }}
              >
                <option value="all">All status</option>
                <option value="issues">Issues</option>
                <option value="offline">Offline</option>
                <option value="gaps">Gaps</option>
                <option value="effects">With effects</option>
                <option value="hidden">Hidden tracks</option>
                <option value="locked">Locked tracks</option>
                <option value="muted">Muted tracks</option>
              </select>
            </div>
          </div>

          {err && <div className="si__error" data-cut-sequence-index-error role="alert">{err}</div>}
          {result && (
            <div className="si__summary" data-cut-sequence-index-summary>
              <span>{result.total} result{result.total === 1 ? '' : 's'}</span>
              <span>
                {result.clip_count} clip{result.clip_count === 1 ? '' : 's'} /{' '}
                {result.marker_count} marker{result.marker_count === 1 ? '' : 's'}
              </span>
              {result.issue_count > 0 && <span>{result.issue_count} issue{result.issue_count === 1 ? '' : 's'}</span>}
              {result.truncated && <span>first 200 shown</span>}
              <span className="si__copy-status" role="status">
                {copyState === 'copied' ? 'CSV copied' : copyState === 'error' ? 'Copy failed' : ''}
              </span>
              <button
                type="button"
                className="si__copy"
                data-cut-sequence-index-copy
                onClick={() => void copyCsv()}
                disabled={result.results.length === 0}
                title="Copy shown rows as CSV"
              >
                <Icon name="copy" size={14} label="Copy shown rows as CSV" />
              </button>
            </div>
          )}

          <div className="si__results" data-cut-sequence-index-results>
            {busy && !result && <div className="si__empty">Loading index...</div>}
            {!busy && result?.results.length === 0 && <div className="si__empty">No matching clips or markers.</div>}
            {result?.results.map((row) => {
              const key = rowKey(row)
              const clip = row.kind === 'clip' ? row : null
              const badges = clip ? clipBadges(clip) : []
              return (
                <article className="si__row" data-cut-sequence-index-row={key} data-cut-sequence-index-row-kind={row.kind} data-cut-sequence-index-issues={clip?.issues?.join(',') ?? ''} key={key}>
                  <div className="si__row-icon">
                    <Icon name={row.kind === 'marker' ? 'marker' : row.clip_kind === 'gap' ? 'warning' : row.track_kind === 'audio' ? 'audioClip' : row.clip_kind === 'caption' ? 'captions' : 'videoClip'} size={14} tone={clip?.issues?.length ? 'warn' : 'default'} />
                  </div>
                  <div className="si__row-main">
                    <strong title={row.label}>{row.label}</strong>
                    <span>{row.sequence_name} / {row.kind === 'clip' ? `${row.track_id} / ` : ''}{formatTime(row.at_ms)}</span>
                    {badges.length > 0 && (
                      <div className="si__badges" title={clip?.effects?.join(', ')}>
                        {badges.map((badge) => <span key={`${badge.kind}:${badge.label}`} data-kind={badge.kind}>{badge.label}</span>)}
                      </div>
                    )}
                    {row.kind === 'marker' && row.note && <p>{row.note}</p>}
                  </div>
                  <div className="si__row-actions">
                    {clip?.asset && (
                      <button type="button" data-cut-sequence-index-source={key} onClick={() => openSource(clip)} title="Open source clip">
                        <Icon name="screenPlay" size={14} label="Source" />
                      </button>
                    )}
                    <button type="button" data-cut-sequence-index-open={key} onClick={() => void openRow(row)} disabled={opening === key} title="Open sequence and move the playhead">
                      <Icon name="playhead" size={14} label="Open" />
                    </button>
                  </div>
                </article>
              )
            })}
          </div>
        </>
      )}
    </section>
  )
}
