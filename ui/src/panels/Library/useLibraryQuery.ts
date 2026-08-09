import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { callVerb, type LibItem, type VerbArgs } from '../../lib/client'
import type { LibraryCollection, SortKey, TypeFilter } from './model'

export const LIBRARY_PAGE_SIZE = 100

export interface LibraryQueryOptions {
  active: boolean
  type: TypeFilter
  folder: string | null
  tag: string | null
  search: string
  sort: SortKey
  collection: LibraryCollection
}

export interface LibraryQueryState {
  items: LibItem[]
  folders: string[]
  tags: string[]
  loading: boolean
  error: string | null
  total: number
  offset: number
  limit: number
  nextOffset: number | null
  pageNumber: number
  pageCount: number
  qDebounced: string
  queryKey: string
  reload: () => void
  previousPage: () => void
  nextPage: () => void
}

/**
 * Owns the Library's bounded server query, stale-response guard, debounce, and
 * page reset semantics. Keeping this out of the controller makes pagination
 * behavior independently reviewable and prevents the workspace component from
 * growing another large state machine.
 */
export function useLibraryQuery({
  active,
  type,
  folder,
  tag,
  search,
  sort,
  collection,
}: LibraryQueryOptions): LibraryQueryState {
  const [qDebounced, setQDebounced] = useState('')
  const [page, setPage] = useState({ key: '', offset: 0 })
  const [reloadToken, setReloadToken] = useState(0)
  const [items, setItems] = useState<LibItem[]>([])
  const [folders, setFolders] = useState<string[]>([])
  const [tags, setTags] = useState<string[]>([])
  const [total, setTotal] = useState(0)
  const [limit, setLimit] = useState(LIBRARY_PAGE_SIZE)
  const [nextOffset, setNextOffset] = useState<number | null>(null)
  const [loading, setLoading] = useState(true)
  const [error, setError] = useState<string | null>(null)
  const requestSeq = useRef(0)

  useEffect(() => {
    const timer = setTimeout(() => setQDebounced(search.trim()), 220)
    return () => clearTimeout(timer)
  }, [search])

  const queryKey = useMemo(
    () => [
      collection,
      type,
      folder ?? '',
      tag ?? '',
      qDebounced,
      sort,
    ].join('\u0000'),
    [collection, type, folder, tag, qDebounced, sort],
  )
  const offset = page.key === queryKey ? page.offset : 0

  useEffect(() => {
    if (page.key !== queryKey) setPage({ key: queryKey, offset: 0 })
  }, [page.key, queryKey])

  useEffect(() => {
    if (!active) return
    const seq = ++requestSeq.current
    setLoading(true)
    setError(null)
    const args: VerbArgs['library.list'] = {
      sort,
      offset,
      limit: LIBRARY_PAGE_SIZE,
    }
    if (type !== 'all') args.type = type
    if (folder != null) args.folder = folder
    if (tag) args.tag = tag
    if (qDebounced) args.q = qDebounced
    if (collection === 'favorites' || collection === 'missing') {
      args.collection = collection
    }

    void callVerb('library.list', args).then((result) => {
      if (seq !== requestSeq.current) return
      if (!result.ok || !result.result) {
        setError(`${result.error?.code ?? 'failed'}: ${result.error?.message ?? 'library.list failed'}`)
        setLoading(false)
        return
      }
      const value = result.result
      // A removal can empty the last page. Move to the new last page and let the
      // next request populate it instead of leaving an apparently empty library.
      if (offset > 0 && value.items.length === 0 && value.total > 0) {
        const lastOffset = Math.floor((value.total - 1) / value.limit) * value.limit
        setPage({ key: queryKey, offset: lastOffset })
        return
      }
      setItems(value.items)
      setFolders(value.folders)
      setTags(value.tags ?? [])
      setTotal(value.total)
      setLimit(value.limit)
      setNextOffset(value.next_offset ?? null)
      setLoading(false)
      document.dispatchEvent(new CustomEvent('cut:library-changed'))
    }).catch((cause: unknown) => {
      if (seq !== requestSeq.current) return
      setError(`failed: ${cause instanceof Error ? cause.message : 'library.list failed'}`)
      setLoading(false)
    })
    return () => {
      if (requestSeq.current === seq) requestSeq.current += 1
    }
  }, [
    active,
    collection,
    folder,
    offset,
    qDebounced,
    queryKey,
    reloadToken,
    sort,
    tag,
    type,
  ])

  const reload = useCallback(() => setReloadToken((value) => value + 1), [])
  const previousPage = useCallback(() => {
    setPage((current) => ({
      key: queryKey,
      offset: Math.max(0, (current.key === queryKey ? current.offset : 0) - LIBRARY_PAGE_SIZE),
    }))
  }, [queryKey])
  const nextPage = useCallback(() => {
    if (nextOffset == null) return
    setPage({ key: queryKey, offset: nextOffset })
  }, [nextOffset, queryKey])
  const pageCount = Math.max(1, Math.ceil(total / Math.max(1, limit)))
  const pageNumber = total === 0 ? 1 : Math.floor(offset / Math.max(1, limit)) + 1

  return {
    items,
    folders,
    tags,
    loading,
    error,
    total,
    offset,
    limit,
    nextOffset,
    pageNumber,
    pageCount,
    qDebounced,
    queryKey,
    reload,
    previousPage,
    nextPage,
  }
}
