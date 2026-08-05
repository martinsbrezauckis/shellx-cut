// panels/Preview/MaskOverlay — on-canvas SHAPE drawing + drag-resize handles for
// edit.add_mask, drawn over the live preview stage. The human surface for the
// region-blur / privacy-mask primitive: the Mask drawer (panels/Mask) ARMS this
// (cut:mask-draw); this owns the GEOMETRY and reports it back (cut:mask-geometry)
// so the drawer's Apply has the points.
//
// REUSES the TransformHandles pattern (hand-rolled PointerEvents, the normalized
// `.pv-stage` coordinate space, window pointermove/up listeners + pointer capture)
// generalized from a SQUARE transform box to a free RECT / ELLIPSE bounding box
// and a POLYGON vertex set. TransformHandles itself is uniform-scale (one `scale`,
// width == height) so it cannot represent a non-square mask region — but the drag
// MECHANICS are identical and reused verbatim in spirit (begin → window listeners →
// onCommit; Δpx / stageRect.dim = normalized delta; round late, clamp to frame).
//
// COORDINATE SPACE: the `.pv-stage` IS the normalized frame (its rect already
// excludes the letterbox bars), so (clientX-rect.left)/rect.width is the frame-X
// fraction (0..1) — the same mapping TransformHandles + the redact draw use.
//
// SHAPE → verb geometry (all points fractions of frame W/H, 0..1):
//   rect    → [[x0,y0],[x1,y1]]   (2 opposite corners)
//   ellipse → [[cx,cy],[rx,ry]]   (centre + radii, derived from the drawn bbox)
//   polygon → the vertices (≥3, drawn closed)
//
// INTERACT: drag an empty area to rubber-band a rect/ellipse bbox; click an empty
// area to drop a polygon vertex. Then drag a CORNER grip to resize (rect/ellipse),
// the box BODY to move, or a VERTEX grip to reshape (polygon). Every COMMIT (draw
// end / grip up / vertex add or move) reports the verb-ready points + a `ready`
// flag (bbox ≥ 2% in both axes, or polygon ≥ 3 verts) the drawer uses to gate Apply.
//
// Callers: Preview/index.tsx (mounted INSIDE the stage while the Mask drawer arms a
// draw). Deps: ../Mask/mask.css (the feature's own scoped .mk-* styles — preview.css
// is deliberately untouched).

import { useEffect, useRef, useState, type PointerEvent as RPointerEvent, type RefObject } from 'react'
import '../Mask/mask.css'

export type MaskShape = 'rect' | 'ellipse' | 'polygon'

/** The verb-ready geometry reported to the Mask drawer on every commit. `points`
 *  is already in edit.add_mask's per-shape format; `ready` is false until the
 *  drawn region is large enough (rect/ellipse) or has ≥3 verts (polygon). */
export interface MaskGeometry {
  shape: MaskShape
  points: [number, number][]
  ready: boolean
}

interface Box { x0: number; y0: number; x1: number; y1: number }
interface Pt { x: number; y: number }
type Corner = 'tl' | 'tr' | 'bl' | 'br'

const CORNERS: Corner[] = ['tl', 'tr', 'bl', 'br']
const MIN_SPAN = 0.02 // a box < 2% of the frame in either axis is a stray click, not a region
const clamp01 = (v: number) => Math.min(1, Math.max(0, v))
const r4 = (v: number) => +clamp01(v).toFixed(4)

/** Sort a raw drag box to [x0<x1, y0<y1] corners, each clamped into the frame. */
function norm(b: Box): Box {
  return {
    x0: clamp01(Math.min(b.x0, b.x1)),
    y0: clamp01(Math.min(b.y0, b.y1)),
    x1: clamp01(Math.max(b.x0, b.x1)),
    y1: clamp01(Math.max(b.y0, b.y1)),
  }
}
function cornerPt(c: Corner, n: Box): Pt {
  return {
    x: c === 'tl' || c === 'bl' ? n.x0 : n.x1,
    y: c === 'tl' || c === 'tr' ? n.y0 : n.y1,
  }
}
const cornerCursor = (c: Corner) => (c === 'tl' || c === 'br' ? 'nwse-resize' : 'nesw-resize')

