import { useEffect, useMemo, useRef, useState } from 'react'
import { getWindowThumbs, type WindowThumbs } from '../../lib/client'
import { msToPx, pxToMs, RAIL_W, type LaidItem } from './layout'

/** Target on-screen px per windowed thumbnail, close to NLE thumbnail density. */
const WIN_THUMB_PX = 96
/** Quantize source windows to this grid so pan/zoom jitter reuses cache. */
const WIN_GRID_MS = 250
/** Windowed tile height in px; mirrors the server default. */
const WIN_H = 80
/** Below this px/sec the base strip is dense enough. */
const WIN_ACTIVATE_PXPS = 200

export interface TimelineFilmstrip {
  url: string
  assetDurMs: number
}

interface WindowedThumbRequest {
  asset: string
  t0: number
  t1: number
  count: number
}

export interface UseWindowedThumbnailsArgs {
  allItems: LaidItem[]
  filmstrips: Map<string, TimelineFilmstrip>
  zoom: number
  viewW: number
  scrollX: number
}

export function useWindowedThumbnails({
  allItems,
  filmstrips,
  zoom,
  viewW,
  scrollX,
}: UseWindowedThumbnailsArgs): Map<string, WindowThumbs> {
  const windowReqs = useMemo(() => {
    const reqs = new Map<string, WindowedThumbRequest>()
    const pxPerSec = msToPx(1000, zoom)
    if (pxPerSec <= WIN_ACTIVATE_PXPS) return reqs
    const laneW = Math.max(200, viewW - RAIL_W)
    const viewLeftMs = pxToMs(scrollX, zoom)
    const viewRightMs = pxToMs(scrollX + laneW, zoom)
    for (const it of allItems) {
      if (it.kind !== 'video' || it.isImage || !it.asset) continue
      if (it.srcInMs === undefined || it.srcOutMs === undefined) continue
      if (!filmstrips.has(it.asset)) continue
      const clipEnd = it.startMs + it.durMs
      const vsTl = Math.max(it.startMs, viewLeftMs)
      const veTl = Math.min(clipEnd, viewRightMs)
      if (veTl - vsTl < 1) continue
      const ratio = (it.srcOutMs - it.srcInMs) / Math.max(1, it.durMs)
      let s0 = it.srcInMs + (vsTl - it.startMs) * ratio
      let s1 = it.srcInMs + (veTl - it.startMs) * ratio
      s0 = Math.max(it.srcInMs, Math.floor(s0 / WIN_GRID_MS) * WIN_GRID_MS)
      s1 = Math.min(it.srcOutMs, Math.ceil(s1 / WIN_GRID_MS) * WIN_GRID_MS)
      if (s1 - s0 < 1) continue
      const onScreenPx = msToPx((s1 - s0) / ratio, zoom)
      const count = Math.min(160, Math.max(12, Math.round(onScreenPx / WIN_THUMB_PX / 12) * 12))
      reqs.set(it.id, { asset: it.asset, t0: Math.round(s0), t1: Math.round(s1), count })
    }
    return reqs
  }, [allItems, filmstrips, zoom, viewW, scrollX])

  const [windowedTiles, setWindowedTiles] = useState<Map<string, WindowThumbs>>(new Map())
  const winReqKey = useRef(new Map<string, string>())

  useEffect(() => {
    const want = new Map<string, string>()
    for (const [clipId, r] of windowReqs) want.set(clipId, `${r.asset}@${r.t0}-${r.t1}#${r.count}`)
    winReqKey.current = want
    setWindowedTiles((prev) => {
      let changed = false
      const next = new Map(prev)
      for (const id of prev.keys()) {
        if (!want.has(id)) {
          next.delete(id)
          changed = true
        }
      }
      return changed ? next : prev
    })
    let live = true
    for (const [clipId, r] of windowReqs) {
      const key = want.get(clipId)
      void getWindowThumbs(r.asset, r.t0, r.t1, r.count, WIN_H).then((tile) => {
        if (!live || !tile) return
        if (winReqKey.current.get(clipId) !== key) return
        setWindowedTiles((prev) => {
          const next = new Map(prev)
          next.set(clipId, tile)
          return next
        })
      })
    }
    return () => {
      live = false
    }
  }, [windowReqs])

  return windowedTiles
}
