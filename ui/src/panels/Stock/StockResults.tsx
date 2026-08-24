// Result rows for Find media. Kept separate so the panel index owns requests
// and the durable coordinator while this component owns stable credit/import UI.

import { Icon } from '../../icons'
import { stockImportKey } from './importCoordinator'
import { type ProviderHit, providerLabel, safeExternalUrl } from './providerCatalog'

interface StockResultsProps {
  hits: ProviderHit[]
  fetchingId: string | null
  fetched: Readonly<Record<string, string>>
  onFetch: (hit: ProviderHit) => void
}

const formatDuration = (ms: number | null) => (ms && ms > 0 ? `${(ms / 1000).toFixed(1)}s` : '')

export function StockResults({ hits, fetchingId, fetched, onFetch }: StockResultsProps) {
  if (hits.length === 0) return null
  const importing = fetchingId !== null
  return (
    <div className="cd-stock-list" data-cut-stock-results aria-busy={importing}>
      {hits.map((hit) => {
        const licenseUrl = safeExternalUrl(hit.licenseUrl)
        const sourceUrl = safeExternalUrl(hit.sourceUrl)
        const importingThisHit = fetchingId === hit.id
        const fetchedAssetId = fetched[stockImportKey(hit.provider, hit.id)]
        const importDisabled = importing || !!fetchedAssetId
        const importLabel = fetchedAssetId
          ? `${hit.title} is already added`
          : importingThisHit
            ? `Importing ${hit.title}`
            : importing
              ? `Import ${hit.title} unavailable while another media item is importing`
              : `Import ${hit.title}`
        return <div className="cd-stock-hit" data-cut-stock-hit={hit.id} data-cut-stock-hit-provider={hit.provider} key={`${hit.provider}:${hit.id}`}>
          <div className="cd-stock-hit-main">
            <div className="cd-stock-hit-title" title={hit.title}>{hit.title}</div>
            <div className="cd-stock-hit-meta">
              <span className="cd-tag" data-cut-stock-hit-provider-label>{providerLabel(hit.provider)}</span>
              <span className="cd-tag">{hit.kind}</span>
              {hit.filetype && <span className="cd-tag">{hit.filetype}</span>}
              {formatDuration(hit.durationMs) && <span className="cd-tag">{formatDuration(hit.durationMs)}</span>}
              {licenseUrl
                ? <a className="cd-tag" data-cut-stock-hit-license href={licenseUrl} target="_blank" rel="noreferrer">{hit.license.toUpperCase()}</a>
                : <span className="cd-tag" data-cut-stock-hit-license>{hit.license.toUpperCase()}</span>}
              {hit.requiresAttribution && <span className="cd-tag" data-cut-stock-hit-attribution-required>Attribution required</span>}
              {sourceUrl && <a className="cd-tag" data-cut-stock-hit-source href={sourceUrl} target="_blank" rel="noreferrer">Source</a>}
            </div>
            <div className="cd-stock-hit-attr" data-cut-stock-hit-attribution title={hit.attribution}>Credit: {hit.attribution}</div>
          </div>
          <button
            className="cd-btn cd-btn--sm"
            data-cut-stock-fetch={hit.id}
            disabled={importDisabled}
            aria-busy={importingThisHit || undefined}
            aria-describedby={importing ? 'cut-stock-import-status' : undefined}
            aria-label={importLabel}
            onClick={() => onFetch(hit)}
          >
            {fetchedAssetId ? <><Icon name="check" size={14} tone="success" /> Added</> : importingThisHit ? 'Importing…' : 'Import'}
          </button>
        </div>
      })}
    </div>
  )
}
