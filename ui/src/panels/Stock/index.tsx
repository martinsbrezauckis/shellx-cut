// panels/Stock — the "Find media" provider-search tab for
// assets.search / assets.fetch).
// Role: the Find ▸ Find media tab that lets a human
// SEARCH the pluggable asset PROVIDERS (Openverse Creative-Commons aggregator +
// a local folder) and FETCH a result straight into the open project as a normal
// asset — with the LICENSE + ATTRIBUTION surfaced before they import it. Makes
// the agent-only assets.* verbs a real user feature.
//
// TRUST STORY: every result shows its license + a ready-to-use attribution line;
// fetching records that credit on the import op (the verb does it). Relay-
// drivable: ui.open{panel:"stock"} opens the compatibility alias for this tab.
//
// Callers: LeftPanel (mounted under Find). Deps: lib/client (verbs), ../drawer.css.

import { useEffect, useState } from 'react'
import { callVerb, type Project } from '../../lib/client'
import { Icon } from '../../icons'
import '../drawer.css'

export interface StockDrawerProps {
  project: Project | null
}

type Provider = 'openverse' | 'local_folder'
type Kind = 'audio' | 'image' | 'video'

/** One normalized provider hit (assets.search result item). */
interface Hit {
  provider: string
  id: string
  title: string
  kind: string
  creator?: string | null
  license: string
  license_url?: string | null
  source_url?: string | null
  filetype?: string | null
  duration_ms?: number | null
  attribution: string
  requires_attribution: boolean
}

const PROVIDERS: { id: Provider; label: string; kinds: Kind[]; net: boolean }[] = [
  { id: 'openverse', label: 'Openverse (Creative Commons)', kinds: ['audio', 'image'], net: true },
  { id: 'local_folder', label: 'Local folder', kinds: ['audio', 'image', 'video'], net: false },
]

function providerFromInput(value: string, fallback: Provider): Provider {
  for (const provider of PROVIDERS) {
    if (provider.id === value) return provider.id
  }
  return fallback
}

