// panels/Stock — the Find ▸ Find media surface for assets.providers/search/fetch.
//
// It reads the live provider catalog before enabling a search. That keeps the
// human picker aligned with the matching cutd's request vocabulary while the
// panel stays deliberately small: choose a source, choose its valid media kind,
// search, inspect the returned license/credit, and import into the open project.

import { useCallback, useEffect, useRef, useState, useSyncExternalStore } from 'react'
import { callVerb, type Project } from '../../lib/client'
import NativeFolderPicker from '../../components/NativeFolderPicker'
import {
  type AssetKind,
  type AssetProvider,
  type AssetProviderName,
  type ProviderHit,
  isAssetProviderName,
  normalizeProviderCatalog,
  normalizeProviderHits,
  preferredProvider,
  providerAllowsEmptyQuery,
  providerLabel,
  providerNeedsDirectory,
  providerQueryLabel,
  providerQueryPlaceholder,
} from './providerCatalog'
import {
  beginStockSearch,
  finishStockSearch,
  initialStockRequestState,
  invalidateStockRequests,
  isCurrentStockSearch,
} from './requestState'
import { stockImportKey, type StockImportCoordinator } from './importCoordinator'
import { StockResults } from './StockResults'
import '../drawer.css'

export interface StockDrawerProps {
  project: Project | null
  /** Monotonic App identity; prevents old-project hits being shown after a switch. */
  projectScope: number
  /** Owned by App so an in-flight import survives any Find/workspace remount. */
  importCoordinator: StockImportCoordinator
}

interface SearchSession { provider: AssetProviderName; dir: string | null }
const providerFailure = (prefix: string, error?: { code?: string; message?: string }) => `${prefix}: ${error?.code ?? 'failed'}: ${error?.message ?? 'request failed'}`

