import { useCallback, useEffect, useState, type MouseEvent } from 'react'
import type { Track } from '../../lib/client'
import type { LaidItem } from './layout'
import type { AssetPickMode, ClipMenuState } from './ClipContextMenu'
import {
  isFiniteTimelineContextPoint,
  resolveTimelineContextTarget,
  type TimelineSurfaceMenuState,
} from './TimelineSurfaceMenuModel'

interface UseTimelineContextMenusArgs {
  allItems: LaidItem[]
  tracks: Track[]
  selectedClipIds: string[]
  clientXToMs: (clientX: number) => number
  onSelect: (clipIds: string[]) => void
}

/** Delegated context-target resolver for the timeline. The DOM only supplies
 * ids; the model decides which one authoritative menu may own that target. */
export function useTimelineContextMenus({
  allItems,
  tracks,
  selectedClipIds,
  clientXToMs,
  onSelect,
}: UseTimelineContextMenusArgs) {
  const [clipMenu, setClipMenu] = useState<ClipMenuState | null>(null)
  const [surfaceMenu, setSurfaceMenu] = useState<TimelineSurfaceMenuState | null>(null)
  const [assetPick, setAssetPick] = useState<AssetPickMode | null>(null)

  const onTimelineContextMenu = useCallback((event: MouseEvent) => {
    if (!isFiniteTimelineContextPoint(event.clientX, event.clientY)) {
      event.preventDefault()
      return
    }
    const target = event.target instanceof HTMLElement ? event.target : null
    if (!target) return
    // A trim handle deliberately overhangs a neighbouring gap by a few pixels.
    // Right-clicking visible gap hatch must still select the gap, not the clip
    // whose resize affordance happens to be on top at that coordinate.
    const gapElement = target.closest('[data-cut-gap]')
      ?? document.elementsFromPoint(event.clientX, event.clientY)
        .find((node): node is HTMLElement => node instanceof HTMLElement && !!node.closest('[data-cut-gap]'))
        ?.closest<HTMLElement>('[data-cut-gap]')
    const gapId = gapElement?.getAttribute('data-cut-gap') ?? null
    const result = resolveTimelineContextTarget({
      itemId: gapId ? null : target.closest('[data-cut-clip]')?.getAttribute('data-cut-clip') ?? null,
      gapId,
      trackId: target.closest('[data-cut-track]')?.getAttribute('data-cut-track') ?? null,
      x: event.clientX,
      y: event.clientY,
      atMs: clientXToMs(event.clientX),
      items: allItems,
      tracks,
    })
    if (result.kind === 'none') return
    event.preventDefault()
    setSurfaceMenu(null)
    if (result.kind !== 'clip') {
      setClipMenu(null)
      setSurfaceMenu(result)
      return
    }
    // NLE convention: preserve a multiselection only when the exact clicked
    // item belongs to it; otherwise the menu owns a single exact clip target.
    if (!(selectedClipIds.length > 1 && selectedClipIds.includes(result.itemId))) onSelect([result.itemId])
    setClipMenu({ x: event.clientX, y: event.clientY, itemId: result.itemId, atMs: clientXToMs(event.clientX) })
  }, [allItems, clientXToMs, onSelect, selectedClipIds, tracks])

  useEffect(() => {
    if (!clipMenu) return
    const onKey = (event: KeyboardEvent) => { if (event.key === 'Escape') setClipMenu(null) }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [clipMenu])
  useEffect(() => { setAssetPick(null) }, [clipMenu])

  return {
    clipMenu,
    setClipMenu,
    surfaceMenu,
    setSurfaceMenu,
    assetPick,
    setAssetPick,
    onTimelineContextMenu,
  }
}
