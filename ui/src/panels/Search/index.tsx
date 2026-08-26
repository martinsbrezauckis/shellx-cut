// Find > Moment — cited search across transcript, visual, perception, marker,
// and metadata evidence. The server owns the durable index and all search
// authority; this panel owns only filters, selection, and navigation intent.

import type { Project } from '../../lib/client'
import Coverage from './Coverage'
import SearchControls from './SearchControls'
import SearchResults from './SearchResults'
import { useMediaIntelligence } from './useMediaIntelligence'
import '../drawer.css'
import './search.css'

export interface SearchDrawerProps {
  project: Project | null
  playheadMs?: number
}

export default function SearchDrawer({ project, playheadMs = 0 }: SearchDrawerProps) {
  const search = useMediaIntelligence(project)
  const assetCount = Object.keys(project?.assets ?? {}).length
  const canSearch = Boolean(search.status?.index_id && !search.rebuild && !search.statusBusy)

  return (
    <section className="cd-embed mi-panel" data-cut-search data-cut-search-open="true" data-cut-search-embed data-cut-intelligence aria-label="Find moment">
      <div className="cd-body mi-panel__body">
        {!project ? (
          <div className="cd-empty" data-cut-intelligence-empty="project">Open or create a project to search its moments.</div>
        ) : assetCount === 0 ? (
          <div className="cd-empty" data-cut-intelligence-empty="media">Import media to find moments across this project.</div>
        ) : (
          <>
            <SearchControls
              query={search.query} mode={search.mode} scope={search.scope}
              searching={search.searching} canSearch={canSearch}
              onQuery={search.setQuery} onMode={search.setMode} onScope={search.setScope}
              onSearch={() => { void search.runSearch() }}
            />
            <Coverage
              status={search.status} busy={search.statusBusy} rebuild={search.rebuild}
              onPrepare={() => { void search.prepare() }}
              onCancel={() => { void search.cancelPrepare() }}
              onRefresh={() => { void search.refreshStatus() }}
            />
            {search.error && <div className="cd-err mi-message" role="alert" data-cut-intelligence-error>{search.error}</div>}
            {search.notice && <p className="cd-note mi-message" data-cut-intelligence-notice>{search.notice}</p>}
            <SearchResults
              project={project} playheadMs={playheadMs}
              hits={search.hits} searchedQuery={search.searchedQuery}
              searching={search.searching} nextCursor={search.nextCursor} indexId={search.resultIndexId}
              selected={search.selected} onToggleSelected={search.toggleSelected}
              onLoadMore={() => { if (search.nextCursor) void search.runSearch(search.nextCursor) }}
            />
          </>
        )}
      </div>
    </section>
  )
}
