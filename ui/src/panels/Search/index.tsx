// panels/Search — the visual-search "Find moment" workspace for
// media.index / media.search).
// Role: the Find tab's embedded workspace that lets a human
// INDEX a video clip's frames (SigLIP2 embeddings) and then SEARCH them by
// content ("the slide with the chart") — clicking a result jumps the playhead to
// that moment. Makes the agent-only visual-search verbs a real user feature.
//
// Requires the perception runtime (SigLIP2) — absent → an honest setup error
// (the verbs surface it). query_vector path is engine-only; this UI uses text.
//
// Callers: LeftPanel (mounted under Find). Deps: lib/client (verbs), ../drawer.css.

import { useEffect, useMemo, useState } from 'react'
import { callVerb, type Project } from '../../lib/client'
import { Icon } from '../../icons'
import { sourceTimelineOccurrences } from '../Timeline/layout'
import '../drawer.css'

export interface SearchDrawerProps {
  project: Project | null
  /** Current Program playhead, used to choose the nearest occurrence of a
   * source-relative visual-search hit when an asset is reused. */
  playheadMs?: number
}

interface Hit { asset: string; start_ms: number; end_ms: number; peak_ms: number; score: number }

function looksLikeVideoPath(path: string | undefined): boolean {
  return !!path && /\.(mp4|mov|m4v|mkv|webm|avi|mpg|mpeg|mts|m2ts)$/i.test(path)
}

/** Video assets in the project (kind video / has a video stream). */
function videoAssets(project: Project | null): { id: string; label: string }[] {
  if (!project) return []
  const out: { id: string; label: string }[] = []
  for (const [id, a] of Object.entries(project.assets ?? {})) {
    const probe = (a as { probe?: { kind?: string; has_video?: boolean } }).probe
    const path = (a as { path?: string }).path ?? id
    if (probe?.kind === 'video' || probe?.has_video || (!probe && looksLikeVideoPath(path))) {
      out.push({ id, label: path.split(/[\\/]/).pop() || id })
    }
  }
  return out
}

const fmt = (ms: number) => `${(ms / 1000).toFixed(1)}s`
const INDEX_FPS = 1

