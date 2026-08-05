import { useCallback, useEffect, useState, type KeyboardEvent } from 'react'

export type LibraryNavigationKey = 'ArrowUp' | 'ArrowDown' | 'Home' | 'End'

export function nextLibraryItemIndex(
  length: number,
  current: number,
  key: LibraryNavigationKey,
): number {
  if (length <= 0) return -1
  if (key === 'Home') return 0
  if (key === 'End') return length - 1
  if (key === 'ArrowUp') return Math.max(0, current - 1)
  return Math.min(length - 1, current + 1)
}

function focusLibraryItem(id: string): void {
  const item = Array.from(document.querySelectorAll<HTMLElement>('[data-cut-library-card]'))
    .find((element) => element.dataset.cutLibraryCard === id)
  item?.focus()
}

/**
 * Roving focus for list/grid item frames. Child buttons, inputs, and selects
 * keep their normal keyboard behavior; navigation keys are handled only while
 * the media frame itself owns focus.
 */
export function useLibraryKeyboardNavigation(itemIds: readonly string[]) {
  const [activeId, setActiveId] = useState<string | null>(itemIds[0] ?? null)
  const itemKey = itemIds.join('\u0000')

  useEffect(() => {
    if (activeId && itemIds.includes(activeId)) return
    const nextId = itemIds[0] ?? null
    setActiveId(nextId)
    if (nextId && document.activeElement === document.body) {
      requestAnimationFrame(() => focusLibraryItem(nextId))
    }
  }, [activeId, itemIds, itemKey])

  const onItemKeyDown = useCallback((
    id: string,
    event: KeyboardEvent<HTMLElement>,
  ) => {
    if (event.target !== event.currentTarget) return
    if (!['ArrowUp', 'ArrowDown', 'Home', 'End'].includes(event.key)) return
    const current = Math.max(0, itemIds.indexOf(id))
    const nextIndex = nextLibraryItemIndex(
      itemIds.length,
      current,
      event.key as LibraryNavigationKey,
    )
    const nextId = itemIds[nextIndex]
    if (!nextId) return
    event.preventDefault()
    event.stopPropagation()
    setActiveId(nextId)
    focusLibraryItem(nextId)
  }, [itemIds])

  return {
    activeId,
    tabIndexFor: (id: string) => id === (activeId ?? itemIds[0]) ? 0 : -1,
    onItemFocus: (id: string) => setActiveId(id),
    onItemKeyDown,
  }
}
