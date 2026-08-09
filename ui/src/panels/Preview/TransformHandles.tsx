// panels/Preview/TransformHandles — on-canvas DIRECT-MANIPULATION handles for an
// overlay clip's transform (edit.transform). Click the overlay to show a bounding
// box with corner handles; drag the body to move,
// drag a CORNER to scale (uniform — the engine has one `scale`, so corner-scale is
// always proportional; we deliberately do NOT ship an aspect-lock toggle that would
// do nothing). This uses PointerEvents so normalized state remains the single
// source of truth and two-way-syncs with the Layer sliders.
//
// Coordinate mapping is direct here: the `.pv-stage` is the normalized space —
// overlays already position at left=x·100% / top=y·100% / size=scale·100% of the
// stage (overlayBoxStyle). So a pixel drag → normalized delta = Δpx / stageRect.dim.
// transform is TOP-LEFT-origin fractions (ox = W·x; NO factor of 2). Round late.
//
// Live feedback: onLive fires every move (Preview moves the actual overlay <video>
// with the box); onCommit fires once on pointer-up (edit.transform) — matching the
// timeline doctrine of NO optimistic commit mid-drag.
//
// Callers: Preview/index.tsx (one per selected, currently-visible overlay).
// Deps: lib/client (ClipTransform).

import { useRef, type PointerEvent as RPointerEvent, type RefObject } from 'react'
import type { ClipTransform } from '../../lib/client'

type T = Required<ClipTransform>
type Corner = 'tl' | 'tr' | 'bl' | 'br'
type Mode = 'move' | Corner

const MIN_SCALE = 0.05

/** Clamp a transform: scale into [MIN_SCALE, 1], and x/y so the box stays fully
 *  inside the frame (x,y ∈ [0, 1−scale]). Keeps the overlay on-canvas + visible. */
function clampT(t: T): T {
  const scale = Math.min(1, Math.max(MIN_SCALE, t.scale))
  const max = Math.max(0, 1 - scale)
  return {
    x: Math.min(max, Math.max(0, t.x)),
    y: Math.min(max, Math.max(0, t.y)),
    scale,
    opacity: t.opacity,
  }
}

interface DragState {
  mode: Mode
  startX: number
  startY: number
  start: T
  last: T
}

export function TransformHandles({
  clipId,
  transform,
  stageRef,
  onLive,
  onCommit,
}: {
  clipId: string
  transform: T
  stageRef: RefObject<HTMLDivElement | null>
  onLive: (t: T) => void
  onCommit: (t: T) => void
}) {
  const drag = useRef<DragState | null>(null)

  const begin = (mode: Mode) => (e: RPointerEvent) => {
    if (e.button !== 0) return
    e.preventDefault()
    e.stopPropagation()
    const start = transform
    drag.current = { mode, startX: e.clientX, startY: e.clientY, start, last: start }
    // Pointer capture keeps the drag alive if the cursor leaves the grip; guard
    // it (a synthetic/edge pointer can make setPointerCapture throw).
    try {
      ;(e.target as Element).setPointerCapture?.(e.pointerId)
    } catch {
      /* no active pointer to capture — window listeners below still track it */
    }

    const onMove = (ev: PointerEvent) => {
      const d = drag.current
      const rect = stageRef.current?.getBoundingClientRect()
      if (!d || !rect || rect.width <= 0 || rect.height <= 0) return
      const s = d.start
      let next: T
      if (d.mode === 'move') {
        const dxN = (ev.clientX - d.startX) / rect.width
        const dyN = (ev.clientY - d.startY) / rect.height
        next = clampT({ ...s, x: s.x + dxN, y: s.y + dyN })
      } else {
        // Corner scale: hold the OPPOSITE corner fixed. The anchor is the corner
        // diagonally across from the dragged one; scale = the larger of the
        // cursor→anchor spans (uniform), then recompute top-left to pin the anchor.
        const anchorX = d.mode === 'tl' || d.mode === 'bl' ? s.x + s.scale : s.x
        const anchorY = d.mode === 'tl' || d.mode === 'tr' ? s.y + s.scale : s.y
        const cx = (ev.clientX - rect.left) / rect.width
        const cy = (ev.clientY - rect.top) / rect.height
        let scale = Math.max(Math.abs(cx - anchorX), Math.abs(cy - anchorY))
        scale = Math.min(1, Math.max(MIN_SCALE, scale))
        const nx = d.mode === 'tl' || d.mode === 'bl' ? anchorX - scale : anchorX
        const ny = d.mode === 'tl' || d.mode === 'tr' ? anchorY - scale : anchorY
        next = clampT({ ...s, x: nx, y: ny, scale })
      }
      d.last = next
      onLive(next)
    }
    const onUp = () => {
      window.removeEventListener('pointermove', onMove)
      window.removeEventListener('pointerup', onUp)
      const d = drag.current
      drag.current = null
      if (d) onCommit(d.last)
    }
    window.addEventListener('pointermove', onMove)
    window.addEventListener('pointerup', onUp)
  }

  // The box geometry IS the normalized transform as % of the stage.
  const box = {
    left: `${(transform.x * 100).toFixed(3)}%`,
    top: `${(transform.y * 100).toFixed(3)}%`,
    width: `${(transform.scale * 100).toFixed(3)}%`,
    height: `${(transform.scale * 100).toFixed(3)}%`,
  }
  const corners: { c: Corner; cursor: string; style: React.CSSProperties }[] = [
    { c: 'tl', cursor: 'nwse-resize', style: { left: 0, top: 0 } },
    { c: 'tr', cursor: 'nesw-resize', style: { right: 0, top: 0 } },
    { c: 'bl', cursor: 'nesw-resize', style: { left: 0, bottom: 0 } },
    { c: 'br', cursor: 'nwse-resize', style: { right: 0, bottom: 0 } },
  ]

  return (
    <div
      className="pv-xform"
      data-cut-xform
      data-cut-xform-clip={clipId}
      style={box}
      onPointerDown={begin('move')}
    >
      {corners.map(({ c, cursor, style }) => (
        <div
          key={c}
          className="pv-xform-handle"
          data-cut-xform-handle={c}
          style={{ ...style, cursor }}
          onPointerDown={begin(c)}
        />
      ))}
    </div>
  )
}
