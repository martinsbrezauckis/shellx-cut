// layout/Divider.tsx — draggable panel divider:
// "Dividers: 1px --hairline, 6px hit area, col-resize/row-resize cursor,
// hover → --border").
// Role: a 1px visible hairline whose ::before pseudo-element (styled in
// theme.css) extends the pointer hit zone to ~6px without adding layout
// width. Pointer-capture drag: reports raw clientX/clientY to the owner,
// which does the container-relative math + clamping (App.tsx). While
// dragging, a body class forces the resize cursor + kills text selection.
// Callers: App.tsx (three instances). Dependencies: react; styles in theme.css.

import type { PointerEvent as ReactPointerEvent } from 'react'

export interface DividerProps {
  /** 'v' = vertical line between columns (col-resize) · 'h' = horizontal. */
  orient: 'v' | 'h'
  /** Stable id → data-cut-divider (addressable DOM). */
  id: string
  /** Pointer position during drag (owner converts to a panel size). */
  onDrag: (clientX: number, clientY: number) => void
}

export default function Divider({ orient, id, onDrag }: DividerProps) {
  const onPointerDown = (e: ReactPointerEvent<HTMLDivElement>) => {
    if (e.button !== 0) return
    e.preventDefault()
    const el = e.currentTarget
    const bodyClass = orient === 'v' ? 'cut-dragging-col' : 'cut-dragging-row'
    el.setPointerCapture(e.pointerId)
    el.classList.add('divider--dragging')
    document.body.classList.add(bodyClass)
    const move = (ev: PointerEvent) => onDrag(ev.clientX, ev.clientY)
    const up = () => {
      el.removeEventListener('pointermove', move)
      el.removeEventListener('pointerup', up)
      el.removeEventListener('pointercancel', up)
      el.classList.remove('divider--dragging')
      document.body.classList.remove(bodyClass)
      // No explicit persist call: useLayout's debounced effect writes the
      // final sizes 250ms after the last state change.
    }
    el.addEventListener('pointermove', move)
    el.addEventListener('pointerup', up)
    el.addEventListener('pointercancel', up)
  }

  return (
    <div
      className={`divider divider--${orient}`}
      data-cut-divider={id}
      role="separator"
      aria-orientation={orient === 'v' ? 'vertical' : 'horizontal'}
      onPointerDown={onPointerDown}
    />
  )
}