export function MaskOverlay({
  shape,
  stageRef,
  onGeometry,
}: {
  shape: MaskShape
  stageRef: RefObject<HTMLDivElement | null>
  onGeometry: (g: MaskGeometry) => void
}) {
  // rect/ellipse share one bounding box; polygon is a vertex list. Each is mirrored
  // into a ref so the pointer-up COMMIT reads the freshest value without depending on
  // a React state flush (the TransformHandles `last`-ref pattern).
  const [box, setBox] = useState<Box | null>(null)
  const [poly, setPoly] = useState<Pt[]>([])
  const boxRef = useRef<Box | null>(null)
  const polyRef = useRef<Pt[]>([])
  const drawing = useRef(false)
  const moved = useRef(false)
  const onGeom = useRef(onGeometry)
  onGeom.current = onGeometry

  const setBoxLive = (b: Box | null) => { boxRef.current = b; setBox(b) }
  const setPolyLive = (p: Pt[]) => { polyRef.current = p; setPoly(p) }

  /** Map a client point → normalized frame fraction (0..1), clamped. */
  const frac = (clientX: number, clientY: number): Pt | null => {
    const rect = stageRef.current?.getBoundingClientRect()
    if (!rect || rect.width <= 0 || rect.height <= 0) return null
    return { x: clamp01((clientX - rect.left) / rect.width), y: clamp01((clientY - rect.top) / rect.height) }
  }

  /** Report the current shape as edit.add_mask geometry (called on every commit). */
  const emit = () => {
    if (shape === 'polygon') {
      const p = polyRef.current
      onGeom.current({ shape, points: p.map((v) => [r4(v.x), r4(v.y)] as [number, number]), ready: p.length >= 3 })
      return
    }
    const b = boxRef.current
    if (!b) { onGeom.current({ shape, points: [], ready: false }); return }
    const n = norm(b)
    const ready = n.x1 - n.x0 >= MIN_SPAN && n.y1 - n.y0 >= MIN_SPAN
    if (shape === 'ellipse') {
      const cx = (n.x0 + n.x1) / 2, cy = (n.y0 + n.y1) / 2
      const rx = (n.x1 - n.x0) / 2, ry = (n.y1 - n.y0) / 2
      onGeom.current({ shape, points: [[r4(cx), r4(cy)], [r4(rx), r4(ry)]], ready })
    } else {
      onGeom.current({ shape, points: [[r4(n.x0), r4(n.y0)], [r4(n.x1), r4(n.y1)]], ready })
    }
  }

  // On (re)mount — a fresh shape kind keys a remount upstream — announce an empty,
  // not-ready geometry so the drawer resets its Apply gate + cached points.
  useEffect(() => {
    onGeom.current({ shape, points: [], ready: false })
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [])

  // --- initial draw on the full-stage capture layer --------------------------
  const onCaptureDown = (e: RPointerEvent) => {
    if (e.button !== 0) return
    const p = frac(e.clientX, e.clientY)
    if (!p) return
    e.preventDefault()
    try { (e.currentTarget as Element).setPointerCapture(e.pointerId) } catch { /* no pointer to capture */ }
    drawing.current = true
    moved.current = false
    // polygon drops a vertex on a click (recorded on up); rect/ellipse anchor a box.
    if (shape !== 'polygon') setBoxLive({ x0: p.x, y0: p.y, x1: p.x, y1: p.y })
  }
  const onCaptureMove = (e: RPointerEvent) => {
    if (!drawing.current) return
    const p = frac(e.clientX, e.clientY)
    if (!p) return
    moved.current = true
    if (shape !== 'polygon' && boxRef.current) setBoxLive({ ...boxRef.current, x1: p.x, y1: p.y })
  }
  const onCaptureUp = (e: RPointerEvent) => {
    if (!drawing.current) return
    drawing.current = false
    try { (e.currentTarget as Element).releasePointerCapture(e.pointerId) } catch { /* not captured */ }
    if (shape === 'polygon') {
      // a click (down→up with no drag) drops a vertex; a drag is ignored (verts are
      // placed one click at a time, then nudged with their grips).
      const p = frac(e.clientX, e.clientY)
      if (!moved.current && p) { setPolyLive([...polyRef.current, p]); emit() }
      return
    }
    emit()
  }

  // --- adjust an existing rect/ellipse box (corner grips + body move) ---------
  // The TransformHandles drag pattern: capture the start, track via window
  // listeners, commit on pointer-up. Δpx / stageRect.dim = normalized delta.
  const beginBox = (mode: Corner | 'move') => (e: RPointerEvent) => {
    if (e.button !== 0 || !boxRef.current) return
    e.preventDefault()
    e.stopPropagation()
    const start = norm(boxRef.current)
    const sx = e.clientX, sy = e.clientY
    try { (e.target as Element).setPointerCapture?.(e.pointerId) } catch { /* window listeners still track it */ }
    const onMove = (ev: PointerEvent) => {
      const rect = stageRef.current?.getBoundingClientRect()
      if (!rect || rect.width <= 0 || rect.height <= 0) return
      const dxN = (ev.clientX - sx) / rect.width
      const dyN = (ev.clientY - sy) / rect.height
      let nb: Box
      if (mode === 'move') {
        // Move the whole box, clamped so it stays fully in-frame (size preserved).
        const w = start.x1 - start.x0, h = start.y1 - start.y0
        const nx0 = Math.min(Math.max(0, start.x0 + dxN), 1 - w)
        const ny0 = Math.min(Math.max(0, start.y0 + dyN), 1 - h)
        nb = { x0: nx0, y0: ny0, x1: nx0 + w, y1: ny0 + h }
      } else {
        nb = { ...start }
        if (mode === 'tl' || mode === 'bl') nb.x0 = clamp01(start.x0 + dxN)
        if (mode === 'tr' || mode === 'br') nb.x1 = clamp01(start.x1 + dxN)
        if (mode === 'tl' || mode === 'tr') nb.y0 = clamp01(start.y0 + dyN)
        if (mode === 'bl' || mode === 'br') nb.y1 = clamp01(start.y1 + dyN)
      }
      setBoxLive(nb)
    }
    const onUp = () => {
      window.removeEventListener('pointermove', onMove)
      window.removeEventListener('pointerup', onUp)
      emit()
    }
    window.addEventListener('pointermove', onMove)
    window.addEventListener('pointerup', onUp)
  }

  // --- adjust a polygon vertex ------------------------------------------------
  const beginVertex = (i: number) => (e: RPointerEvent) => {
    if (e.button !== 0) return
    e.preventDefault()
    e.stopPropagation()
    const start = polyRef.current[i]
    if (!start) return
    const sx = e.clientX, sy = e.clientY
    try { (e.target as Element).setPointerCapture?.(e.pointerId) } catch { /* window listeners track it */ }
    const onMove = (ev: PointerEvent) => {
      const rect = stageRef.current?.getBoundingClientRect()
      if (!rect || rect.width <= 0 || rect.height <= 0) return
      const nx = clamp01(start.x + (ev.clientX - sx) / rect.width)
      const ny = clamp01(start.y + (ev.clientY - sy) / rect.height)
      setPolyLive(polyRef.current.map((p, idx) => (idx === i ? { x: nx, y: ny } : p)))
    }
    const onUp = () => {
      window.removeEventListener('pointermove', onMove)
      window.removeEventListener('pointerup', onUp)
      emit()
    }
    window.addEventListener('pointermove', onMove)
    window.addEventListener('pointerup', onUp)
  }

  const n = box ? norm(box) : null
  const polyPts = poly.map((p) => `${(p.x * 100).toFixed(3)},${(p.y * 100).toFixed(3)}`).join(' ')

  return (
    <>
      {/* Full-stage capture layer — draws a new rect/ellipse on empty area, or drops
          a polygon vertex on a click. Crosshair signals "drag a region". */}
      <div
        className="mk-capture"
        data-cut-mask-capture
        data-cut-mask-capture-shape={shape}
        onPointerDown={onCaptureDown}
        onPointerMove={onCaptureMove}
        onPointerUp={onCaptureUp}
      />

      {/* Shape OUTLINE — visual only (pointer-events:none). viewBox 0..100 with
          preserveAspectRatio:none lets us position in frame-% directly; the stroke
          stays crisp via vector-effect:non-scaling-stroke (.mk-shape). */}
      <svg className="mk-svg" data-cut-mask-shape={shape} viewBox="0 0 100 100" preserveAspectRatio="none" aria-hidden="true">
        {shape === 'rect' && n && (
          <rect className="mk-shape" x={n.x0 * 100} y={n.y0 * 100} width={(n.x1 - n.x0) * 100} height={(n.y1 - n.y0) * 100} />
        )}
        {shape === 'ellipse' && n && (
          <ellipse
            className="mk-shape"
            cx={((n.x0 + n.x1) / 2) * 100}
            cy={((n.y0 + n.y1) / 2) * 100}
            rx={((n.x1 - n.x0) / 2) * 100}
            ry={((n.y1 - n.y0) / 2) * 100}
          />
        )}
        {shape === 'polygon' && poly.length >= 3 && <polygon className="mk-shape" points={polyPts} />}
        {shape === 'polygon' && poly.length > 0 && poly.length < 3 && (
          <polyline className="mk-shape mk-shape--open" points={polyPts} />
        )}
      </svg>

      {/* rect/ellipse: draggable interior (move) + 4 corner grips (resize). Sit above
          the capture layer so a press inside the box moves it; outside starts a new one. */}
      {shape !== 'polygon' && n && (
        <>
          <div
            className="mk-body"
            data-cut-mask-body
            style={{ left: `${n.x0 * 100}%`, top: `${n.y0 * 100}%`, width: `${(n.x1 - n.x0) * 100}%`, height: `${(n.y1 - n.y0) * 100}%` }}
            onPointerDown={beginBox('move')}
          />
          {CORNERS.map((c) => {
            const cp = cornerPt(c, n)
            return (
              <div
                key={c}
                className="mk-handle"
                data-cut-mask-handle={c}
                style={{ left: `${cp.x * 100}%`, top: `${cp.y * 100}%`, cursor: cornerCursor(c) }}
                onPointerDown={beginBox(c)}
              />
            )
          })}
        </>
      )}

      {/* polygon: one drag grip per vertex. */}
      {shape === 'polygon' &&
        poly.map((p, i) => (
          <div
            key={i}
            className="mk-handle mk-handle--vertex"
            data-cut-mask-handle={`v${i}`}
            style={{ left: `${p.x * 100}%`, top: `${p.y * 100}%` }}
            onPointerDown={beginVertex(i)}
          />
        ))}
    </>
  )
}
