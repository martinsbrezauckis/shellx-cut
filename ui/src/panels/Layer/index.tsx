// panels/Layer — the layer / picture-in-picture drawer (UI for edit.transform +
// edit.reorder_track). Supports adding and arranging video over video. The
// engine already composites overlay video tracks (track
// order = z-order; ffmpeg overlay); this drawer is the human control surface:
// position / scale / OPACITY of an overlay clip, its stacking order (bring
// forward / send back), and "add a video layer".
//
// PLACEMENT: contextual to the selected video clip (like Grade) — a "Layer"
// button on the timeline toolbar that enables on a video-clip selection and
// dispatches cut:open-layer. Seeds its sliders from the clip's current transform
// so re-opening edits from where the clip is. Identity (0,0,1,1) clears the
// transform (the clip composites full-frame). No live preview here: it fires the
// verb and the COMPOSED preview at the playhead shows the actual result.
//
// Callers: App.tsx (mounted when open, with the selected clip id + project).
// Deps: lib/client, ../drawer.css.

import { useMemo, useState } from 'react'
import {
  callVerb,
  mediaClipTimelineDurationMs,
  type ClipAnimation,
  type ClipCrop,
  type ClipFreeze,
  type ClipTransform,
  type Keyframe,
  type KfParam,
  type KfInterp,
  type Project,
} from '../../lib/client'
import { Icon } from '../../icons'
import { baseVideoTrackId } from '../../lib/layerStack'
import { trackOrderStatus, trackReorderTargetIndex } from '../Timeline/trackControlsModel'
import { useBlockingOverlay } from '../../components/overlay/useBlockingOverlay'
import '../drawer.css'

/** The animatable parameters the Layer keyframe editor exposes (video overlay
 *  clips). Volume keyframes live on audio clips, edited elsewhere. scale = an
 *  eased animated ZOOM (centred), the multi-point form of edit.animate. */
type LayerKfParam = Extract<KfParam, 'opacity' | 'pos_x' | 'pos_y' | 'scale'>
const KF_PARAMS: { value: LayerKfParam; label: string; lo: number; hi: number; def: number }[] = [
  { value: 'opacity', label: 'Opacity (fade)', lo: 0, hi: 1, def: 1 },
  { value: 'pos_x', label: 'Position X (slide)', lo: -0.5, hi: 1.5, def: 0 },
  { value: 'pos_y', label: 'Position Y (slide)', lo: -0.5, hi: 1.5, def: 0 },
  { value: 'scale', label: 'Scale (zoom)', lo: 1, hi: 4, def: 1 },
]

/** Interpolation presets for the keyframe editor — linear/hold + the Penner curve
 *  set (one stored interp per track). Eased motion reads professional rather than
 *  mechanical; back/elastic overshoot, bounce settles. Mirrors cut-core::KfInterp. */
const KF_INTERPS: { value: KfInterp; label: string }[] = [
  { value: 'linear', label: 'Linear (constant)' },
  { value: 'hold', label: 'Hold (stepped)' },
  { value: 'ease_in_out_cubic', label: 'Ease in-out (smooth)' },
  { value: 'ease_out_cubic', label: 'Ease out (decelerate)' },
  { value: 'ease_in_cubic', label: 'Ease in (accelerate)' },
  { value: 'ease_in_out_quad', label: 'Ease in-out (gentle)' },
  { value: 'ease_in_out_expo', label: 'Ease in-out (sharp)' },
  { value: 'ease_out_back', label: 'Back (overshoot)' },
  { value: 'ease_out_elastic', label: 'Elastic (spring)' },
  { value: 'ease_out_bounce', label: 'Bounce (settle)' },
]

/** Ken Burns presets exposed in the Motion section (mirror the engine set). */
type KenBurnsPreset = 'zoom_in' | 'zoom_out' | 'pan_left' | 'pan_right' | 'pan_up' | 'pan_down'
const KEN_BURNS_PRESETS: { value: KenBurnsPreset; label: string }[] = [
  { value: 'zoom_in', label: 'Zoom in' },
  { value: 'zoom_out', label: 'Zoom out' },
  { value: 'pan_left', label: 'Pan left' },
  { value: 'pan_right', label: 'Pan right' },
  { value: 'pan_up', label: 'Pan up' },
  { value: 'pan_down', label: 'Pan down' },
]

export interface LayerDrawerProps {
  project: Project | null
  /** The clip to edit (App passes selectedClipIds[0]). */
  clipId: string | null
  onClose: () => void
}

/** Locate the media clip + its track placement so the drawer can seed the
 *  transform and reason about z-order (is this the base layer? can it move?). */