export default function StockDrawer({ project }: StockDrawerProps) {
  const [provider, setProvider] = useState<Provider>('openverse')
  const [kind, setKind] = useState<Kind>('audio')
  const [q, setQ] = useState('')
  const [dir, setDir] = useState('') // local_folder only
  const [hits, setHits] = useState<Hit[]>([])
  const [searching, setSearching] = useState(false)
  const [fetchingId, setFetchingId] = useState<string | null>(null)
  const [fetched, setFetched] = useState<Record<string, string>>({}) // hit id → asset_id
  const [err, setErr] = useState<string | null>(null)
  const [note, setNote] = useState<string | null>(null)

  const meta = PROVIDERS.find((p) => p.id === provider)!

  // Keep the kind valid for the selected provider.
  useEffect(() => {
    if (!meta.kinds.includes(kind)) setKind(meta.kinds[0])
  }, [provider]) // eslint-disable-line react-hooks/exhaustive-deps

  const search = async () => {
    if (provider === 'local_folder' && !dir.trim()) { setErr('Enter a folder path to search'); return }
    if (provider === 'openverse' && !q.trim()) { setErr('Enter a search term'); return }
    setSearching(true); setErr(null); setNote(null); setHits([]); setFetched({})
    try {
      const args: Record<string, unknown> = { provider, q: q.trim(), kind, limit: 16 }
      if (provider === 'local_folder') args.dir = dir.trim()
      const r = await callVerb('assets.search', args as never)
      if (r.ok) {
        const list = (r.result as { hits?: Hit[] }).hits ?? []
        setHits(list)
        if (list.length === 0) setNote('No results.')
      } else {
        setErr(`${r.error?.code ?? 'failed'}: ${r.error?.message ?? 'search failed'}`)
      }
    } catch { setErr('server unreachable') }
    finally { setSearching(false) }
  }

  const fetchHit = async (h: Hit) => {
    if (!project) { setErr('Create or open a project first — fetched media imports into it.'); return }
    setFetchingId(h.id); setErr(null); setNote(null)
    try {
      const fetchArgs: Record<string, string> = { provider: h.provider, id: h.id, kind: h.kind }
      if (h.provider === 'local_folder') fetchArgs.dir = dir.trim()
      const r = await callVerb('assets.fetch', fetchArgs as never)
      if (r.ok) {
        const aid = (r.result as { asset_id?: string }).asset_id ?? ''
        setFetched((f) => ({ ...f, [h.id]: aid }))
        setNote(`Imported "${h.title}" → ${aid}. It's in the Assets tray.`)
      } else {
        setErr(`${r.error?.code ?? 'failed'}: ${r.error?.message ?? 'fetch failed'}`)
      }
    } catch { setErr('server unreachable') }
    finally { setFetchingId(null) }
  }

  const fmtDur = (ms?: number | null) => (ms && ms > 0 ? `${(ms / 1000).toFixed(1)}s` : '')

  const body = (
        <div className="cd-body">
          {/* provider */}
          <label className="cd-field">
            <span className="cd-field-label">Provider</span>
            <select className="cd-sel" data-cut-stock-provider value={provider} onChange={(e) => setProvider(providerFromInput(e.target.value, provider))}>
              {PROVIDERS.map((p) => <option key={p.id} value={p.id}>{p.label}</option>)}
            </select>
          </label>

          {/* kind */}
          <div className="cd-field">
            <span className="cd-field-label">Kind</span>
            <div className="cd-seg" role="tablist" data-cut-stock-kind>
              {meta.kinds.map((k) => (
                <button
                  key={k} role="tab" aria-selected={kind === k}
                  className={`cd-seg-btn ${kind === k ? 'cd-seg-btn--on' : ''}`}
                  data-cut-stock-kind-opt={k} onClick={() => setKind(k)}
                >{k}</button>
              ))}
            </div>
          </div>

          {provider === 'local_folder' && (
            <label className="cd-field">
              <span className="cd-field-label">Folder</span>
              <input className="cd-input cd-input--mono" data-cut-stock-dir type="text" spellCheck={false}
                placeholder="/path/to/your/media" value={dir} onChange={(e) => setDir(e.target.value)} />
            </label>
          )}

          {/* query */}
          <label className="cd-field">
            <span className="cd-field-label">{provider === 'local_folder' ? 'Filename contains' : 'Search'}</span>
            <input className="cd-input" data-cut-stock-query autoFocus
              placeholder={provider === 'local_folder' ? 'e.g. whoosh' : 'e.g. rain, applause, whoosh'}
              value={q} onChange={(e) => setQ(e.target.value)}
              onKeyDown={(e) => { if (e.key === 'Enter') void search() }} />
          </label>

          <button className="cd-btn cd-btn--primary" data-cut-stock-search disabled={searching} onClick={() => void search()}>
            {searching ? 'Searching…' : 'Search'}
          </button>

          {err && <div className="cd-err" data-cut-stock-error role="alert">{err}</div>}
          {note && <p className="cd-note" data-cut-stock-note>{note}</p>}

          {/* results */}
          {hits.length > 0 && (
            <div className="cd-stock-list" data-cut-stock-results>
              {hits.map((h) => (
                <div className="cd-stock-hit" data-cut-stock-hit={h.id} key={h.id}>
                  <div className="cd-stock-hit-main">
                    <div className="cd-stock-hit-title" title={h.title}>{h.title}</div>
                    <div className="cd-stock-hit-meta">
                      <span className="cd-tag">{h.kind}</span>
                      {h.filetype && <span className="cd-tag">{h.filetype}</span>}
                      {fmtDur(h.duration_ms) && <span className="cd-tag">{fmtDur(h.duration_ms)}</span>}
                      <span className="cd-tag" data-cut-stock-hit-license title={h.attribution}>
                        {h.license.toUpperCase()}{h.requires_attribution ? ' ⚠' : ''}
                      </span>
                    </div>
                    <div className="cd-stock-hit-attr" title={h.attribution}>{h.attribution}</div>
                  </div>
                  <button
                    className="cd-btn cd-btn--sm" data-cut-stock-fetch={h.id}
                    disabled={fetchingId === h.id || !!fetched[h.id]}
                    onClick={() => void fetchHit(h)}
                  >
                    {fetched[h.id] ? <><Icon name="check" size={14} tone="success" /> Added</> : fetchingId === h.id ? 'Fetching…' : 'Import'}
                  </button>
                </div>
              ))}
            </div>
          )}
          <p className="cd-note">
            Openverse results are commercial-use Creative Commons; license + credit are shown and recorded on import.
            {' '}<code>⚠</code> = attribution required.
          </p>
        </div>
  )

  return (
    <section className="cd-embed" data-cut-stock data-cut-stock-open="true" data-cut-stock-embed aria-label="Find media">
      {body}
    </section>
  )
}
