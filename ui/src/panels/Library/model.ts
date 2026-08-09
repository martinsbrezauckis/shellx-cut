import type { LibItem } from '../../lib/client'

export type TypeFilter = 'all' | 'video' | 'audio' | 'image'
export type SortKey = 'added' | 'recent' | 'name' | 'uses'
export type ViewMode = 'grid' | 'list'
export type LibraryCollection = 'all' | 'recent' | 'favorites' | 'missing'

export const SORT_KEYS: SortKey[] = ['added', 'recent', 'name', 'uses']

export const TYPE_TABS: { key: TypeFilter; label: string }[] = [
  { key: 'all', label: 'All' },
  { key: 'video', label: 'Video' },
  { key: 'audio', label: 'Audio' },
  { key: 'image', label: 'Image' },
]

export function libraryDetailItem<T extends { id: string }>(
  visible: readonly T[],
  selected: readonly T[],
  activeId: string | null,
): T | null {
  if (selected.length === 1) return selected[0]
  if (selected.length > 1) return null
  return visible.find((item) => item.id === activeId) ?? null
}

export function sortKeyFromInput(value: string, fallback: SortKey): SortKey {
  for (const option of SORT_KEYS) {
    if (option === value) return option
  }
  return fallback
}

export function shortDur(ms?: number): string {
  if (!ms || ms <= 0) return ''
  if (ms < 60_000) return `${(ms / 1000).toFixed(1)}s`
  const m = Math.floor(ms / 60_000)
  const s = Math.round((ms % 60_000) / 1000)
  return `${m}:${String(s).padStart(2, '0')}`
}

export function posterSrc(it: LibItem): string | null {
  // media_ok:false = the resolved source is gone (computed by library.list);
  // a poster request could only 404, so callers render the kind glyph instead.
  if (it.media_ok === false) return null
  if (it.type === 'image' && it.blob) return `/api/library-blob/${it.blob}`
  return `/api/library-poster?id=${encodeURIComponent(it.id)}`
}