interface ClipPlacement {
  found: boolean
  transform: ClipTransform | null
  /** Current SOURCE crop rectangle (edit.crop), or null = whole frame. */
  crop: ClipCrop | null
  /** The clip's asset id — to look up the source pixel geometry for crop bounds. */
  assetId: string | null
  trackId: string | null
  /** Persisted timeline edit guard for this track. */
  locked: boolean
  /** True when this clip sits on the FIRST video track with clips = the base
   *  canvas (the bottom layer). */
  isBaseVideo: boolean
  /** Motion state (edit.reverse / edit.freeze / edit.animate) to seed the controls. */
  reverse: boolean
  freeze: ClipFreeze | null
  animation: ClipAnimation | null
  /** Visible source span (src_out − src_in) in ms — the freeze at_ms range. */
  srcSpanMs: number
  /** Realized constant-speed timeline span — the keyframe time range. */
  timelineSpanMs: number
  /** Parameter keyframes on the clip (edit.keyframe) — seeds the keyframe editor. */
  keyframes: Keyframe[]
}

function findPlacement(project: Project | null, clipId: string | null): ClipPlacement {
  const base: ClipPlacement = { found: false, transform: null, crop: null, assetId: null, trackId: null, locked: false, isBaseVideo: false, reverse: false, freeze: null, animation: null, srcSpanMs: 0, timelineSpanMs: 0, keyframes: [] }
  if (!project || !clipId) return base
  const baseTrackId = baseVideoTrackId(project.tracks)
  for (const t of project.tracks) {
    for (const c of t.clips) {
      if ('asset' in c && c.id === clipId) {
        return {
          found: true,
          transform: c.transform ?? null,
          crop: c.crop ?? null,
          assetId: c.asset,
          trackId: t.id,
          locked: !!t.locked,
          isBaseVideo: t.id === baseTrackId,
          reverse: c.reverse ?? false,
          freeze: c.freeze ?? null,
          animation: c.animation ?? null,
          srcSpanMs: Math.max(0, (c.src_out_ms ?? 0) - (c.src_in_ms ?? 0)),
          timelineSpanMs: mediaClipTimelineDurationMs(c),
          keyframes: c.keyframes ?? [],
        }
      }
    }
  }
  return base
}

/** Source pixel geometry from the clip's asset probe — crop is in SOURCE px and
 *  must stay inside this. Returns null until the asset is probed (no dimensions
 *  yet → the crop UI shows a "probe pending" hint instead of unbounded sliders). */
function sourceDims(project: Project | null, assetId: string | null): { w: number; h: number } | null {
  if (!project || !assetId) return null
  const probe = project.assets?.[assetId]?.probe as { width?: number; height?: number } | undefined
  if (!probe?.width || !probe?.height) return null
  return { w: probe.width, h: probe.height }
}

/** Identity transform — full-frame, fully opaque (clears the stored transform). */
const IDENTITY = { x: 0, y: 0, scale: 1, opacity: 1 }

/** A tiny SVG sparkline of a keyframe track — clip time (x) vs value (y) — so the
 *  user sees the curve the renderer will interpolate (linear between points). */
function KfCurve({ points, maxT, lo, hi }: { points: { t_ms: number; value: number }[]; maxT: number; lo: number; hi: number }) {
  const W = 232, H = 52, pad = 5
  const xOf = (t: number) => pad + (Math.max(0, Math.min(t, maxT)) / maxT) * (W - 2 * pad)
  const yOf = (v: number) => H - pad - Math.max(0, Math.min((v - lo) / (hi - lo || 1), 1)) * (H - 2 * pad)
  const poly = points.map((p) => `${xOf(p.t_ms).toFixed(1)},${yOf(p.value).toFixed(1)}`).join(' ')
  return (
    <svg data-cut-layer-kf-curve width={W} height={H} viewBox={`0 0 ${W} ${H}`} preserveAspectRatio="none" style={{ width: '100%', height: H, background: 'rgba(255,255,255,0.04)', borderRadius: 4 }}>
      {points.length >= 2 && <polyline points={poly} fill="none" stroke="var(--cut, #4ea1ff)" strokeWidth={1.5} />}
      {points.map((p) => (
        <circle key={p.t_ms} cx={xOf(p.t_ms)} cy={yOf(p.value)} r={2.6} fill="var(--cut, #4ea1ff)" />
      ))}
      {points.length === 0 && (
        <text x={W / 2} y={H / 2} textAnchor="middle" dominantBaseline="middle" fill="currentColor" opacity={0.5} fontSize={11}>no keyframes — add a point</text>
      )}
    </svg>
  )
}

/** A single layer/transform slider. Module-level (NOT defined inside LayerDrawer) so its
 *  `<input>` keeps a stable identity across renders — a component defined in the render body
 *  remounts on every onChange and interrupts the pointer-drag, making the slider stick
 *  mid-drag (the same "hanging" class as the grade sliders). */
function LayerSlider({ label, attr, value, set, min, max, step, fmt }: {
  label: string; attr: string; value: number; set: (n: number) => void; min: number; max: number; step: number; fmt?: (n: number) => string
}) {
  return (
    <label className="cd-field">
      <span className="cd-field-label">
        {label} <span className="cd-val" data-cut-layer-val={attr}>{fmt ? fmt(value) : value}</span>
      </span>
      <input
        className="cd-range"
        data-cut-layer-input={attr}
        type="range"
        min={min}
        max={max}
        step={step}
        value={value}
        onChange={(e) => set(Number(e.target.value))}
      />
    </label>
  )
}

