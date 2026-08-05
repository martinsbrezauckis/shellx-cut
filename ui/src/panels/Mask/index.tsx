// panels/Mask — the human UI for edit.add_mask / edit.redact: a REGION-blur /
// privacy-mask drawer. The engine verbs shipped + pass every coverage gate, but a
// human needs presets and duration controls rather than raw verb names. This adds
// the on-canvas drawing UI plus beginner-facing privacy workflows.
//
// SHAPE: a DOCKED side panel, NOT the cd-scrim drawer the other tools use. A scrim
// is a full-screen backdrop with onMouseDown=close, so it would intercept the very
// first draw gesture on the preview and close the panel — the same reason the
// Inspector's "Draw region" works only because it lives in the scrim-less rail. So
// the Mask panel docks at the right (mirroring the cd-* internals: cd-head / cd-body
// / cd-btn / cd-err) and leaves the preview fully interactive for drawing.
//
// FLOW (drawer owns the controls + the verb; the preview owns the geometry):
//   • On open with a BASE-track video clip selected, it ARMS the preview draw
//     (cut:mask-draw {active, clip, shape, nonce}) and seeks the playhead onto the
//     clip so what's being masked is visible.
//   • The user draws a rect / ellipse / polygon on the preview (Preview/MaskOverlay,
//     the TransformHandles pattern); the overlay reports the verb-ready geometry back
//     (cut:mask-geometry) — we cache it and gate Apply on `ready`.
//   • Apply → edit.add_mask for whole-clip masks or edit.redact{range_ms} for
//     timed privacy. The panel flips the preview to the COMPOSED frame
//     (cut:show-composed) so the blurred / pixelated / blacked region is visible.
//
// HONEST surfacing: masks render on the BASE (first) video track only (the engine
// refuses an overlay clip), so a non-base / no selection shows an inline hint and
// disables Apply. Errors are never swallowed (cd-err). Every control carries a
// data-cut-mask-* hook for the Debug API + the full-coverage gate.
//
// Callers: App.tsx (activeDrawer === 'mask'). Deps: lib/client (callVerb), Timeline/
// layout (clip placement → seek), Preview/MaskOverlay (the shared geometry types),
// ../drawer.css (cd-* internals), ./mask.css (the docked shell + .mk-* overlay).

import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { callVerb, type Project } from '../../lib/client'
import { layoutTrack } from '../Timeline/layout'
import type { MaskShape, MaskGeometry } from '../Preview/MaskOverlay'
import { Icon } from '../../icons'
import '../drawer.css'
import './mask.css'

export interface MaskDrawerProps {
  project: Project | null
  /** The currently-selected clip id (App passes selectedClipIds[0]). */
  clipId: string | null
  playheadMs: number
  /** Seek the playhead (so the masked clip is visible while drawing). */
  onSeek: (atMs: number) => void
  onClose: () => void
}

type Effect = 'blur' | 'pixelate' | 'black'
type RedactMode = 'blur' | 'pixelate' | 'box'
type MaskPreset = 'face' | 'rectangle' | 'plate' | 'custom'
type DurationMode = 'whole' | 'timed'
type Result =
  | { kind: 'applied'; effect: Effect; shape: MaskShape; ops: number; timed: boolean; rangeMs?: [number, number] }
  | { kind: 'cleared' }

const SHAPES: { id: MaskShape; label: string }[] = [
  { id: 'rect', label: 'Rectangle' },
  { id: 'ellipse', label: 'Ellipse' },
  { id: 'polygon', label: 'Polygon' },
]
const MASK_PRESETS: { id: MaskPreset; label: string; shape: MaskShape; effect: Effect; strength: number; feather: number; hint: string }[] = [
  { id: 'face', label: 'Blur face', shape: 'ellipse', effect: 'blur', strength: 25, feather: 0.03, hint: 'Draw an oval around a face.' },
  { id: 'rectangle', label: 'Blur rectangle', shape: 'rect', effect: 'blur', strength: 18, feather: 0.01, hint: 'Drag a box over a screen area, label, or object.' },
  { id: 'plate', label: 'Hide plate/text', shape: 'rect', effect: 'black', strength: 0, feather: 0, hint: 'Cover a license plate, address, or visible private text.' },
  { id: 'custom', label: 'Custom', shape: 'polygon', effect: 'pixelate', strength: 16, feather: 0, hint: 'Choose shape and effect yourself.' },
]
const EFFECTS: { id: Effect; label: string }[] = [
  { id: 'blur', label: 'Blur' },
  { id: 'pixelate', label: 'Pixelate' },
  { id: 'black', label: 'Black box' },
]
const MASK_EFFECT_COPY: Record<Effect, string> = {
  blur: 'blurred',
  pixelate: 'pixelated',
  black: 'blacked out',
}
// Per-effect strength: blur = gaussian sigma px (default 15), pixelate = mosaic
// block px (default 16); black ignores strength (a solid censor).
const STRENGTH: Record<Effect, { def: number; min: number; max: number; step: number; unit: string }> = {
  blur: { def: 15, min: 1, max: 60, step: 1, unit: 'px sigma' },
  pixelate: { def: 16, min: 4, max: 64, step: 1, unit: 'px block' },
  black: { def: 0, min: 0, max: 0, step: 1, unit: '' },
}
const clamp = (value: number, min: number, max: number) => Math.max(min, Math.min(max, value))

