import { useEffect, useMemo, useState } from 'react'
import { callVerb, type MediaEvidenceHit, type Project } from '../../lib/client'
import { evidenceChatAttachments } from '../../lib/evidenceAttachments'
import { Icon } from '../../icons'
import { assetLabel, formatEvidenceTime, KIND_PRESENTATION, nearestActiveOccurrence, selectedEvidencePrompt } from './model'

interface SearchResultsProps {
  project: Project | null
  playheadMs: number
  hits: MediaEvidenceHit[]
  searchedQuery: string
  searching: boolean
  nextCursor: string | null
  indexId: string | null
  selected: Set<string>
  onToggleSelected: (evidenceId: string) => void
  onLoadMore: () => void
}

const matchLabel = (hit: MediaEvidenceHit) => hit.match === 'semantic' ? 'Visual match' : hit.match === 'exact' ? 'Exact words' : 'Text match'

export default function SearchResults({ project, playheadMs, hits, searchedQuery, searching, nextCursor, indexId, selected, onToggleSelected, onLoadMore }: SearchResultsProps) {
  const [active, setActive] = useState(0)
  const selectedHits = useMemo(() => hits.filter((hit) => selected.has(hit.evidence_id)), [hits, selected])
  useEffect(() => { setActive(0) }, [hits])

  const preview = (hit: MediaEvidenceHit) => {
    if (!hit.available) return
    document.dispatchEvent(new CustomEvent('cut:open-source-monitor', { detail: { asset: hit.asset_id, at_ms: Math.round(hit.anchor_ms) } }))
  }
  const timeline = (hit: MediaEvidenceHit) => {
    const occurrence = nearestActiveOccurrence(hit, project, playheadMs)
    if (occurrence) void callVerb('ui.playhead', { at_ms: Math.round(occurrence.timeline_start_ms) })
  }
  const askAgent = () => {
    if (!selectedHits.length) return
    document.dispatchEvent(new CustomEvent('cut:open-chat', {
      detail: {
        prompt: selectedEvidencePrompt(selectedHits, project),
        evidence: evidenceChatAttachments(selectedHits, indexId, project),
      },
    }))
  }

  if (!hits.length) return null
  return (
    <section className="mi-results" aria-label={`Results for ${searchedQuery}`} data-cut-intelligence-results>
      <header className="mi-results__header">
        <span>{hits.length} cited {hits.length === 1 ? 'moment' : 'moments'} for “{searchedQuery}”</span>
        {selectedHits.length > 0 && <button type="button" className="cd-btn cd-btn--sm cd-btn--ghost" onClick={askAgent} data-cut-intelligence-ask-agent><Icon name="agent" size={14} /> Ask Agent about {selectedHits.length}</button>}
      </header>
      <div className="mi-results__list" role="list">
        {hits.map((hit, index) => {
          const presentation = KIND_PRESENTATION[hit.kind]
          const occurrence = nearestActiveOccurrence(hit, project, playheadMs)
          const title = assetLabel(project, hit.asset_id)
          const checked = selected.has(hit.evidence_id)
          const timelineNote = occurrence ? `Timeline ${formatEvidenceTime(occurrence.timeline_start_ms)}` : hit.occurrence_count > 0 ? 'Used in another sequence' : 'Not on a timeline'
          return (
            <article
              key={hit.evidence_id}
              className={`mi-hit ${active === index ? 'mi-hit--active' : ''} ${checked ? 'mi-hit--selected' : ''}`}
              role="listitem" tabIndex={active === index ? 0 : -1}
              data-cut-intelligence-hit={hit.evidence_id}
              onFocus={() => setActive(index)} onDoubleClick={() => preview(hit)}
              onKeyDown={(event) => {
                if (event.key === 'ArrowDown' || event.key === 'ArrowUp') {
                  event.preventDefault()
                  const delta = event.key === 'ArrowDown' ? 1 : -1
                  const next = Math.max(0, Math.min(hits.length - 1, index + delta))
                  setActive(next)
                  document.querySelector<HTMLElement>(`[data-cut-intelligence-hit="${hits[next].evidence_id}"]`)?.focus()
                } else if (event.key === 'Enter') preview(hit)
                else if (event.key === ' ') { event.preventDefault(); onToggleSelected(hit.evidence_id) }
              }}
            >
              <label className="mi-hit__select" title="Select this cited moment">
                <input type="checkbox" checked={checked} onChange={() => onToggleSelected(hit.evidence_id)} aria-label={`Select ${title} at ${formatEvidenceTime(hit.anchor_ms)}`} data-cut-intelligence-select={hit.evidence_id} />
              </label>
              <span className="mi-hit__icon"><Icon name={presentation.icon} size={14} /></span>
              <div className="mi-hit__body">
                <div className="mi-hit__identity"><strong title={title}>{title}</strong><time>{formatEvidenceTime(hit.source_start_ms)}–{formatEvidenceTime(hit.source_end_ms)}</time></div>
                <p>{hit.speaker ? <b>{hit.speaker}: </b> : null}{hit.excerpt}</p>
                <div className="mi-hit__meta"><span>{presentation.label}</span><span>{matchLabel(hit)}</span><span>{timelineNote}{hit.occurrence_count > 1 ? ` · ${hit.occurrence_count} uses` : ''}</span>{!hit.available && <span className="mi-hit__offline">Source offline</span>}</div>
              </div>
              <div className="mi-hit__actions">
                <button type="button" className="cd-btn cd-btn--sm" onClick={() => preview(hit)} disabled={!hit.available} title={hit.available ? `Preview ${title} at source time` : 'Relink this source before previewing'} data-cut-intelligence-preview={hit.evidence_id}><Icon name="screenPlay" size={14} /> Preview</button>
                <button type="button" className="cd-btn cd-btn--sm cd-btn--ghost" onClick={() => timeline(hit)} disabled={!occurrence} title={occurrence ? 'Jump to the nearest use in this sequence' : timelineNote} data-cut-intelligence-timeline={hit.evidence_id}><Icon name="playhead" size={14} /> Timeline</button>
              </div>
            </article>
          )
        })}
      </div>
      {nextCursor && <button type="button" className="cd-btn cd-btn--sm mi-results__more" disabled={searching} onClick={onLoadMore} data-cut-intelligence-more>{searching ? 'Loading…' : 'Load more'}</button>}
    </section>
  )
}