export default function LayerDrawer({ project, clipId, onClose }: LayerDrawerProps) {
  const overlay = useBlockingOverlay<HTMLElement>(onClose)
  const place = useMemo(() => findPlacement(project, clipId), [project, clipId])
  const order = useMemo(
    () => place.trackId ? trackOrderStatus(project?.tracks ?? [], place.trackId) : null,
    [place.trackId, project],
  )
  // Source pixel geometry (crop bounds) from the clip's asset probe.
  const dims = useMemo(() => sourceDims(project, place.assetId), [project, place.assetId])

  // Seed from the clip's current transform (or identity).
  const [x, setX] = useState(place.transform?.x ?? IDENTITY.x)
  const [y, setY] = useState(place.transform?.y ?? IDENTITY.y)
  const [scale, setScale] = useState(place.transform?.scale ?? IDENTITY.scale)
  const [opacity, setOpacity] = useState(place.transform?.opacity ?? IDENTITY.opacity)
  // Seed crop from the clip's current crop, or identity = whole source frame.
  const [cropX, setCropX] = useState(place.crop?.x ?? 0)
  const [cropY, setCropY] = useState(place.crop?.y ?? 0)
  const [cropW, setCropW] = useState(place.crop?.w ?? dims?.w ?? 0)
  const [cropH, setCropH] = useState(place.crop?.h ?? dims?.h ?? 0)
  // Motion (edit.reverse / edit.freeze / edit.animate) — seed from the clip.
  const [reverse, setReverse] = useState(place.reverse)
  const [freezeOn, setFreezeOn] = useState(place.freeze != null)
  const [freezeAt, setFreezeAt] = useState(place.freeze?.at_ms ?? 0)
  const [hasAnim, setHasAnim] = useState(place.animation != null)
  const [preset, setPreset] = useState<KenBurnsPreset>('zoom_in')
  const [amount, setAmount] = useState(0.3)
  // Animated-PiP slide (edit.slide) — slide the overlay IN from / OUT to a screen
  // edge over a duration. The easy path vs hand-authoring pos_x/pos_y keyframes:
  // the verb resolves an edge + mode + duration into a position-keyframe track.
  const [slideEdge, setSlideEdge] = useState<'left' | 'right' | 'top' | 'bottom'>('left')
  const [slideMode, setSlideMode] = useState<'in' | 'out'>('in')
  const [slideMs, setSlideMs] = useState(500)
  // Keyframes (edit.keyframe) — animate opacity / pos_x / pos_y / scale over the clip.
  const [kfParam, setKfParam] = useState<LayerKfParam>('opacity')
  const [kfInterp, setKfInterp] = useState<KfInterp>('linear')
  const [kfTime, setKfTime] = useState(0)
  const [kfValue, setKfValue] = useState(1)
  const [busy, setBusy] = useState(false)
  const [note, setNote] = useState<string | null>(null)
  const [err, setErr] = useState<string | null>(null)

  // Esc closes (drawer family convention).

  const reset = () => { setX(IDENTITY.x); setY(IDENTITY.y); setScale(IDENTITY.scale); setOpacity(IDENTITY.opacity) }
  /** Reset the crop sliders to the whole source frame; applying that clears the
   *  stored crop server-side (identity crop = no crop). Slider-only, like Reset. */
  const resetCrop = () => { if (dims) { setCropX(0); setCropY(0); setCropW(dims.w); setCropH(dims.h) } }

  /** Apply the source crop (edit.crop, SOURCE px). Clamp to the source geometry
   *  so we never send an out-of-bounds rect (the server would hard-error); an
   *  identity crop (origin + full size) clears the stored crop. */
  const applyCrop = async () => {
    if (!place.found || !clipId || place.locked || busy || !dims) return
    const w = Math.max(1, Math.min(Math.round(cropW), dims.w))
    const x0 = Math.max(0, Math.min(Math.round(cropX), dims.w - w))
    const h = Math.max(1, Math.min(Math.round(cropH), dims.h))
    const y0 = Math.max(0, Math.min(Math.round(cropY), dims.h - h))
    setBusy(true); setErr(null); setNote(null)
    try {
      const r = await callVerb('edit.crop', { clip: clipId, x: x0, y: y0, w, h, rationale: 'user: crop source rectangle' })
      if (r.ok) {
        const cleared = x0 === 0 && y0 === 0 && w === dims.w && h === dims.h
        setNote(cleared ? 'Crop cleared (whole frame).' : 'Crop applied — scrub the COMPOSED preview to see it.')
        document.dispatchEvent(new CustomEvent('cut:show-composed'))
      } else {
        setErr(`${r.error?.code ?? 'failed'}: ${r.error?.message ?? 'edit.crop failed'}`)
      }
    } catch { setErr('server unreachable') } finally { setBusy(false) }
  }

  const applyTransform = async () => {
    if (!place.found || !clipId || place.locked || busy) return
    setBusy(true); setErr(null); setNote(null)
    try {
      const r = await callVerb('edit.transform', {
        clip: clipId,
        x: +x.toFixed(3),
        y: +y.toFixed(3),
        scale: +scale.toFixed(3),
        opacity: +opacity.toFixed(3),
        rationale: 'user: layer / PiP transform',
      })
      if (r.ok) {
        setNote('Layer updated — scrub the preview to see it composite.')
        // Show the COMPOSED frame so the PiP/opacity is actually visible (the
        // raw proxy shows neither) — makes the receipt true.
        document.dispatchEvent(new CustomEvent('cut:show-composed'))
      } else {
        setErr(`${r.error?.code ?? 'failed'}: ${r.error?.message ?? 'edit.transform failed'}`)
      }
    } catch { setErr('server unreachable') } finally { setBusy(false) }
  }

  // Reorder the clip's track ±1 in the stack (bring forward / send back).
  const reorder = async (delta: 1 | -1) => {
    if (!place.found || !place.trackId || place.locked || busy) return
    const target = trackReorderTargetIndex(project?.tracks ?? [], place.trackId, delta > 0 ? 'forward' : 'back')
    if (target == null) return
    setBusy(true); setErr(null); setNote(null)
    try {
      const r = await callVerb('edit.reorder_track', {
        track: place.trackId,
        index: target,
        rationale: `user: ${delta > 0 ? 'bring layer forward' : 'send layer back'}`,
      })
      if (r.ok) {
        setNote(delta > 0 ? 'Brought forward.' : 'Sent back.')
        document.dispatchEvent(new CustomEvent('cut:show-composed'))
      }
      else setErr(`${r.error?.code ?? 'failed'}: ${r.error?.message ?? 'reorder failed'}`)
    } catch { setErr('server unreachable') } finally { setBusy(false) }
  }

  const addLayer = async () => {
    if (busy) return
    setBusy(true); setErr(null); setNote(null)
    try {
      const r = await callVerb('edit.add_track', { kind: 'video', rationale: 'user: add video layer' })
      if (r.ok) setNote('Empty video layer added — drag a clip onto it (it overlays the layers below).')
      else setErr(`${r.error?.code ?? 'failed'}: ${r.error?.message ?? 'add layer failed'}`)
    } catch { setErr('server unreachable') } finally { setBusy(false) }
  }

  // --- Motion: reverse / freeze / Ken Burns (edit.reverse/freeze/animate) ----

  /** Toggle reverse playback. enabled:false clears it. */
  const applyReverse = async (next: boolean) => {
    if (!place.found || !clipId || place.locked || busy) return
    setBusy(true); setErr(null); setNote(null)
    try {
      const r = await callVerb('edit.reverse', { clip: clipId, enabled: next, rationale: 'user: reverse clip' })
      if (r.ok) {
        setReverse(next)
        setNote(next ? 'Clip reversed — render or scrub to see it play backward.' : 'Reverse cleared.')
      } else setErr(`${r.error?.code ?? 'failed'}: ${r.error?.message ?? 'edit.reverse failed'}`)
    } catch { setErr('server unreachable') } finally { setBusy(false) }
  }

  /** Toggle freeze-frame at the chosen offset. enabled:false clears it. */
  const applyFreeze = async (on: boolean) => {
    if (!place.found || !clipId || place.locked || busy) return
    setBusy(true); setErr(null); setNote(null)
    try {
      const r = await callVerb('edit.freeze', { clip: clipId, at_ms: Math.round(freezeAt), enabled: on, rationale: 'user: freeze frame' })
      if (r.ok) {
        setFreezeOn(on)
        setNote(on ? 'Freeze applied — the clip holds that one frame (audio plays through).' : 'Freeze cleared.')
        document.dispatchEvent(new CustomEvent('cut:show-composed'))
      } else setErr(`${r.error?.code ?? 'failed'}: ${r.error?.message ?? 'edit.freeze failed'}`)
    } catch { setErr('server unreachable') } finally { setBusy(false) }
  }

  /** Apply a Ken Burns preset (zoom/pan) at the chosen amount. */
  const applyAnimate = async () => {
    if (!place.found || !clipId || place.locked || busy) return
    setBusy(true); setErr(null); setNote(null)
    try {
      const r = await callVerb('edit.animate', { clip: clipId, preset, amount: +amount.toFixed(2), rationale: 'user: ken burns' })
      if (r.ok) {
        setHasAnim(true)
        setNote('Ken Burns applied — render or scrub to see the pan/zoom.')
      } else setErr(`${r.error?.code ?? 'failed'}: ${r.error?.message ?? 'edit.animate failed'}`)
    } catch { setErr('server unreachable') } finally { setBusy(false) }
  }

  /** Clear any Ken Burns animation (edit.animate enabled:false). */
  const clearAnimate = async () => {
    if (!place.found || !clipId || place.locked || busy) return
    setBusy(true); setErr(null); setNote(null)
    try {
      const r = await callVerb('edit.animate', { clip: clipId, enabled: false, rationale: 'user: clear ken burns' })
      if (r.ok) { setHasAnim(false); setNote('Ken Burns cleared.') }
      else setErr(`${r.error?.code ?? 'failed'}: ${r.error?.message ?? 'edit.animate failed'}`)
    } catch { setErr('server unreachable') } finally { setBusy(false) }
  }

  /** Apply an animated slide (edit.slide) — slide the overlay in from / out to the
   *  chosen edge over `slideMs`. The verb lowers to a pos_x/pos_y keyframe track on
   *  the clip, so the COMPOSED preview shows the motion. */
  const applySlide = async () => {
    if (!place.found || !clipId || place.locked || busy) return
    setBusy(true); setErr(null); setNote(null)
    try {
      const r = await callVerb('edit.slide', {
        clip: clipId,
        edge: slideEdge,
        mode: slideMode,
        slide_ms: Math.round(slideMs),
        rationale: `user: slide ${slideMode} from ${slideEdge}`,
      })
      if (r.ok) {
        setNote(`Slide ${slideMode === 'in' ? 'in' : 'out'} (${slideEdge}, ${(slideMs / 1000).toFixed(2)}s) — scrub the COMPOSED preview to see the overlay move.`)
        document.dispatchEvent(new CustomEvent('cut:show-composed'))
      } else {
        setErr(`${r.error?.code ?? 'failed'}: ${r.error?.message ?? 'edit.slide failed'}`)
      }
    } catch { setErr('server unreachable') } finally { setBusy(false) }
  }

  // --- Keyframes (edit.keyframe): the selected param's points, sorted by time.
  const kfMeta = KF_PARAMS.find((p) => p.value === kfParam)!
  const kfPoints = useMemo(
    () => (place.keyframes.find((k) => k.param === kfParam)?.points ?? []).slice().sort((a, b) => a.t_ms - b.t_ms),
    [place.keyframes, kfParam],
  )
  /** SET the selected param's full point list (edit.keyframe replaces the track),
   *  with the chosen easing curve (one interp per track). `interp` defaults to the
   *  current selection but is passable so the easing dropdown can re-apply on change
   *  without waiting for the async state update. */
  const applyKf = async (points: { t_ms: number; value: number }[], interp: KfInterp = kfInterp) => {
    if (!place.found || !clipId || place.locked || busy) return
    setBusy(true); setErr(null); setNote(null)
    try {
      const r = await callVerb('edit.keyframe', { clip: clipId, param: kfParam, points, interp, rationale: `user: keyframe ${kfParam} (${interp})` })
      if (r.ok) {
        setNote(points.length ? `Keyframed ${kfParam} — ${points.length} point${points.length === 1 ? '' : 's'} (${interp}).` : `Cleared ${kfParam} keyframes.`)
        document.dispatchEvent(new CustomEvent('cut:show-composed'))
      } else setErr(`${r.error?.code ?? 'failed'}: ${r.error?.message ?? 'edit.keyframe failed'}`)
    } catch { setErr('server unreachable') } finally { setBusy(false) }
  }
  /** Add/replace a point at the chosen time with the chosen value. */
  const addKfPoint = () => {
    const t = Math.round(kfTime)
    const pts = kfPoints.filter((p) => p.t_ms !== t).concat([{ t_ms: t, value: +kfValue.toFixed(3) }]).sort((a, b) => a.t_ms - b.t_ms)
    void applyKf(pts)
  }
  const removeKfPoint = (t: number) => void applyKf(kfPoints.filter((p) => p.t_ms !== t))
  const clearKf = () => void applyKf([])
  /** Switch the edited param + reset the value input to that param's default, and
   *  seed the easing select from that param's stored track (so it reflects reality). */
  const switchKfParam = (p: LayerKfParam) => {
    setKfParam(p)
    setKfValue(KF_PARAMS.find((m) => m.value === p)!.def)
    setKfInterp(place.keyframes.find((k) => k.param === p)?.interp ?? 'linear')
  }
  /** Change the easing curve. If the track already has points, re-apply them with
   *  the new interp immediately (one interp per track). */
  const switchKfInterp = (i: KfInterp) => {
    setKfInterp(i)
    if (kfPoints.length > 0) void applyKf(kfPoints, i)
  }

  const pct = (n: number) => `${Math.round(n * 100)}%`

  return (
    <div className="cd-scrim" data-cut-layer-scrim onMouseDown={overlay.onScrimMouseDown}>
      <aside
        ref={overlay.dialogRef}
        className="cd-drawer"
        data-cut-layer
        data-cut-layer-open="true"
        role="dialog"
        aria-modal="true"
        aria-label="Layer / picture-in-picture"
        data-cut-blocking-overlay
        tabIndex={-1}
        onKeyDown={overlay.onDialogKeyDown}
        onMouseDown={(e) => e.stopPropagation()}
      >
        <header className="cd-head">
          <div>
            <h2 className="cd-title">Layer / PiP</h2>
            <p className="cd-sub">Position, crop, resize, reorder, reverse, freeze, or animate an overlay layer.</p>
          </div>
          <button className="cd-btn cd-btn--ghost" data-cut-layer-close onClick={onClose}>Close</button>
        </header>

        <div className="cd-body">
          {!place.found ? (
            <div className="cd-empty" data-cut-layer-empty>
              Select a video clip on the timeline to edit its layer.
            </div>
          ) : (
            <>
              <p className="cd-note" data-cut-layer-clip>
                Layer for clip <code>{clipId}</code> on track <code>{place.trackId}</code>.
              </p>
              {place.isBaseVideo && (
                <div className="cd-note cd-note--warn" data-cut-layer-base-note>
                  This is the <strong>base layer</strong> (the bottom of the stack). Position and scale place it over the black canvas; moving it forward makes it composite over another video layer.
                </div>
              )}
              {place.locked && (
                <div className="cd-note cd-note--warn" data-cut-layer-locked-note>
                  This track is locked. Unlock it in the timeline before changing its clips or stacking order.
                </div>
              )}

              <fieldset data-cut-layer-edit-fieldset disabled={busy || place.locked} style={{ border: 0, padding: 0, margin: 0, minWidth: 0 }}>

              <LayerSlider label="Scale (size)" attr="scale" value={scale} set={setScale} min={0.1} max={1} step={0.01} fmt={pct} />
              <LayerSlider label="X (left→right)" attr="x" value={x} set={setX} min={0} max={1} step={0.01} fmt={pct} />
              <LayerSlider label="Y (top→bottom)" attr="y" value={y} set={setY} min={0} max={1} step={0.01} fmt={pct} />
              <LayerSlider label="Opacity" attr="opacity" value={opacity} set={setOpacity} min={0} max={1} step={0.01} fmt={pct} />

              <div style={{ display: 'flex', alignItems: 'center', gap: 'var(--space-3)' }}>
                <button
                  className="cd-btn cd-btn--primary"
                  data-cut-layer-apply
                  disabled={busy}
                  onClick={() => void applyTransform()}
                  style={{ flex: 1 }}
                >
                  {busy ? 'Applying…' : 'Apply layer'}
                </button>
                <button className="cd-reset" data-cut-layer-reset onClick={reset} type="button">
                  Reset (full-frame)
                </button>
              </div>

              {/* Crop — keep a source rectangle in pixels, e.g. trim a baked-in
                  letterbox or combine elements from two sources. crop -> conform
                  -> transform, so it composes with the PiP transform above. */}
              <div className="cd-field" data-cut-layer-crop-section>
                <span className="cd-field-label">
                  Crop (source rectangle)
                  {dims && <span className="cd-val" data-cut-layer-val="crop_src">{dims.w}×{dims.h}px</span>}
                </span>
                {!dims ? (
                  <div className="cd-note" data-cut-layer-crop-pending>
                    Waiting for the asset probe — crop needs the source dimensions. Re-open once the clip shows its size.
                  </div>
                ) : (
                  <>
                    <LayerSlider label="Crop X (left)" attr="crop_x" value={cropX} set={setCropX} min={0} max={dims.w} step={1} fmt={(n) => `${Math.round(n)}px`} />
                    <LayerSlider label="Crop Y (top)" attr="crop_y" value={cropY} set={setCropY} min={0} max={dims.h} step={1} fmt={(n) => `${Math.round(n)}px`} />
                    <LayerSlider label="Crop W (width)" attr="crop_w" value={cropW} set={setCropW} min={1} max={dims.w} step={1} fmt={(n) => `${Math.round(n)}px`} />
                    <LayerSlider label="Crop H (height)" attr="crop_h" value={cropH} set={setCropH} min={1} max={dims.h} step={1} fmt={(n) => `${Math.round(n)}px`} />
                    <div style={{ display: 'flex', alignItems: 'center', gap: 'var(--space-3)' }}>
                      <button
                        className="cd-btn cd-btn--primary"
                        data-cut-layer-crop-apply
                        disabled={busy}
                        onClick={() => void applyCrop()}
                        style={{ flex: 1 }}
                      >
                        {busy ? 'Applying…' : 'Apply crop'}
                      </button>
                      <button className="cd-reset" data-cut-layer-crop-reset onClick={resetCrop} type="button">
                        Reset (whole frame)
                      </button>
                    </div>
                  </>
                )}
              </div>

              {/* Motion — reverse / freeze / Ken Burns (edit.reverse / edit.freeze
                  / edit.animate). Per-clip playback + animation; CPU-only at render. */}
              <div className="cd-field" data-cut-layer-motion-section>
                <span className="cd-field-label">Motion (reverse · freeze · Ken Burns)</span>

                {/* Reverse — a plain toggle. */}
                <label className="cd-field" style={{ flexDirection: 'row', alignItems: 'center', gap: 'var(--space-2)' }}>
                  <input
                    type="checkbox"
                    data-cut-layer-reverse
                    checked={reverse}
                    disabled={busy}
                    onChange={(e) => void applyReverse(e.target.checked)}
                  />
                  <span className="cd-field-label" style={{ margin: 0 }}>Reverse (play backward)</span>
                </label>

                {/* Freeze — hold one frame at a chosen offset for the whole slot. */}
                <label className="cd-field" style={{ flexDirection: 'row', alignItems: 'center', gap: 'var(--space-2)' }}>
                  <input
                    type="checkbox"
                    data-cut-layer-freeze
                    checked={freezeOn}
                    disabled={busy}
                    onChange={(e) => void applyFreeze(e.target.checked)}
                  />
                  <span className="cd-field-label" style={{ margin: 0 }}>Freeze frame (hold the picture)</span>
                </label>
                {place.srcSpanMs > 0 && (
                  <LayerSlider
                    label="Freeze at (into clip)"
                    attr="freeze_at"
                    value={freezeAt}
                    set={setFreezeAt}
                    min={0}
                    max={place.srcSpanMs}
                    step={10}
                    fmt={(n) => `${(n / 1000).toFixed(2)}s`}
                  />
                )}

                {/* Ken Burns — preset + amount + apply/clear. */}
                <label className="cd-field">
                  <span className="cd-field-label">Ken Burns pan / zoom</span>
                  <select
                    className="cd-range"
                    data-cut-layer-kenburns-preset
                    value={preset}
                    disabled={busy}
                    onChange={(e) => setPreset(e.target.value as KenBurnsPreset)}
                  >
                    {KEN_BURNS_PRESETS.map((p) => (
                      <option key={p.value} value={p.value}>{p.label}</option>
                    ))}
                  </select>
                </label>
                <LayerSlider label="Amount" attr="kenburns_amount" value={amount} set={setAmount} min={0.05} max={1} step={0.05} fmt={(n) => `${n.toFixed(2)}×`} />
                <div style={{ display: 'flex', alignItems: 'center', gap: 'var(--space-3)' }}>
                  <button
                    className="cd-btn cd-btn--primary"
                    data-cut-layer-kenburns-apply
                    disabled={busy}
                    onClick={() => void applyAnimate()}
                    style={{ flex: 1 }}
                  >
                    {busy ? 'Applying…' : 'Apply Ken Burns'}
                  </button>
                  <button
                    className="cd-reset"
                    data-cut-layer-kenburns-clear
                    onClick={() => void clearAnimate()}
                    type="button"
                    disabled={busy || !hasAnim}
                  >
                    Clear
                  </button>
                </div>

                {/* Animated slide-in / slide-out (edit.slide) — the easy path vs
                    hand-authoring pos_x/pos_y keyframes. Pick an edge + in/out +
                    duration; the verb resolves it to a position-keyframe track. */}
                <div className="cd-field" data-cut-layer-slide-section>
                  <span className="cd-field-label">Slide in / out (animated PiP)</span>
                  <div className="cd-row">
                    <label className="cd-field">
                      <span className="cd-field-label">Edge</span>
                      <select
                        className="cd-range"
                        data-cut-layer-slide-edge
                        value={slideEdge}
                        disabled={busy}
                        onChange={(e) => setSlideEdge(e.target.value as 'left' | 'right' | 'top' | 'bottom')}
                      >
                        <option value="left">From left</option>
                        <option value="right">From right</option>
                        <option value="top">From top</option>
                        <option value="bottom">From bottom</option>
                      </select>
                    </label>
                    <label className="cd-field">
                      <span className="cd-field-label">Direction</span>
                      <select
                        className="cd-range"
                        data-cut-layer-slide-mode
                        value={slideMode}
                        disabled={busy}
                        onChange={(e) => setSlideMode(e.target.value as 'in' | 'out')}
                      >
                        <option value="in">Slide in (enter)</option>
                        <option value="out">Slide out (exit)</option>
                      </select>
                    </label>
                  </div>
                  <LayerSlider
                    label="Slide duration"
                    attr="slide_ms"
                    value={slideMs}
                    set={setSlideMs}
                    min={100}
                    max={Math.max(2000, place.srcSpanMs || 2000)}
                    step={50}
                    fmt={(n) => `${(n / 1000).toFixed(2)}s`}
                  />
                  <button
                    className="cd-btn cd-btn--primary"
                    data-cut-action="edit-slide"
                    data-cut-layer-slide-apply
                    disabled={busy}
                    onClick={() => void applySlide()}
                    style={{ width: '100%' }}
                  >
                    {busy ? 'Applying…' : `Apply slide ${slideMode}`}
                  </button>
                </div>
              </div>

              {/* Keyframes — animate opacity / position over the clip (edit.keyframe).
                  A point list + a curve sparkline; SET semantics (sending the full
                  list replaces that param's track). CPU-only at render. */}
              <div className="cd-field" data-cut-layer-keyframes-section>
                <span className="cd-field-label">Keyframes (animate over time)</span>
                <label className="cd-field">
                  <span className="cd-field-label">Parameter</span>
                  <select
                    className="cd-range"
                    data-cut-layer-kf-param
                    value={kfParam}
                    disabled={busy}
                    onChange={(e) => switchKfParam(e.target.value as LayerKfParam)}
                  >
                    {KF_PARAMS.map((p) => (
                      <option key={p.value} value={p.value}>{p.label}</option>
                    ))}
                  </select>
                </label>

                {/* Easing curve (one interp per track) — eased motion reads
                    professional; changing it re-applies to the existing points. */}
                <label className="cd-field">
                  <span className="cd-field-label">Easing</span>
                  <select
                    className="cd-range"
                    data-cut-layer-kf-interp
                    value={kfInterp}
                    disabled={busy}
                    onChange={(e) => switchKfInterp(e.target.value as KfInterp)}
                  >
                    {KF_INTERPS.map((i) => (
                      <option key={i.value} value={i.value}>{i.label}</option>
                    ))}
                  </select>
                </label>

                {/* Curve sparkline of the current points (clip time → value). */}
                <KfCurve points={kfPoints} maxT={Math.max(1, place.timelineSpanMs)} lo={kfMeta.lo} hi={kfMeta.hi} />

                {place.timelineSpanMs > 0 && (
                  <LayerSlider label="Time (into clip)" attr="kf_time" value={kfTime} set={setKfTime} min={0} max={place.timelineSpanMs} step={10} fmt={(n) => `${(n / 1000).toFixed(2)}s`} />
                )}
                <LayerSlider label="Value" attr="kf_value" value={kfValue} set={setKfValue} min={kfMeta.lo} max={kfMeta.hi} step={0.01} fmt={(n) => n.toFixed(2)} />
                <div style={{ display: 'flex', alignItems: 'center', gap: 'var(--space-3)' }}>
                  <button className="cd-btn cd-btn--primary" data-cut-layer-kf-add disabled={busy} onClick={addKfPoint} style={{ flex: 1 }}>
                    {busy ? 'Saving…' : 'Add / update point'}
                  </button>
                  <button className="cd-reset" data-cut-layer-kf-clear type="button" disabled={busy || kfPoints.length === 0} onClick={clearKf}>
                    Clear
                  </button>
                </div>

                {/* The point list — each row removable. */}
                {kfPoints.length > 0 && (
                  <div data-cut-layer-kf-points style={{ display: 'flex', flexDirection: 'column', gap: '2px', marginTop: 'var(--space-2)' }}>
                    {kfPoints.map((p) => (
                      <div key={p.t_ms} style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', gap: 'var(--space-2)', fontSize: '12px' }}>
                        <code style={{ opacity: 0.85 }}>{(p.t_ms / 1000).toFixed(2)}s → {p.value.toFixed(2)}</code>
                        <button className="cd-reset" data-cut-layer-kf-remove={p.t_ms} type="button" disabled={busy} onClick={() => removeKfPoint(p.t_ms)} title="Remove this point"><Icon name="close" size={14} label="Remove this point" /></button>
                      </div>
                    ))}
                  </div>
                )}
              </div>

              {/* Stacking order — video track order IS the z-order. */}
              <div className="cd-field">
                <span className="cd-field-label">Stacking order</span>
                <div style={{ display: 'flex', gap: 'var(--space-2)' }}>
                  <button
                    className="cd-btn cd-btn--ghost"
                    data-cut-layer-forward
                    disabled={busy || !order?.canMoveForward}
                    onClick={() => void reorder(1)}
                    style={{ flex: 1 }}
                  >
                    ↑ Bring forward
                  </button>
                  <button
                    className="cd-btn cd-btn--ghost"
                    data-cut-layer-back
                    disabled={busy || !order?.canMoveBack}
                    onClick={() => void reorder(-1)}
                    style={{ flex: 1 }}
                  >
                    ↓ Send back
                  </button>
                </div>
              </div>
              </fieldset>

              <button className="cd-btn cd-btn--ghost" data-cut-layer-add onClick={() => void addLayer()} disabled={busy} style={{ width: '100%' }}>
                + Add video layer
              </button>

              <p className="cd-note">The preview at the playhead shows the composed layered result.</p>

              {err && <div className="cd-err" data-cut-layer-error role="alert">{err}</div>}
              {note && <div className="cd-result" data-cut-layer-note><div className="cd-result-head">{note}</div></div>}
            </>
          )}
        </div>
      </aside>
    </div>
  )
}