export default function MaskDrawer({ project, clipId, playheadMs, onSeek, onClose }: MaskDrawerProps) {
  const [preset, setPreset] = useState<MaskPreset>('rectangle')
  const [shape, setShape] = useState<MaskShape>('rect')
  const [effect, setEffect] = useState<Effect>('blur')
  const [strength, setStrength] = useState<number>(STRENGTH.blur.def)
  const [feather, setFeather] = useState<number>(0)
  const [invert, setInvert] = useState<boolean>(false)
  const [durationMode, setDurationMode] = useState<DurationMode>('whole')
  const [durationSeconds, setDurationSeconds] = useState<number>(5)
  const [geometry, setGeometry] = useState<MaskGeometry | null>(null)
  const [drawNonce, setDrawNonce] = useState(0)
  const [busy, setBusy] = useState(false)
  const [err, setErr] = useState<string | null>(null)
  const [result, setResult] = useState<Result | null>(null)

  // Resolve the BASE (first) video track + whether the selection sits on it — masks
  // render on the base track only (the engine refuses an overlay clip with a clear
  // error, so we gate up front rather than fail on Apply).
  const baseTrack = useMemo(() => project?.tracks.find((t) => t.kind === 'video') ?? null, [project])
  const baseClipIds = useMemo(() => {
    const s = new Set<string>()
    for (const c of baseTrack?.clips ?? []) if ('id' in c) s.add(c.id)
    return s
  }, [baseTrack])
  const onBase = !!clipId && baseClipIds.has(clipId)
  const clip = onBase ? clipId : null
  const selectedButNotBase = !!clipId && !onBase

  const clipPlacement = useMemo(() => {
    if (!clip || !baseTrack) return null
    return layoutTrack(baseTrack).find((i) => i.id === clip) ?? null
  }, [clip, baseTrack])

  const timedRange = useMemo<[number, number] | null>(() => {
    if (durationMode !== 'timed' || !clipPlacement) return null
    const start = Math.round(clamp(playheadMs - clipPlacement.startMs, 0, clipPlacement.durMs))
    const durationMs = Math.round(clamp(durationSeconds, 0.5, 120) * 1000)
    const end = Math.min(clipPlacement.durMs, start + durationMs)
    return end > start ? [start, end] : null
  }, [durationMode, clipPlacement, playheadMs, durationSeconds])

  const durationSummary = timedRange
    ? `${(timedRange[0] / 1000).toFixed(1)}s to ${(timedRange[1] / 1000).toFixed(1)}s in this clip`
    : 'Move the playhead inside the selected clip.'

  // Esc closes (docked panel — no scrim to click away).
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => { if (e.key === 'Escape') onClose() }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [onClose])

  // Receive the live geometry the preview overlay draws (verb-ready points + ready).
  useEffect(() => {
    const onGeom = (e: Event) => setGeometry((e as CustomEvent).detail as MaskGeometry)
    document.addEventListener('cut:mask-geometry', onGeom)
    return () => document.removeEventListener('cut:mask-geometry', onGeom)
  }, [])

  // ARM the preview draw for the active clip + shape; re-arm on a shape change or a
  // "Clear shape" (drawNonce) — the nonce keys a fresh MaskOverlay so the shape resets.
  // Disarm on close / no-base-clip. Mirrors the Inspector's cut:redact-draw arm.
  const nonceForArm = drawNonce
  useEffect(() => {
    if (!clip) {
      document.dispatchEvent(new CustomEvent('cut:mask-draw', { detail: { active: false } }))
      return
    }
    document.dispatchEvent(new CustomEvent('cut:mask-draw', { detail: { active: true, clip, shape, nonce: nonceForArm } }))
    return () => { document.dispatchEvent(new CustomEvent('cut:mask-draw', { detail: { active: false } })) }
  }, [clip, shape, nonceForArm])

  // On a new active clip, seek the playhead onto it so the stage shows the clip being
  // masked (the draw surface + the Apply result are then both visible). Once per clip.
  const seekedFor = useRef<string | null>(null)
  useEffect(() => {
    if (!clip || !baseTrack || seekedFor.current === clip) return
    if (clipPlacement) { onSeek(Math.round(clipPlacement.startMs + Math.min(250, clipPlacement.durMs / 2))); seekedFor.current = clip }
  }, [clip, baseTrack, clipPlacement, onSeek])

  const applyPreset = (p: (typeof MASK_PRESETS)[number]) => {
    setPreset(p.id)
    if (p.shape !== shape) {
      setGeometry(null)
      setDrawNonce((n) => n + 1)
    }
    setShape(p.shape)
    setEffect(p.effect)
    setStrength(p.strength)
    setFeather(p.feather)
    setInvert(false)
    setErr(null)
    setResult(null)
  }
  const changeShape = (s: MaskShape) => {
    if (s === shape) return
    setPreset('custom')
    setShape(s)
    setGeometry(null)
    setResult(null)
  }
  const changeEffect = (next: Effect) => {
    if (next === effect) return
    setPreset('custom')
    setEffect(next)
    setStrength(STRENGTH[next].def)
    setResult(null)
  }
  const clearShape = () => { setGeometry(null); setResult(null); setDrawNonce((n) => n + 1) }

  const ready = !!geometry?.ready
  const canApply = !!clip && ready && !busy && (durationMode === 'whole' || !!timedRange)

  // Apply → edit.add_mask for the whole clip, or edit.redact with range_ms for a
  // privacy mask that starts at the current playhead.
  const apply = useCallback(async () => {
    if (!clip || !geometry?.ready || busy) return
    setBusy(true); setErr(null); setResult(null)
    try {
      let r
      if (durationMode === 'timed' && timedRange) {
        const mode: RedactMode = effect === 'black' ? 'box' : effect
        const base = {
          clip,
          shape: geometry.shape,
          points: geometry.points,
          feather: +feather.toFixed(4),
          invert,
          mode,
          range_ms: timedRange,
          rationale: `mask drawer: timed ${invert ? 'inverted ' : ''}${mode} ${geometry.shape} region`,
        }
        r = await callVerb('edit.redact', effect !== 'black' ? { ...base, strength } : base)
      } else {
        // black is a solid censor (no strength); blur/pixelate carry their strength.
        const base = {
          clip,
          shape: geometry.shape,
          points: geometry.points,
          feather: +feather.toFixed(4),
          invert,
          effect,
          rationale: `mask drawer: ${invert ? 'inverted ' : ''}${effect} ${geometry.shape} region`,
        }
        r = await callVerb('edit.add_mask', effect !== 'black' ? { ...base, strength } : base)
      }
      setBusy(false)
      if (r.ok) {
        setResult({ kind: 'applied', effect, shape: geometry.shape, ops: r.op_ids?.length ?? 0, timed: durationMode === 'timed', rangeMs: timedRange ?? undefined })
        document.dispatchEvent(new CustomEvent('cut:show-composed'))
      } else {
        setErr(r.error?.message ?? r.error?.suggested_action ?? r.error?.code ?? 'mask apply failed')
      }
    } catch {
      setBusy(false)
      setErr('server unreachable')
    }
  }, [clip, geometry, busy, durationMode, timedRange, feather, invert, effect, strength])

  // Remove any mask on the clip (edit.add_mask{enabled:false}).
  const remove = useCallback(async () => {
    if (!clip || busy) return
    setBusy(true); setErr(null); setResult(null)
    try {
      const r = await callVerb('edit.add_mask', { clip, enabled: false, rationale: 'mask drawer: clear mask' })
      setBusy(false)
      if (r.ok) { setResult({ kind: 'cleared' }); document.dispatchEvent(new CustomEvent('cut:show-composed')) }
      else setErr(r.error?.message ?? r.error?.code ?? 'could not clear the mask')
    } catch {
      setBusy(false)
      setErr('server unreachable')
    }
  }, [clip, busy])

  const drawHint =
    shape === 'polygon'
      ? 'Click on the preview to drop points (3 or more); drag a point to adjust.'
      : 'Drag a region on the preview; drag a corner to resize, the body to move.'

  return (
    <aside
      className="mk-drawer"
      data-cut-panel="mask"
      data-cut-mask
      data-cut-mask-open="true"
      data-cut-mask-ready={ready ? 'true' : 'false'}
      role="dialog"
      aria-label="Mask and privacy"
    >
      <header className="cd-head">
        <div>
          <h2 className="cd-title"><Icon name="mask" size={16} tone="brand" /> Mask / privacy</h2>
          <p className="cd-sub">
            Blur, pixelate, or cover part of the selected clip. Draw the area on the preview, then apply it here.
          </p>
        </div>
        <button className="cd-btn cd-btn--ghost" data-cut-mask-close onClick={onClose}>Close</button>
      </header>

      <div className="cd-body" data-cut-mask-body>
        {err && <div className="cd-err" data-cut-mask-error role="alert">{err}</div>}

        {/* Clip status / requirement (mirrors Recipes' "open a project" hint). */}
        {!clip ? (
          <p className="cd-note cd-note--warn" data-cut-mask-noclip>
            {selectedButNotBase
              ? 'Masks render on the base (first) video track only — select a clip on the base track.'
              : 'Select a base-track video clip, then draw a region on the preview.'}
          </p>
        ) : (
          <p className="cd-note cd-note--mono" data-cut-mask-clip={clip}>
            Masking <strong>{clip}</strong> · {drawHint}
          </p>
        )}

        {/* QUICK ACTIONS. */}
        <section className="cd-field" aria-label="Quick mask action">
          <span className="cd-field-label">Quick action</span>
          <div className="mk-presets" data-cut-mask-presets>
            {MASK_PRESETS.map((p) => (
              <button
                key={p.id}
                type="button"
                className={`mk-preset ${preset === p.id ? 'mk-preset--on' : ''}`}
                data-cut-mask-preset={p.id}
                aria-pressed={preset === p.id}
                title={p.hint}
                onClick={() => applyPreset(p)}
              >
                {p.label}
              </button>
            ))}
          </div>
        </section>

        {/* SHAPE kind. */}
        <label className="cd-field">
          <span className="cd-field-label">Shape</span>
          <div className="cd-seg" data-cut-mask-shape-seg role="group" aria-label="Mask shape">
            {SHAPES.map((s) => (
              <button
                key={s.id}
                type="button"
                className={`cd-seg-btn ${shape === s.id ? 'cd-seg-btn--on' : ''}`}
                data-cut-mask-shape-kind={s.id}
                aria-pressed={shape === s.id}
                onClick={() => changeShape(s.id)}
              >
                {s.label}
              </button>
            ))}
          </div>
        </label>

        {/* Draw status + clear-shape. */}
        <div className="mk-drawstatus" data-cut-mask-drawstatus={ready ? 'ready' : 'empty'}>
          <span className={`mk-dot ${ready ? 'mk-dot--on' : ''}`} aria-hidden="true" />
          <span className="mk-drawstatus-text">
            {ready ? 'Region drawn — ready to apply' : 'No region yet — draw one on the preview'}
          </span>
          <button
            type="button"
            className="cd-btn cd-btn--ghost cd-btn--sm"
            data-cut-mask-clear-shape
            disabled={!geometry?.points.length}
            onClick={clearShape}
            title="Discard the drawn region and start over"
          >
            Clear shape
          </button>
        </div>

        {/* EFFECT. */}
        <label className="cd-field">
          <span className="cd-field-label">Effect</span>
          <div className="cd-seg" data-cut-mask-effect-seg role="group" aria-label="Mask effect">
            {EFFECTS.map((eo) => (
              <button
                key={eo.id}
                type="button"
                className={`cd-seg-btn ${effect === eo.id ? 'cd-seg-btn--on' : ''}`}
                data-cut-mask-effect={eo.id}
                aria-pressed={effect === eo.id}
                onClick={() => changeEffect(eo.id)}
              >
                {eo.label}
              </button>
            ))}
          </div>
        </label>

        {/* STRENGTH (blur/pixelate only — black is a solid censor). */}
        {effect !== 'black' && (
          <label className="cd-field">
            <span className="cd-field-label">
              Strength <span className="cd-val">{strength} {STRENGTH[effect].unit}</span>
            </span>
            <input
              className="cd-range"
              type="range"
              data-cut-mask-strength
              min={STRENGTH[effect].min}
              max={STRENGTH[effect].max}
              step={STRENGTH[effect].step}
              value={strength}
              onChange={(e) => { setPreset('custom'); setStrength(Number(e.target.value)) }}
            />
          </label>
        )}

        {/* FEATHER (soft edge, fraction of frame height). */}
        <label className="cd-field">
          <span className="cd-field-label">
            Feather <span className="cd-val">{(feather * 100).toFixed(1)}%</span>
          </span>
          <input
            className="cd-range"
            type="range"
            data-cut-mask-feather
            min={0}
            max={0.25}
            step={0.005}
            value={feather}
            onChange={(e) => { setPreset('custom'); setFeather(Number(e.target.value)) }}
          />
        </label>

        {/* INVERT. */}
        <label className="cd-toggle" data-cut-mask-invert-field>
          <input
            type="checkbox"
            data-cut-mask-invert
            checked={invert}
            onChange={(e) => { setPreset('custom'); setInvert(e.target.checked) }}
          />
          <span>Invert — affect everything <em>outside</em> the shape</span>
        </label>

        {/* DURATION. Whole clip uses edit.add_mask; timed privacy uses edit.redact. */}
        <section className="cd-field" data-cut-mask-duration aria-label="Mask duration">
          <span className="cd-field-label">Duration</span>
          <div className="cd-seg" role="group" aria-label="Mask duration mode">
            <button
              type="button"
              className={`cd-seg-btn ${durationMode === 'whole' ? 'cd-seg-btn--on' : ''}`}
              data-cut-mask-duration-mode="whole"
              aria-pressed={durationMode === 'whole'}
              onClick={() => setDurationMode('whole')}
            >
              Whole clip
            </button>
            <button
              type="button"
              className={`cd-seg-btn ${durationMode === 'timed' ? 'cd-seg-btn--on' : ''}`}
              data-cut-mask-duration-mode="timed"
              aria-pressed={durationMode === 'timed'}
              onClick={() => setDurationMode('timed')}
            >
              From playhead
            </button>
          </div>
          {durationMode === 'timed' && (
            <>
              <label className="mk-inline-field" data-cut-mask-duration-seconds>
                <span>Seconds</span>
                <input
                  className="cd-input"
                  data-cut-mask-duration-seconds-input
                  type="number"
                  min={0.5}
                  max={120}
                  step={0.5}
                  value={durationSeconds}
                  onChange={(e) => {
                    const next = Number(e.target.value)
                    if (Number.isFinite(next)) setDurationSeconds(clamp(next, 0.5, 120))
                  }}
                />
              </label>
              <p className="cd-note" data-cut-mask-duration-range>{durationSummary}</p>
            </>
          )}
        </section>

        {/* ACTIONS. */}
        <div className="mk-actions">
          <button
            type="button"
            className="cd-btn cd-btn--primary"
            data-cut-mask-apply
            disabled={!canApply}
            onClick={() => void apply()}
            title={!clip ? 'Select a base-track clip first' : !ready ? 'Draw a region on the preview first' : durationMode === 'timed' && !timedRange ? 'Move the playhead inside the selected clip first' : 'Apply mask to this clip'}
          >
            {busy ? 'Applying…' : <><Icon name="mask" size={14} /> Apply mask</>}
          </button>
          <button
            type="button"
            className="cd-btn cd-btn--ghost"
            data-cut-mask-remove
            disabled={!clip || busy}
            onClick={() => void remove()}
            title="Remove the mask from this clip"
          >
            Remove mask
          </button>
        </div>

        {/* RESULT — the op landed; the preview now shows the masked region. */}
        {result && (
          <div className="cd-result" data-cut-mask-result data-cut-mask-result-kind={result.kind}>
            <div className="cd-result-head">
              <Icon name="check" size={16} tone="success" />
              {result.kind === 'applied'
                ? `Mask applied — ${result.effect} ${result.shape} region${result.timed && result.rangeMs ? ` · ${(result.rangeMs[0] / 1000).toFixed(1)}s-${(result.rangeMs[1] / 1000).toFixed(1)}s` : ''}${result.ops ? ` · ${result.ops} op${result.ops === 1 ? '' : 's'}` : ''}`
                : 'Mask cleared from the clip'}
            </div>
            <p className="cd-result-foot">
              {result.kind === 'applied'
                ? <>The preview is showing the <strong>composed</strong> frame — the region is now {MASK_EFFECT_COPY[result.effect]}.</>
                : <>The clip is back to its original frame.</>}
            </p>
          </div>
        )}
      </div>
    </aside>
  )
}
