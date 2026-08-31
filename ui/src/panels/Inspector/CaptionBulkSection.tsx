import { useEffect, useMemo, useState, type ReactNode } from 'react'
import { TemporalRange, formatTemporalTime } from '../../components/TemporalNavigation'
import { callVerb, type Project } from '../../lib/client'
import './caption-bulk.css'

type CaptionBulkPreview = {
  project_revision: string
  preview_hash: string
  scope: { track: string; range_ms: [number, number] | null; summary: string }
  match_count: number
  affected_cue_count: number
  rows: CaptionBulkRow[]
  rows_limit: number
  omitted_rows: number
  can_apply: boolean
  timing_refresh: { available: boolean; reason?: string }
}

type CaptionBulkRow = {
  cue_id: string
  range_ms: [number, number]
  text: string
  replacement_text: string
  match_ranges_utf16: Array<[number, number]>
}

interface CaptionBulkSectionProps {
  project: Project | null
  onSeek: (atMs: number) => void
}

function requestId() {
  const suffix = typeof crypto?.randomUUID === 'function'
    ? crypto.randomUUID()
    : `${Date.now()}-${Math.random().toString(36).slice(2)}`
  return `caption-bulk-${suffix}`
}

function highlightedText(text: string, ranges: Array<[number, number]>) {
  let cursor = 0
  const parts: ReactNode[] = []
  ranges.forEach(([start, end], index) => {
    if (start > cursor) parts.push(text.slice(cursor, start))
    parts.push(<mark key={`${start}-${end}-${index}`}>{text.slice(start, end)}</mark>)
    cursor = end
  })
  if (cursor < text.length) parts.push(text.slice(cursor))
  return parts
}

