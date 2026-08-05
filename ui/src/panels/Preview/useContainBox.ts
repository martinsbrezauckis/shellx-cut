import { useLayoutEffect, useState, type RefObject } from 'react'

/**
 * Compute the letterboxed CONTAIN box (px) for the project aspect inside a
 * measured container. ResizeObserver keeps it correct on relayout.
 */
export function useContainBox(ref: RefObject<HTMLElement | null>, aspect: number): { w: number; h: number } {
  const [box, setBox] = useState<{ w: number; h: number }>({ w: 0, h: 0 })
  useLayoutEffect(() => {
    const el = ref.current
    if (!el) return
    let frame = 0
    const compute = () => {
      const cw = el.clientWidth
      const ch = el.clientHeight
      if (cw <= 0 || ch <= 0) return
      let w = cw
      let h = cw / aspect
      if (h > ch) {
        h = ch
        w = ch * aspect
      }
      const next = { w: Math.round(w), h: Math.round(h) }
      setBox((current) => current.w === next.w && current.h === next.h ? current : next)
    }
    compute()
    const ro = new ResizeObserver(() => {
      cancelAnimationFrame(frame)
      frame = requestAnimationFrame(compute)
    })
    ro.observe(el)
    return () => {
      cancelAnimationFrame(frame)
      ro.disconnect()
    }
  }, [ref, aspect])
  return box
}
