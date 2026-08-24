// App-owned continuity for one Find media import.
//
// Stock itself is intentionally mounted only for the active Find sub-surface.
// A fetch may nevertheless outlive that component while the user visits Find
// moment, Sequence, Library, or Record. App creates this coordinator once and
// passes it down; it is not a module-global singleton and therefore cannot
// leak work between app lifetimes.

import type { AssetKind, AssetProviderName, ProviderHit } from './providerCatalog'
import {
  beginStockFetch,
  finishStockFetch,
  initialStockRequestState,
  isCurrentStockFetch,
  type StockRequestState,
} from './requestState'

export interface StockSearchMemory {
  provider: AssetProviderName
  kind: AssetKind
  dir: string
  q: string
  session: { provider: AssetProviderName; dir: string | null }
  hits: ProviderHit[]
}

export interface StockImportSnapshot {
  /** App's monotonic project session; never derive identity from project name. */
  projectScope: number
  /** Scope that admitted the active request, retained until that request settles. */
  activeRequestScope: number | null
  request: StockRequestState
  fetched: Readonly<Record<string, string>>
  search: StockSearchMemory | null
}

export interface StockImportCoordinator {
  getSnapshot(): StockImportSnapshot
  subscribe(listener: () => void): () => void
  setProjectScope(scope: number): void
  begin(scope: number, id: string): StockRequestState | null
  finish(scope: number, epoch: number, completed?: { key: string; assetId: string }): StockImportSnapshot
  isActive(scope: number, epoch: number): boolean
  isCurrentScope(scope: number): boolean
  rememberSearch(scope: number, search: StockSearchMemory): void
  clearSearch(scope: number): void
}

export function stockImportKey(provider: AssetProviderName, id: string): string {
  return `${provider}:${id}`
}

export function createStockImportCoordinator(): StockImportCoordinator {
  let snapshot: StockImportSnapshot = {
    projectScope: 0,
    activeRequestScope: null,
    request: initialStockRequestState,
    fetched: Object.freeze({}),
    search: null,
  }
  const listeners = new Set<() => void>()
  const publish = (next: StockImportSnapshot) => {
    snapshot = next
    listeners.forEach((listener) => listener())
  }

  return {
    getSnapshot: () => snapshot,
    subscribe(listener) {
      listeners.add(listener)
      return () => listeners.delete(listener)
    },
    setProjectScope(scope) {
      if (scope === snapshot.projectScope) return
      // Keep a started request globally locked until it settles, but erase all
      // project-A presentation state before project B can render. Its eventual
      // completion may release the lock, never populate B's Added state.
      publish({
        ...snapshot,
        projectScope: scope,
        fetched: Object.freeze({}),
        search: null,
      })
    },
    begin(scope, id) {
      if (scope !== snapshot.projectScope) return null
      const request = beginStockFetch(snapshot.request, id)
      if (!request) return null
      publish({ ...snapshot, request, activeRequestScope: scope })
      return request
    },
    finish(scope, epoch, completed) {
      if (!isCurrentStockFetch(snapshot.request, epoch) || snapshot.activeRequestScope !== scope) return snapshot
      const request = finishStockFetch(snapshot.request, epoch)
      const fetched = completed && scope === snapshot.projectScope
        ? Object.freeze({ ...snapshot.fetched, [completed.key]: completed.assetId })
        : snapshot.fetched
      publish({ ...snapshot, request, activeRequestScope: null, fetched })
      return snapshot
    },
    isActive: (scope, epoch) => snapshot.activeRequestScope === scope && isCurrentStockFetch(snapshot.request, epoch),
    isCurrentScope: (scope) => snapshot.projectScope === scope,
    rememberSearch(scope, search) {
      if (scope !== snapshot.projectScope) return
      publish({ ...snapshot, search })
    },
    clearSearch(scope) {
      if (scope !== snapshot.projectScope) return
      if (snapshot.search !== null) publish({ ...snapshot, search: null })
    },
  }
}
