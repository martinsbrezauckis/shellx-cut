// panels/Shape — the vector-shape drawer for edit.add_shape.
// Role: a right-side drawer (the Title/Matte drawer family) that drives ONE verb
// — edit.add_shape — to drop an annotation shape (rect / ellipse / line / arrow,
// + an optional callout label) on a top overlay track over a timed span. Makes
// the agent-only edit.add_shape a real user feature (callouts, arrows, boxes for
// tutorials/explainers).
//
// UX: position via a 3×3 PRESET grid (rect/ellipse) or two preset points
// (line/arrow) — far friendlier than raw normalized coords; colors + stroke +
// the optional label are the style knobs. Fires the verb; the shape composites
// through the existing title-overlay pipeline + shows in the preview.
//
// Callers: App.tsx (mounted when open). Deps: lib/client (verbs), ../drawer.css.

import { useEffect, useState } from 'react'
import { callVerb, type Project } from '../../lib/client'
import { useBlockingOverlay } from '../../components/overlay/useBlockingOverlay'
import '../drawer.css'

export interface ShapeDrawerProps {
  project: Project | null
  /** Live playhead (ms) — seeds the default span. */
  defaultInMs: number
  onClose: () => void
}

type ShapeKind = 'rect' | 'ellipse' | 'line' | 'arrow'
type Anim = 'fade' | 'slide_up' | 'slide_down' | 'slide_left' | 'slide_right' | 'pop' | 'none'
const ANIMS: Anim[] = ['fade', 'slide_up', 'slide_down', 'slide_left', 'slide_right', 'pop', 'none']

function animFromInput(value: string, fallback: Anim): Anim {
  for (const option of ANIMS) {
    if (option === value) return option
  }
  return fallback
}

interface ShapeResult { shape_track: string; clip_id: string; shape: string; range_ms: [number, number] }

/** 3×3 box presets (rect/ellipse): {x,y,w,h} normalized, by position. */
const BOX_PRESETS: { id: string; label: string; x: number; y: number; w: number; h: number }[] = [
  { id: 'tl', label: '↖', x: 0.06, y: 0.08, w: 0.34, h: 0.22 },
  { id: 'tc', label: '↑', x: 0.33, y: 0.08, w: 0.34, h: 0.22 },
  { id: 'tr', label: '↗', x: 0.60, y: 0.08, w: 0.34, h: 0.22 },
  { id: 'ml', label: '←', x: 0.06, y: 0.39, w: 0.34, h: 0.22 },
  { id: 'mc', label: '•', x: 0.30, y: 0.39, w: 0.40, h: 0.22 },
  { id: 'mr', label: '→', x: 0.60, y: 0.39, w: 0.34, h: 0.22 },
  { id: 'bl', label: '↙', x: 0.06, y: 0.70, w: 0.34, h: 0.22 },
  { id: 'bc', label: '↓', x: 0.33, y: 0.70, w: 0.34, h: 0.22 },
  { id: 'br', label: '↘', x: 0.60, y: 0.70, w: 0.34, h: 0.22 },
]
/** Line/arrow direction presets: start→end normalized points. */
const LINE_PRESETS: { id: string; label: string; x: number; y: number; x2: number; y2: number }[] = [
  { id: 'lr', label: '→ across', x: 0.15, y: 0.5, x2: 0.85, y2: 0.5 },
  { id: 'diag', label: '↘ point to', x: 0.2, y: 0.25, x2: 0.7, y2: 0.7 },
  { id: 'up', label: '↗ rise', x: 0.2, y: 0.8, x2: 0.8, y2: 0.3 },
]