export default function SearchDrawer({ project, playheadMs = 0 }: SearchDrawerProps) {
  const assets = useMemo(() => videoAssets(project), [project])
  const [indexing, setIndexing] = useState<string | null>(null)
  const [indexed, setIndexed] = useState<Record<string, number>>({}) // asset → frame count
  const [q, setQ] = useState('')
  const [searching, setSearching] = useState(false)
  const [hits, setHits] = useState<Hit[]>([])
  const [err, setErr] = useState<string | null>(null)
  const [note, setNote] = useState<string | null>(null)
  const indexedAssetCount = Object.keys(indexed).length
  const assetKey = assets.map((a) => a.id).join(',')
  const resultRows = useMemo(() => hits.map((hit) => {
    const occurrences = sourceTimelineOccurrences(project, hit.asset, hit.peak_ms)
    const nearest = occurrences.reduce<(typeof occurrences)[number] | null>((best, occurrence) => (
      !best || Math.abs(occurrence.atMs - playheadMs) < Math.abs(best.atMs - playheadMs) ? occurrence : best
    ), null)
    return {
      hit,
      assetName: assets.find((asset) => asset.id === hit.asset)?.label ?? hit.asset,
      occurrences,
      nearest,
    }
  }), [assets, hits, playheadMs, project])

  useEffect(() => {
    let alive = true
    if (!project || assets.length === 0) {
      setIndexed({})
      return
    }
    void callVerb('media.index_status', {}).then((r) => {
      if (!alive || !r.ok || !r.result) return
      const statusMap: Record<string, number> = {}
      const known = new Set(assets.map((a) => a.id))
      for (const item of ((r.result as { assets?: Array<{ asset: string; indexed_frames?: number }> }).assets ?? [])) {
        if (known.has(item.asset)) statusMap[item.asset] = item.indexed_frames ?? 0
      }
      setIndexed(statusMap)
    })
    return () => { alive = false }
  }, [project, assetKey, assets])

  const index = async (assetId: string) => {
    setIndexing(assetId); setErr(null); setNote(null)
    try {
      const r = await callVerb('media.index', { asset: assetId, fps: INDEX_FPS })
      if (r.ok) {
        const n = (r.result as { indexed_frames?: number }).indexed_frames ?? 0
        setIndexed((m) => ({ ...m, [assetId]: n }))
        setNote(`Indexed ${n} frames — now searchable.`)
      } else setErr(`${r.error?.code ?? 'failed'}: ${r.error?.message ?? 'index failed'}`)
    } catch { setErr('server unreachable') }
    finally { setIndexing(null) }
  }

  const search = async () => {
    if (!q.trim()) { setErr('Type what to look for'); return }
    setSearching(true); setErr(null); setNote(null); setHits([])
    try {
      const r = await callVerb('media.search', { query: q.trim(), top_k: 8 })
      if (r.ok) {
        const list = (r.result as { hits?: Hit[] }).hits ?? []
        setHits(list)
        if (list.length === 0) setNote('No matching indexed moments.')
      } else {
        const message = r.error?.message ?? 'search failed'
        if (message.includes('no visual-search index')) setErr('Index at least one clip before searching.')
        else setErr(`${r.error?.code ?? 'failed'}: ${message}`)
      }
    } catch { setErr('server unreachable') }
    finally { setSearching(false) }
  }

  // Jump the playhead to a found moment (ui.playhead relays to this same client).
  const jump = (ms: number) => { void callVerb('ui.playhead', { at_ms: Math.round(ms) }) }
  const openSource = (hit: Hit) => {
    document.dispatchEvent(new CustomEvent('cut:open-source-monitor', {
      detail: { asset: hit.asset, at_ms: Math.round(hit.peak_ms) },
    }))
  }

  const body = (
        <div className="cd-body">
          {assets.length === 0 ? (
            <div className="cd-empty" data-cut-search-empty>Import a video first — then index it for content search.</div>
          ) : (
            <>
              <div className="cd-field">
                <span className="cd-field-label">Clips to index</span>
                <div className="cd-stock-list" data-cut-search-assets>
                  {assets.map((a) => (
                    <div className="cd-stock-hit" data-cut-search-asset={a.id} key={a.id}>
                      <div className="cd-stock-hit-main">
                        <div className="cd-stock-hit-title" title={a.label}>{a.label}</div>
                        <div className="cd-stock-hit-attr">{indexed[a.id] != null ? `indexed ${indexed[a.id]} frames` : 'not indexed'}</div>
                      </div>
                      <button className="cd-btn cd-btn--sm" data-cut-search-index={a.id}
                        disabled={indexing === a.id || indexed[a.id] != null}
                        title="Index one frame per second for content search"
                        onClick={() => void index(a.id)}>
                        {indexed[a.id] != null ? <><Icon name="check" size={14} tone="success" /> Indexed</> : indexing === a.id ? 'Indexing…' : 'Index'}
                      </button>
                    </div>
                  ))}
                </div>
              </div>

              <label className="cd-field">
                <span className="cd-field-label">Search for</span>
                <input className="cd-input" data-cut-search-query autoFocus placeholder="e.g. a person, the title card, a red scene"
                  value={q} onChange={(e) => setQ(e.target.value)} onKeyDown={(e) => { if (e.key === 'Enter') void search() }} />
              </label>
              <button
                className="cd-btn cd-btn--primary"
                data-cut-search-go
                disabled={searching}
                title={indexedAssetCount > 0 ? 'Search the indexed clips' : 'Search existing indexes; if none exist, index a clip first'}
                onClick={() => void search()}
              >
                {searching ? 'Searching…' : 'Search'}
              </button>

              {err && <div className="cd-err" data-cut-search-error role="alert">{err}</div>}
              {note && <p className="cd-note" data-cut-search-note>{note}</p>}

              {resultRows.length > 0 && (
                <div className="cd-stock-list" data-cut-search-results>
                  {resultRows.map(({ hit: h, assetName, occurrences, nearest }, i) => (
                    <div className="cd-stock-hit cd-search-hit" data-cut-search-hit={i} key={`${h.asset}:${h.peak_ms}:${i}`}>
                      <div className="cd-stock-hit-main">
                        <div className="cd-stock-hit-title" title={assetName}>{assetName} · {fmt(h.start_ms)} - {fmt(h.end_ms)}</div>
                        <div className="cd-stock-hit-attr">
                          source {fmt(h.peak_ms)} · {nearest ? `timeline ${fmt(nearest.atMs)}${occurrences.length > 1 ? ` (${occurrences.length} uses)` : ''}` : 'not on timeline'} · score {h.score.toFixed(3)}
                        </div>
                      </div>
                      <div className="cd-search-hit-actions">
                        {nearest && (
                          <button
                            type="button"
                            className="cd-btn cd-btn--sm cd-btn--ghost"
                            data-cut-search-jump={i}
                            title={`Jump to this source moment at ${fmt(nearest.atMs)} on the timeline`}
                            onClick={() => jump(nearest.atMs)}
                          >
                            <Icon name="playhead" size={14} /> Timeline
                          </button>
                        )}
                        <button
                          type="button"
                          className="cd-btn cd-btn--sm cd-btn--ghost"
                          data-cut-search-source={i}
                          title={`Open ${assetName} at source ${fmt(h.peak_ms)}`}
                          onClick={() => openSource(h)}
                        >
                          <Icon name="screenPlay" size={14} /> Source
                        </button>
                      </div>
                    </div>
                  ))}
                </div>
              )}
            </>
          )}
        </div>
  )

  return (
    <section className="cd-embed" data-cut-search data-cut-search-open="true" data-cut-search-embed aria-label="Find moment">
      {body}
    </section>
  )
}
