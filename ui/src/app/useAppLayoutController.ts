import { useCallback, useEffect, useRef, useState } from 'react'
import { useLayout } from '../layout/useLayout'
import { clamp } from './model'

/** pixel minimums for the transcript|preview split. */
const MIN_TRANSCRIPT_PX = 320
const MIN_PREVIEW_PX = 480
const MIN_TIMELINE_PX = 160

export function useAppLayoutController(selectedClipIds: string[]) {
  const [layout, setLayout] = useLayout()
  void selectedClipIds

  const middleRef = useRef<HTMLDivElement>(null)
  const mainRef = useRef<HTMLDivElement>(null)
  const splitRef = useRef<HTMLDivElement>(null)

  const dragSplit = useCallback(
    (clientX: number) => {
      const r = splitRef.current?.getBoundingClientRect()
      if (!r || r.width <= 0) return
      const hi = Math.max(MIN_TRANSCRIPT_PX, r.width - MIN_PREVIEW_PX - 1)
      const px = clamp(clientX - r.left, MIN_TRANSCRIPT_PX, hi)
      setLayout((l) => ({ ...l, txFrac: px / r.width }))
    },
    [setLayout],
  )

  const [splitW, setSplitW] = useState(0)
  useEffect(() => {
    const el = splitRef.current
    if (!el) return
    let frame = 0
    const commitWidth = (width: number) => {
      cancelAnimationFrame(frame)
      frame = requestAnimationFrame(() => {
        setSplitW((current) => Math.abs(current - width) < 0.5 ? current : width)
      })
    }
    const ro = new ResizeObserver((entries) => commitWidth(entries[0].contentRect.width))
    ro.observe(el)
    commitWidth(el.getBoundingClientRect().width)
    return () => {
      cancelAnimationFrame(frame)
      ro.disconnect()
    }
  }, [])

  const txWidth =
    splitW > 0
      ? `${clamp(layout.txFrac * splitW, MIN_TRANSCRIPT_PX, Math.max(MIN_TRANSCRIPT_PX, splitW - MIN_PREVIEW_PX))}px`
      : `${layout.txFrac * 100}%`

  const dragTimeline = useCallback(
    (_x: number, clientY: number) => {
      const r = mainRef.current?.getBoundingClientRect()
      if (!r || r.height <= 0) return
      const h = clamp(r.bottom - clientY, MIN_TIMELINE_PX, r.height * 0.6)
      setLayout((l) => ({ ...l, tlH: Math.round(h) }))
    },
    [setLayout],
  )

  const dragRail = useCallback(
    (clientX: number) => {
      const r = middleRef.current?.getBoundingClientRect()
      if (!r) return
      const w = clamp(r.right - clientX, 280, 480)
      setLayout((l) => ({ ...l, railW: Math.round(w) }))
    },
    [setLayout],
  )

  return {
    layout,
    setLayout,
    middleRef,
    mainRef,
    splitRef,
    txWidth,
    dragSplit,
    dragTimeline,
    dragRail,
  }
}