/** Inline reviewed find-and-replace for a single caption track. */
export default function CaptionBulkSection({ project, onSeek }: CaptionBulkSectionProps) {
  const tracks = useMemo(
    () => (project?.tracks ?? []).filter((track) => track.kind === 'caption'),
    [project],
  )
  const [track, setTrack] = useState('')
  const [find, setFind] = useState('')
  const [replaceWith, setReplaceWith] = useState('')
  const [matchMode, setMatchMode] = useState<'contains' | 'whole_word'>('contains')
  const [caseSensitive, setCaseSensitive] = useState(false)
  const [rangeEnabled, setRangeEnabled] = useState(false)
  const [rangeStart, setRangeStart] = useState(0)
  const [rangeEnd, setRangeEnd] = useState(0)
  const [preview, setPreview] = useState<CaptionBulkPreview | null>(null)
  const [currentIndex, setCurrentIndex] = useState(0)
  const [refreshTiming, setRefreshTiming] = useState(false)
  const [busy, setBusy] = useState<'preview' | 'apply' | ''>('')
  const [note, setNote] = useState<string | null>(null)

  useEffect(() => {
    if (!tracks.some((candidate) => candidate.id === track)) {
      setTrack(tracks[0]?.id ?? '')
    }
  }, [tracks, track])

  useEffect(() => {
    setPreview(null)
    setCurrentIndex(0)
    setRefreshTiming(false)
  }, [track, find, replaceWith, matchMode, caseSensitive, rangeEnabled, rangeStart, rangeEnd])

  const scopeRange = rangeEnabled ? [Math.max(0, Math.round(rangeStart)), Math.max(0, Math.round(rangeEnd))] as [number, number] : undefined
  const current = preview?.rows[currentIndex] ?? null
  const canPreview = !!project && !!track && find.length > 0 && busy === ''
  const canApply = !!preview?.can_apply && busy === ''

  const previewMatches = async () => {
    if (!canPreview) return
    setBusy('preview')
    setNote(null)
    const result = await callVerb('captions.bulk_preview', {
      track,
      find,
      replace_with: replaceWith,
      match_mode: matchMode,
      case_sensitive: caseSensitive,
      ...(scopeRange ? { range_ms: scopeRange } : {}),
    })
    setBusy('')
    if (result.ok) {
      const next = result.result as CaptionBulkPreview
      setPreview(next)
      setCurrentIndex(0)
      setRefreshTiming(false)
      setNote(next.match_count === 0 ? 'No matching captions in this scope.' : null)
    } else {
      setNote(result.error?.message ?? result.error?.code ?? 'Could not preview caption replacements')
    }
  }

  const applyMatches = async () => {
    if (!preview || !canApply) return
    setBusy('apply')
    setNote(null)
    const result = await callVerb('captions.bulk_apply', {
      preview_hash: preview.preview_hash,
      refresh_timing: refreshTiming,
      request_id: requestId(),
      expected_revision: preview.project_revision,
      rationale: 'inspector: reviewed caption find and replace',
    })
    setBusy('')
    if (result.ok) {
      setPreview(null)
      setRefreshTiming(false)
      setNote(`Replaced ${preview.match_count} match${preview.match_count === 1 ? '' : 'es'} in one Undo step.`)
      document.dispatchEvent(new CustomEvent('cut:show-composed'))
    } else {
      // Preserve every input and the visible preview on refusal so the editor
      // can inspect the conflict and explicitly refresh it.
      setNote(result.error?.message ?? result.error?.code ?? 'Caption replacement was refused')
    }
  }

  const step = (delta: number) => {
    if (!preview?.rows.length) return
    const next = (currentIndex + delta + preview.rows.length) % preview.rows.length
    setCurrentIndex(next)
    onSeek(preview.rows[next].range_ms[0])
  }

  return (
    <section className="insp__group insp__caption-bulk" data-cut-inspector-group="caption-bulk">
      <div className="insp__group-title insp__group-title--sub">Find & replace captions</div>
      <div className="insp__row">
        <select
          className="insp__select"
          data-cut-caption-bulk-track
          value={track}
          disabled={!project || tracks.length === 0 || busy !== ''}
          title="Caption track to search"
          onChange={(event) => setTrack(event.target.value)}
        >
          {tracks.length === 0 && <option value="">No caption track</option>}
          {tracks.map((candidate) => <option key={candidate.id} value={candidate.id}>{candidate.id}</option>)}
        </select>
        <select
          className="insp__select"
          data-cut-caption-bulk-match-mode
          value={matchMode}
          disabled={!project || busy !== ''}
          title="How the search text matches a caption"
          onChange={(event) => setMatchMode(event.target.value as 'contains' | 'whole_word')}
        >
          <option value="contains">Contains</option>
          <option value="whole_word">Whole word</option>
        </select>
      </div>
      <div className="insp__field">
        <input
          className="insp__text"
          data-cut-caption-bulk-find
          value={find}
          disabled={!project || busy !== ''}
          placeholder="Find in captions…"
          onChange={(event) => setFind(event.target.value)}
          onKeyDown={(event) => { if (event.key === 'Enter') void previewMatches() }}
        />
      </div>
      <div className="insp__field">
        <input
          className="insp__text"
          data-cut-caption-bulk-replace
          value={replaceWith}
          disabled={!project || busy !== ''}
          placeholder="Replace with (empty removes text)"
          onChange={(event) => setReplaceWith(event.target.value)}
          onKeyDown={(event) => { if (event.key === 'Enter') void previewMatches() }}
        />
      </div>
      <div className="insp__row">
        <label className="insp__inline insp__caption-bulk-check">
          <input
            type="checkbox"
            data-cut-caption-bulk-case-sensitive
            checked={caseSensitive}
            disabled={!project || busy !== ''}
            onChange={(event) => setCaseSensitive(event.target.checked)}
          />
          Match case
        </label>
        <label className="insp__inline insp__caption-bulk-check">
          <input
            type="checkbox"
            data-cut-caption-bulk-range-enabled
            checked={rangeEnabled}
            disabled={!project || busy !== ''}
            onChange={(event) => setRangeEnabled(event.target.checked)}
          />
          Limit to timeline range
        </label>
      </div>
      {rangeEnabled && (
        <div className="insp__row" data-cut-caption-bulk-range>
          <label className="insp__label">Start
            <input type="number" min={0} className="insp__num" data-cut-caption-bulk-range-start value={rangeStart}
              disabled={!project || busy !== ''} onChange={(event) => setRangeStart(Number(event.target.value) || 0)} />
          </label>
          <label className="insp__label">End
            <input type="number" min={0} className="insp__num" data-cut-caption-bulk-range-end value={rangeEnd}
              disabled={!project || busy !== ''} onChange={(event) => setRangeEnd(Number(event.target.value) || 0)} />
          </label>
          <span className="insp__hint">ms</span>
        </div>
      )}
      <div className="insp__row">
        <button
          type="button"
          className="insp__btn"
          data-cut-action="caption-bulk-preview"
          disabled={!canPreview}
          title="Preview exact caption changes before they are applied"
          onClick={() => void previewMatches()}
        >{busy === 'preview' ? 'Previewing…' : 'Preview'}</button>
        <span className="insp__hint" data-cut-caption-bulk-scope>
          {preview?.scope.summary ?? (scopeRange ? `${track || 'Caption track'} · ${scopeRange[0]}–${scopeRange[1]} ms` : `${track || 'Caption track'} · entire timeline`)}
        </span>
      </div>
      {preview && (
        <div className="insp__caption-bulk-preview" data-cut-caption-bulk-preview>
          <div className="insp__row insp__caption-bulk-nav">
            <button type="button" className="insp__btn" data-cut-action="caption-bulk-previous"
              disabled={preview.rows.length < 2 || busy !== ''} onClick={() => step(-1)}>Previous</button>
            <span className="insp__hint" data-cut-caption-bulk-count>
              {preview.match_count} match{preview.match_count === 1 ? '' : 'es'} · {preview.affected_cue_count} cue{preview.affected_cue_count === 1 ? '' : 's'}
            </span>
            <button type="button" className="insp__btn" data-cut-action="caption-bulk-next"
              disabled={preview.rows.length < 2 || busy !== ''} onClick={() => step(1)}>Next</button>
          </div>
          {current && (
            <article className="insp__caption-bulk-hit" data-cut-caption-bulk-current-cue={current.cue_id}>
              <div className="insp__caption-bulk-hit-meta">
                Displayed cue {currentIndex + 1} of {preview.rows.length}
                <TemporalRange
                  rangeMs={current.range_ms}
                  className="insp__hint"
                  data={{ 'data-cut-caption-bulk-current-range': current.cue_id }}
                  title="Seek to this caption's exact timeline range"
                  onActivate={onSeek}
                />
              </div>
              <p className="insp__caption-bulk-before" data-cut-caption-bulk-highlight>{highlightedText(current.text, current.match_ranges_utf16)}</p>
              <p className="insp__caption-bulk-after" data-cut-caption-bulk-replacement-preview>→ {current.replacement_text || '∅ (remove matched text)'}</p>
            </article>
          )}
          {preview.omitted_rows > 0 && <p className="insp__hint" data-cut-caption-bulk-bounded>Showing the first {preview.rows_limit} of {preview.affected_cue_count} reviewed cues. Replace applies all reviewed matches.</p>}
          {preview.timing_refresh.available ? (
            <label className="insp__inline insp__caption-bulk-check" data-cut-caption-bulk-timing-available title={preview.timing_refresh.reason}>
              <input type="checkbox" data-cut-caption-bulk-refresh-timing checked={refreshTiming}
                disabled={busy !== ''} onChange={(event) => setRefreshTiming(event.target.checked)} />
              Refresh timing from exact transcript words
            </label>
          ) : (
            <p className="insp__hint" data-cut-caption-bulk-timing>{preview.timing_refresh.reason ?? 'Timing is preserved; exact transcript word evidence is unavailable.'}</p>
          )}
          <div className="insp__row">
            <button
              type="button"
              className="insp__btn insp__btn--primary"
              data-cut-action="caption-bulk-apply"
              disabled={!canApply}
              title={preview.can_apply ? `Replace ${preview.match_count} reviewed match${preview.match_count === 1 ? '' : 'es'} as one Undo step` : 'A complete changed-caption preview is required before replacing'}
              onClick={() => void applyMatches()}
            >{busy === 'apply' ? 'Replacing…' : `Replace ${preview.match_count}`}</button>
          </div>
        </div>
      )}
      {note && <p className="insp__hint" data-cut-caption-bulk-note>{note}</p>}
      {!tracks.length && <p className="insp__hint" data-cut-caption-bulk-empty>Create, generate, or import a caption track before using Find & Replace.</p>}
      {current && <span className="sr-only">Current caption begins at {formatTemporalTime(current.range_ms[0])}</span>}
    </section>
  )
}