export default function StockDrawer({ project, projectScope, importCoordinator }: StockDrawerProps) {
  const [providers, setProviders] = useState<AssetProvider[]>([])
  const [provider, setProvider] = useState<AssetProviderName | null>(null)
  const [catalogState, setCatalogState] = useState<'loading' | 'ready' | 'error'>('loading')
  const [catalogError, setCatalogError] = useState<string | null>(null)
  const [kind, setKind] = useState<AssetKind>('audio')
  const [q, setQ] = useState('')
  const [dir, setDir] = useState('')
  const [hits, setHits] = useState<ProviderHit[]>([])
  const [searchScope, setSearchScope] = useState(projectScope)
  const [searchSession, setSearchSession] = useState<SearchSession | null>(null)
  const [searching, setSearching] = useState(false)
  const [err, setErr] = useState<string | null>(null)
  const [note, setNote] = useState<string | null>(null)
  const requestState = useRef(initialStockRequestState)
  const importSnapshot = useSyncExternalStore(
    importCoordinator.subscribe,
    importCoordinator.getSnapshot,
    importCoordinator.getSnapshot,
  )

  const loadProviders = useCallback(async () => {
    setCatalogState('loading')
    setCatalogError(null)
    try {
      const response = await callVerb('assets.providers', {})
      if (!response.ok) {
        setCatalogState('error')
        setCatalogError(providerFailure('Could not load media sources', response.error))
        return
      }
      const catalog = normalizeProviderCatalog((response.result as { providers?: unknown }).providers)
      const initial = preferredProvider(catalog)
      if (!initial) {
        setCatalogState('error')
        setCatalogError('No compatible media sources are available from this Cut server.')
        return
      }
      setProviders(catalog)
      setProvider((current) => catalog.some((item) => item.name === current) ? current : initial.name)
      setCatalogState('ready')
    } catch {
      setCatalogState('error')
      setCatalogError('Could not load media sources: server unreachable')
    }
  }, [])

  useEffect(() => { void loadProviders() }, [loadProviders])

  // A Find-media remount starts with local inputs empty, but the app-lifetime
  // coordinator retains the last safe, server-normalized hit list. Rehydrate it
  // after the live catalog returns so the active Import is still visible and
  // disabled instead of offering a fresh route around the in-flight request.
  useEffect(() => {
    const remembered = importSnapshot.search
    if (catalogState !== 'ready' || !remembered || !providers.some((item) => item.name === remembered.provider)) return
    setProvider(remembered.provider)
    setKind(remembered.kind)
    setDir(remembered.dir)
    setQ(remembered.q)
    setHits(remembered.hits)
    setSearchScope(projectScope)
    setSearchSession(remembered.session)
  }, [catalogState, importSnapshot.search, providers])

  // A project close/open may leave this component mounted during the App shell
  // transition. Do not render a prior project's local hit state for even one
  // frame; the coordinator has already cleared its durable cache and retained
  // any old request lock until that request settles.
  useEffect(() => {
    requestState.current = invalidateStockRequests(requestState.current)
    setSearching(false)
    setHits([])
    setSearchScope(projectScope)
    setSearchSession(null)
    setErr(null)
    setNote(null)
  }, [projectScope])

  const meta = provider ? providers.find((item) => item.name === provider) ?? null : null

  // A server controls each provider's valid kinds. Never send a stale kind after
  // changing source, even when an old server/catalog is unusual or partial.
  useEffect(() => {
    if (!meta) return
    if (!meta.kinds.includes(kind)) setKind(meta.kinds[0])
  }, [kind, meta])

  const clearSearch = () => {
    requestState.current = invalidateStockRequests(requestState.current)
    setSearching(false)
    setHits([])
    setSearchScope(projectScope)
    setSearchSession(null)
    importCoordinator.clearSearch(projectScope)
    setErr(null)
    setNote(null)
  }

  const chooseProvider = (value: string) => {
    // A source switch invalidates pending results. Do not make that available
    // while the server is already importing one of those results: it could
    // visually clear the busy state and admit a second fetch before the first
    // response settles.
    if (importSnapshot.request.fetchingId !== null) return
    if (!isAssetProviderName(value) || !providers.some((item) => item.name === value)) return
    setProvider(value)
    clearSearch()
  }

  const search = async () => {
    if (importSnapshot.request.fetchingId !== null) return
    if (!meta || catalogState !== 'ready') {
      setErr('Media sources are not ready yet.')
      return
    }
    if (meta.needsKey) {
      setErr(`${providerLabel(meta.name)} needs source setup before it can be searched here.`)
      return
    }
    const query = q.trim()
    const searchDir = dir.trim()
    if (providerNeedsDirectory(meta.name) && !searchDir) {
      setErr('Choose a media folder to search.')
      return
    }
    if (!providerAllowsEmptyQuery(meta.name) && !query) {
      setErr('Enter a search term.')
      return
    }
    const requestStateAfterStart = beginStockSearch(requestState.current)
    requestState.current = requestStateAfterStart
    const request = requestStateAfterStart.searchEpoch
    setSearching(requestStateAfterStart.searching)
    setErr(null)
    setNote(null)
    setHits([])
    setSearchScope(projectScope)
    setSearchSession(null)
    importCoordinator.clearSearch(projectScope)
    try {
      const args: Record<string, unknown> = { provider: meta.name, q: query, kind, limit: 16 }
      if (providerNeedsDirectory(meta.name)) args.dir = searchDir
      const response = await callVerb('assets.search', args as never)
      if (!isCurrentStockSearch(requestState.current, request)) return
      if (!response.ok) {
        setErr(providerFailure('Search failed', response.error))
        return
      }
      const list = normalizeProviderHits((response.result as { hits?: unknown }).hits, meta)
      const session = { provider: meta.name, dir: providerNeedsDirectory(meta.name) ? searchDir : null }
      setHits(list)
      setSearchScope(projectScope)
      setSearchSession(session)
      importCoordinator.rememberSearch(projectScope, { provider: meta.name, kind, dir: searchDir, q: query, session, hits: list })
      if (list.length === 0) setNote('No usable results from this source.')
    } catch {
      if (isCurrentStockSearch(requestState.current, request)) setErr('Search failed: server unreachable')
    } finally {
      const settled = finishStockSearch(requestState.current, request)
      if (settled !== requestState.current) {
        requestState.current = settled
        setSearching(settled.searching)
      }
    }
  }

  const fetchHit = async (hit: ProviderHit) => {
    if (!project) {
      setErr('Create or open a project first — imported media goes into that project.')
      return
    }
    const session = searchSession
    if (!session || session.provider !== hit.provider) {
      setErr('Search this source again before importing a result.')
      return
    }
    if (providerNeedsDirectory(hit.provider) && !session.dir) {
      setErr('The original local folder is unavailable. Search that folder again before importing.')
      return
    }
    const requestStateAfterStart = importCoordinator.begin(projectScope, hit.id)
    // Admission is App-owned and updates before the first await. A queued
    // handler — including one from a just-remounted Find media panel — cannot
    // dispatch a second assets.fetch before React repaints disabled controls.
    if (!requestStateAfterStart) return
    const request = requestStateAfterStart.fetchEpoch
    let completed: { key: string; assetId: string } | undefined
    setErr(null)
    setNote(null)
    try {
      const args: Record<string, string> = { provider: hit.provider, id: hit.id, kind: hit.kind }
      if (session.dir) args.dir = session.dir
      const response = await callVerb('assets.fetch', args as never)
      if (!importCoordinator.isActive(projectScope, request)) return
      if (response.ok) {
        const assetId = (response.result as { asset_id?: string }).asset_id ?? ''
        if (importCoordinator.isCurrentScope(projectScope)) {
          completed = { key: stockImportKey(hit.provider, hit.id), assetId }
          setNote(`Imported “${hit.title}” into Assets.`)
        }
      } else {
        if (importCoordinator.isCurrentScope(projectScope)) setErr(providerFailure('Import failed', response.error))
      }
    } catch {
      if (importCoordinator.isActive(projectScope, request) && importCoordinator.isCurrentScope(projectScope)) setErr('Import failed: server unreachable')
    } finally {
      importCoordinator.finish(projectScope, request, completed)
    }
  }

  const fetchingId = importSnapshot.request.fetchingId
  const importing = fetchingId !== null
  const visibleHits = searchScope === projectScope ? hits : []

  return (
    <section className="cd-embed" data-cut-stock data-cut-stock-open="true" data-cut-stock-embed aria-label="Find media">
      <div className="cd-body">
        <div className="cd-note" data-cut-stock-providers-status aria-live="polite">
          {catalogState === 'loading' && 'Loading available media sources…'}
          {catalogState === 'ready' && `${providers.length} media sources available.`}
          {catalogState === 'error' && catalogError}
        </div>

        {meta && catalogState === 'ready' && <>
          <label className="cd-field">
            <span className="cd-field-label">Source</span>
            <select className="cd-sel" data-cut-stock-provider value={provider ?? ''} disabled={importing} onChange={(event) => chooseProvider(event.target.value)}>
              {providers.map((item) => (
                <option key={item.name} value={item.name} data-cut-stock-provider-option={item.name}>
                  {providerLabel(item.name)}{item.network ? '' : ' · offline'}
                </option>
              ))}
            </select>
          </label>

          <p className="cd-note" data-cut-stock-provider-note data-cut-stock-provider-network={meta.network ? 'network' : 'offline'}>
            {meta.note} {meta.network ? 'This source is contacted when you search or import a result.' : 'This source works offline.'}
          </p>

          {meta.needsKey && <p className="cd-note cd-note--warn" data-cut-stock-provider-setup>
            This source needs setup before searching is available in Find media.
          </p>}

          <div className="cd-field">
            <span className="cd-field-label">Kind</span>
            <div className="cd-seg" role="tablist" data-cut-stock-kind>
              {meta.kinds.map((item) => (
                <button
                  key={item}
                  type="button"
                  role="tab"
                  aria-selected={kind === item}
                  className={`cd-seg-btn ${kind === item ? 'cd-seg-btn--on' : ''}`}
                  data-cut-stock-kind-opt={item}
                  disabled={importing}
                  onClick={() => setKind(item)}
                >{item}</button>
              ))}
            </div>
          </div>

          {providerNeedsDirectory(meta.name) && (
            <NativeFolderPicker kind="stock" label="Folder" dialogTitle="Choose media folder — ShellX Cut"
              value={dir} disabled={importing} onChooseStart={() => setErr(null)}
              onDesktopRequired={() => setNote('Open the desktop app to choose a local media folder.')}
              onSelected={(selected) => { clearSearch(); setDir(selected); setNote('Media folder selected.') }} />
          )}

          <label className="cd-field">
            <span className="cd-field-label">{providerQueryLabel(meta.name)}</span>
            <input className="cd-input" data-cut-stock-query autoFocus disabled={importing}
              placeholder={providerQueryPlaceholder(meta.name)} value={q}
              onChange={(event) => setQ(event.target.value)}
              onKeyDown={(event) => { if (event.key === 'Enter') void search() }} />
          </label>

          <button className="cd-btn cd-btn--primary" data-cut-stock-search disabled={searching || importing || meta.needsKey || (providerNeedsDirectory(meta.name) && !dir.trim())} onClick={() => void search()}>
            {searching ? 'Searching…' : meta.name === 'stickers' && !q.trim() ? 'Browse stickers' : 'Search'}
          </button>
        </>}

        {err && <div className="cd-err" data-cut-stock-error role="alert">{err}</div>}
        {note && <p className="cd-note" data-cut-stock-note>{note}</p>}
        <p className="cd-note" id="cut-stock-import-status" data-cut-stock-import-status role="status" aria-live="polite">
          {importing ? 'Importing one media item. Other imports are unavailable until it finishes.' : ''}
        </p>

        <StockResults
          hits={visibleHits}
          fetchingId={fetchingId}
          fetched={importSnapshot.fetched}
          onFetch={fetchHit}
        />

        {meta && <p className="cd-note">
          License and credit come from the selected source and are retained with each import.
        </p>}
      </div>
    </section>
  )
}