export default function ShapeDrawer({ project, defaultInMs, onClose }: ShapeDrawerProps) {
  const overlay = useBlockingOverlay<HTMLElement>(onClose)
  const [shape, setShape] = useState<ShapeKind>('rect')
  const [boxPreset, setBoxPreset] = useState('mc')
  const [linePreset, setLinePreset] = useState('diag')
  const [fillOn, setFillOn] = useState(true)
  const [fill, setFill] = useState('#3366FF')
  const [stroke, setStroke] = useState('#FFFFFF')
  const [strokePx, setStrokePx] = useState(6)
  const [text, setText] = useState('')
  const [color] = useState('#FFFFFF') // label color (white; matches the callout default)
  const [anim, setAnim] = useState<Anim>('fade')
  const [inS, setInS] = useState(Math.max(0, defaultInMs / 1000))
  const [outS, setOutS] = useState(Math.max(0, defaultInMs / 1000) + 3)
  const [busy, setBusy] = useState(false)
  const [result, setResult] = useState<ShapeResult | null>(null)
  const [err, setErr] = useState<string | null>(null)

  const isBox = shape === 'rect' || shape === 'ellipse'
  // line/arrow render as a stroked path (+ arrowhead); a hairline <6px is invisible
  // at preview scale and the engine bumps it anyway, so these kinds enforce a 6px
  // minimum width. Boxes use `stroke` as a border and allow a thin 1px line.
  const strokeMin = isBox ? 1 : 6

  // Keep the displayed Width truthful: when the kind switches to line/arrow, lift a
  // sub-minimum width up to strokeMin so the number shown == the number sent (no
  // silent engine override). Box→line carrying a 1–5px border is the path that hit this.
  useEffect(() => {
    if (strokePx < strokeMin) setStrokePx(strokeMin)
  }, [strokeMin, strokePx])


  const inMs = Math.round(inS * 1000), outMs = Math.round(outS * 1000)
  const canFire = !!project && outMs > inMs

  const fire = async () => {
    if (!canFire) return
    setBusy(true); setErr(null); setResult(null)
    try {
      const args: Record<string, unknown> = { shape, range_ms: [inMs, outMs], stroke, stroke_px: strokePx, animation: anim }
      if (isBox) {
        const p = BOX_PRESETS.find((b) => b.id === boxPreset)!
        args.x = p.x; args.y = p.y; args.w = p.w; args.h = p.h
        if (fillOn) args.fill = fill
        if (text.trim()) { args.text = text.trim(); args.color = color }
        if (shape === 'rect') args.radius_px = 16
      } else {
        const p = LINE_PRESETS.find((l) => l.id === linePreset)!
        args.x = p.x; args.y = p.y; args.x2 = p.x2; args.y2 = p.y2
        // line/arrow use `stroke` as the line color. Safety net only: the Width
        // input now enforces a 6px min (strokeMin) for these kinds, so this
        // sub-6 override is unreachable from the UI and never silently rewrites a
        // value the user can see. Kept to guard non-UI/programmatic callers.
        if (strokePx < 6) args.stroke_px = 8
      }
      const r = await callVerb('edit.add_shape', args as never)
      if (r.ok) { setResult(r.result as ShapeResult); document.dispatchEvent(new CustomEvent('cut:show-composed')) }
      else setErr(r.error?.message ?? 'Could not add the shape.')
    } catch { setErr('server unreachable') }
    finally { setBusy(false) }
  }

  return (
    <div className="cd-scrim" data-cut-shape-scrim onMouseDown={overlay.onScrimMouseDown}>
      <aside ref={overlay.dialogRef} className="cd-drawer" data-cut-shape data-cut-shape-open="true"
        data-cut-blocking-overlay role="dialog" aria-modal="true" aria-label="Add shape" tabIndex={-1}
        onKeyDown={overlay.onDialogKeyDown}>
        <header className="cd-head">
          <div>
            <h2 className="cd-title">Shape</h2>
            <p className="cd-sub">Add a rectangle, ellipse, line, arrow, or labelled callout.</p>
          </div>
          <button className="cd-btn cd-btn--ghost" data-cut-shape-close onClick={onClose}>Close</button>
        </header>

        <div className="cd-body">
          {!project ? (
            <div className="cd-empty" data-cut-shape-empty>Create a project in Projects first.</div>
          ) : (
            <>
              {/* shape kind */}
              <div className="cd-field">
                <span className="cd-field-label">Shape</span>
                <div className="cd-seg" role="tablist" data-cut-shape-kind>
                  {(['rect', 'ellipse', 'line', 'arrow'] as ShapeKind[]).map((k) => (
                    <button key={k} role="tab" aria-selected={shape === k}
                      className={`cd-seg-btn ${shape === k ? 'cd-seg-btn--on' : ''}`}
                      data-cut-shape-kind-opt={k} onClick={() => setShape(k)}>{k}</button>
                  ))}
                </div>
              </div>

              {/* position */}
              {isBox ? (
                <div className="cd-field">
                  <span className="cd-field-label">Position</span>
                  <div className="cd-grid3" data-cut-shape-box-presets>
                    {BOX_PRESETS.map((p) => (
                      <button key={p.id} className={`cd-grid3-cell ${boxPreset === p.id ? 'cd-grid3-cell--on' : ''}`}
                        data-cut-shape-box-preset={p.id} title={p.id} onClick={() => setBoxPreset(p.id)}>{p.label}</button>
                    ))}
                  </div>
                </div>
              ) : (
                <div className="cd-field">
                  <span className="cd-field-label">Direction</span>
                  <div className="cd-seg" data-cut-shape-line-presets>
                    {LINE_PRESETS.map((p) => (
                      <button key={p.id} className={`cd-seg-btn ${linePreset === p.id ? 'cd-seg-btn--on' : ''}`}
                        data-cut-shape-line-preset={p.id} onClick={() => setLinePreset(p.id)}>{p.label}</button>
                    ))}
                  </div>
                </div>
              )}

              {/* style */}
              {isBox && (
                <div className="cd-row">
                  <label className="cd-check"><input type="checkbox" data-cut-shape-fill-on checked={fillOn} onChange={(e) => setFillOn(e.target.checked)} /><span>Fill</span></label>
                  {fillOn && <input className="cd-input cd-input--mono" data-cut-shape-fill type="text" aria-label="Fill color" value={fill} onChange={(e) => setFill(e.target.value)} style={{ maxWidth: 110 }} />}
                </div>
              )}
              <div className="cd-row">
                <label className="cd-field cd-field--inline"><span className="cd-field-label">{isBox ? 'Border' : 'Color'}</span>
                  <input className="cd-input cd-input--mono" data-cut-shape-stroke type="text" value={stroke} onChange={(e) => setStroke(e.target.value)} style={{ maxWidth: 110 }} /></label>
                <label className="cd-field cd-field--inline"><span className="cd-field-label">Width</span>
                  {/* min is strokeMin (6 for line/arrow, 1 for boxes) so the shown value always == the sent value. */}
                  <input className="cd-input cd-input--mono" data-cut-shape-strokepx type="number" min={strokeMin} max={40} value={strokePx} onChange={(e) => setStrokePx(Math.min(40, Math.max(strokeMin, Number(e.target.value) || strokeMin)))} style={{ maxWidth: 70 }} /></label>
              </div>

              {isBox && (
                <label className="cd-field">
                  <span className="cd-field-label">Label (optional callout)</span>
                  <input className="cd-input" data-cut-shape-text type="text" placeholder="e.g. Look here" value={text} onChange={(e) => setText(e.target.value)} />
                </label>
              )}

              {/* animation */}
              <label className="cd-field">
                <span className="cd-field-label">Animation</span>
                <select className="cd-sel" data-cut-shape-anim value={anim} onChange={(e) => setAnim(animFromInput(e.target.value, anim))}>
                  {ANIMS.map((a) => <option key={a} value={a}>{a}</option>)}
                </select>
              </label>

              {/* span */}
              <div className="cd-row">
                <label className="cd-field"><span className="cd-field-label">In (s)</span>
                  <input className="cd-input cd-input--mono" data-cut-shape-in type="number" min={0} step={0.1} value={inS} onChange={(e) => setInS(Math.max(0, Number(e.target.value) || 0))} /></label>
                <label className="cd-field"><span className="cd-field-label">Out (s)</span>
                  <input className="cd-input cd-input--mono" data-cut-shape-out type="number" min={0} step={0.1} value={outS} onChange={(e) => setOutS(Math.max(0, Number(e.target.value) || 0))} /></label>
              </div>

              <button className="cd-btn cd-btn--primary" data-cut-shape-apply disabled={busy || !canFire} onClick={() => void fire()}>
                {busy ? 'Adding…' : 'Add shape'}
              </button>
              {err && <div className="cd-err" data-cut-shape-error role="alert">{err}</div>}
              {result && (
                <div className="cd-result" data-cut-shape-result>
                  <div className="cd-result-head" data-cut-shape-result-kind>{result.shape} placed · {result.shape_track}</div>
                  <div className="cd-result-foot">Scrub to the span — the shape shows in the composed preview.</div>
                </div>
              )}
            </>
          )}
        </div>
      </aside>
    </div>
  )
}
