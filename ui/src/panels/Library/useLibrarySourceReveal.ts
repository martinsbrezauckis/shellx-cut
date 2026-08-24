// Exact asset reveal into the existing server-paged Library surface.

import { useEffect, useRef, type Dispatch, type SetStateAction } from 'react'
import type { SourceNavigationState } from '../../app/useSourceNavigationController'
import type { LibItem } from '../../lib/client'
import type { LibraryCollection, SortKey, TypeFilter } from './model'

interface LibrarySourceRevealInput {
  sourceNavigation?: SourceNavigationState | null
  revealedLibraryId: string | null
  loading: boolean
  queryError: string | null
  loadedQueryKey: string | null
  queryKey: string
  visibleItems: LibItem[]
  flash: (message: string) => void
  setType: Dispatch<SetStateAction<TypeFilter>>
  setFolder: Dispatch<SetStateAction<string | null>>
  setTagFilter: Dispatch<SetStateAction<string | null>>
  setQ: Dispatch<SetStateAction<string>>
  setSort: Dispatch<SetStateAction<SortKey>>
  setCollection: Dispatch<SetStateAction<LibraryCollection>>
  setSelected: Dispatch<SetStateAction<Set<string>>>
  setAnchorId: Dispatch<SetStateAction<string | null>>
}

export function useLibrarySourceReveal({
  sourceNavigation,
  revealedLibraryId,
  loading,
  queryError,
  loadedQueryKey,
  queryKey,
  visibleItems,
  flash,
  setType,
  setFolder,
  setTagFilter,
  setQ,
  setSort,
  setCollection,
  setSelected,
  setAnchorId,
}: LibrarySourceRevealInput) {
  const handled = useRef<number | null>(null)

  useEffect(() => {
    if (!sourceNavigation || sourceNavigation.destination !== 'library') return
    setType('all'); setFolder(null); setTagFilter(null); setQ(''); setSort('added'); setCollection('all')
    setSelected(new Set())
    setAnchorId(null)
  }, [setAnchorId, setCollection, setFolder, setQ, setSelected, setSort, setTagFilter, setType, sourceNavigation])

  useEffect(() => {
    if (!sourceNavigation || sourceNavigation.destination !== 'library') return
    if (handled.current === sourceNavigation.nonce) return
    if (!revealedLibraryId) {
      handled.current = sourceNavigation.nonce
      flash('This project source is not registered in the Library')
      return
    }
    if (loading || queryError || loadedQueryKey !== queryKey) return
    handled.current = sourceNavigation.nonce
    const item = visibleItems.find((candidate) => candidate.id === revealedLibraryId)
    if (!item) {
      flash('This project source is not registered in the Library')
      return
    }
    setSelected(new Set([item.id]))
    setAnchorId(item.id)
    flash('Revealed the registered source in Library')
    const frame = window.requestAnimationFrame(() => {
      const card = Array.from(document.querySelectorAll<HTMLElement>('[data-cut-library-card]'))
        .find((element) => element.dataset.cutLibraryCard === item.id)
      card?.scrollIntoView({ block: 'nearest', behavior: 'smooth' })
      card?.focus({ preventScroll: true })
    })
    return () => window.cancelAnimationFrame(frame)
  }, [flash, loadedQueryKey, loading, queryError, queryKey, revealedLibraryId, setAnchorId, setSelected, sourceNavigation, visibleItems])
}
