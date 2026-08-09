import { useCallback, useEffect, useRef, useState } from 'react'
import type { DragEvent as ReactDragEvent, MouseEvent as ReactMouseEvent, PointerEvent as ReactPointerEvent } from 'react'
import { ASSET_DRAG_MOVE } from './dnd'

export interface AssetCardDragItem {
  asset: string
  kind: string
  name: string
}

export interface AssetCardDragGhost {
  x: number
  y: number
  name: string
  kind: string
}

interface ActiveDrag<T> {
  item: T
  input: 'pointer' | 'mouse'
  startX: number
  startY: number
  dragging: boolean
  pointerId?: number
  captureTarget?: Element
}

const INTERACTIVE_TARGET = 'button, input, select, textarea, a'
const DRAG_THRESHOLD_PX = 5

export function useAssetCardDrag<T extends AssetCardDragItem>(
  onDrop: (item: T, clientX: number, clientY: number, alt: boolean) => void,
) {
  const active = useRef<ActiveDrag<T> | null>(null)
  const onDropRef = useRef(onDrop)
  const [ghost, setGhost] = useState<AssetCardDragGhost | null>(null)
  onDropRef.current = onDrop

  const finish = useCallback((clientX: number, clientY: number, alt: boolean, drop: boolean) => {
    const drag = active.current
    active.current = null
    setGhost(null)
    if (drag?.captureTarget && drag.pointerId != null) {
      try {
        if (drag.captureTarget.hasPointerCapture?.(drag.pointerId)) {
          drag.captureTarget.releasePointerCapture?.(drag.pointerId)
        }
      } catch {
        // Pointer capture may already have been released by the WebView.
      }
    }
    if (drop && drag?.dragging) onDropRef.current(drag.item, clientX, clientY, alt)
  }, [])

  const move = useCallback((clientX: number, clientY: number) => {
    const drag = active.current
    if (!drag) return
    if (!drag.dragging) {
      if (Math.hypot(clientX - drag.startX, clientY - drag.startY) < DRAG_THRESHOLD_PX) return
      drag.dragging = true
    }
    setGhost({ x: clientX, y: clientY, name: drag.item.name, kind: drag.item.kind })
    document.dispatchEvent(new CustomEvent(ASSET_DRAG_MOVE, {
      detail: { asset: drag.item.asset, kind: drag.item.kind, clientX, clientY },
    }))
  }, [])

  useEffect(() => {
    const onPointerMove = (event: PointerEvent) => {
      const drag = active.current
      if (!drag || drag.input !== 'pointer' || drag.pointerId !== event.pointerId) return
      if (event.cancelable) event.preventDefault()
      move(event.clientX, event.clientY)
    }
    const onPointerUp = (event: PointerEvent) => {
      const drag = active.current
      if (!drag || drag.input !== 'pointer' || drag.pointerId !== event.pointerId) return
      finish(event.clientX, event.clientY, event.altKey, true)
    }
    const onPointerCancel = (event: PointerEvent) => {
      const drag = active.current
      if (!drag || drag.input !== 'pointer' || drag.pointerId !== event.pointerId) return
      finish(event.clientX, event.clientY, event.altKey, false)
    }
    const onMouseMove = (event: MouseEvent) => {
      if (active.current?.input !== 'mouse') return
      if (event.cancelable) event.preventDefault()
      move(event.clientX, event.clientY)
    }
    const onMouseUp = (event: MouseEvent) => {
      if (active.current?.input !== 'mouse') return
      finish(event.clientX, event.clientY, event.altKey, true)
    }
    window.addEventListener('pointermove', onPointerMove)
    window.addEventListener('pointerup', onPointerUp)
    window.addEventListener('pointercancel', onPointerCancel)
    window.addEventListener('mousemove', onMouseMove)
    window.addEventListener('mouseup', onMouseUp)
    return () => {
      window.removeEventListener('pointermove', onPointerMove)
      window.removeEventListener('pointerup', onPointerUp)
      window.removeEventListener('pointercancel', onPointerCancel)
      window.removeEventListener('mousemove', onMouseMove)
      window.removeEventListener('mouseup', onMouseUp)
      active.current = null
    }
  }, [finish, move])

  const startPointerDrag = useCallback((event: ReactPointerEvent, item: T) => {
    if (event.button !== 0 || (event.target as HTMLElement).closest(INTERACTIVE_TARGET)) return
    event.preventDefault()
    const captureTarget = event.currentTarget
    try {
      captureTarget.setPointerCapture?.(event.pointerId)
    } catch {
      // The window listeners remain a complete fallback if capture is refused.
    }
    active.current = {
      item,
      input: 'pointer',
      startX: event.clientX,
      startY: event.clientY,
      dragging: false,
      pointerId: event.pointerId,
      captureTarget,
    }
  }, [])

  const startMouseDrag = useCallback((event: ReactMouseEvent, item: T) => {
    if (active.current || event.button !== 0 || (event.target as HTMLElement).closest(INTERACTIVE_TARGET)) return
    event.preventDefault()
    active.current = {
      item,
      input: 'mouse',
      startX: event.clientX,
      startY: event.clientY,
      dragging: false,
    }
  }, [])

  const preventNativeDrag = useCallback((event: ReactDragEvent) => event.preventDefault(), [])

  return { ghost, startPointerDrag, startMouseDrag, preventNativeDrag }
}
