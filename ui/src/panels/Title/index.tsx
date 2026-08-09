// panels/Title — the title drawer (0.5.0 UI for title.add).
// Role: a right-side drawer (the MusicBed/Environment drawer family) that drives
// ONE verb — title.add — to place a native motion-graphics title (resvg per
// frame → transparent overlay) on the top-most title track over a timed span.
//
// PLACEMENT: titles use a grouped preset picker rather than a single bare button
// because the variant
// space (lower-third vs intro card, …) needs a chooser. This drawer IS that
// picker: a topbar "Title" button opens it, the preset lives inside. Medium-
// frequency casual action → fast topbar reach, the choices one level in.
//
// TRUST STORY: the drawer does NOT preview. It fires the
// verb; the generated title .mov composites through the EXISTING overlay
// pipeline, so a title TRACK appears on the timeline and the Preview poster
// shows the title at that time the moment op_applied lands. The drawer then
// shows a short receipt read straight from the verb result (the same facts).
// Relay-drivable is NOT claimed — title.add has no ui.* relay; this is one
// human client of a verb an agent calls directly.
//
// Callers: App.tsx (mounted when open). Deps: lib/client (verbs), ../drawer.css.

import { useEffect, useRef, useState } from 'react'
import { callVerb, type Project } from '../../lib/client'
import { useBlockingOverlay } from '../../components/overlay/useBlockingOverlay'
import '../drawer.css'

const clamp01 = (v: number) => Math.min(1, Math.max(0, v))
const round3 = (v: number) => Math.round(v * 1000) / 1000
/** 3×3 quick-anchor grid (normalized) — corners/edges/centre at the 10/50/90 marks. */
const ANCHORS: { id: string; x: number; y: number; align: 'left' | 'center' | 'right' }[] = [
  { id: 'tl', x: 0.1, y: 0.12, align: 'left' }, { id: 'tc', x: 0.5, y: 0.12, align: 'center' }, { id: 'tr', x: 0.9, y: 0.12, align: 'right' },
  { id: 'ml', x: 0.1, y: 0.5, align: 'left' }, { id: 'mc', x: 0.5, y: 0.5, align: 'center' }, { id: 'mr', x: 0.9, y: 0.5, align: 'right' },
  { id: 'bl', x: 0.1, y: 0.88, align: 'left' }, { id: 'bc', x: 0.5, y: 0.88, align: 'center' }, { id: 'br', x: 0.9, y: 0.88, align: 'right' },
]
type TitleAlign = (typeof ANCHORS)[number]['align']

function alignFromInput(value: string, fallback: TitleAlign): TitleAlign {
  for (const option of ANCHORS) {
    if (option.align === value) return option.align
  }
  return fallback
}

export interface TitleDrawerProps {
  project: Project | null
  /** The current playhead (ms) — seeds the default in-point. */
  defaultInMs: number
  onClose: () => void
}

/** title.add presets (schema enum). lower_third = bottom bar; title_card = big centred. */
const PRESETS = [
  { id: 'lower_third', label: 'Lower third (name/role bar)' },
  { id: 'title_card', label: 'Title card (big centred intro)' },
] as const
type Preset = (typeof PRESETS)[number]['id']

function presetFromInput(value: string, fallback: Preset): Preset {
  for (const option of PRESETS) {
    if (option.id === value) return option.id
  }
  return fallback
}

/** Animated-text TEMPLATES (schema enum) → friendly labels + which extra
 * params each look reads (accent color / an emphasis word). Mirrors the
 * title.templates catalog; the list verb can deepen this with descriptions. */
const TEMPLATES = [
  { id: 'typewriter', label: 'Typewriter — type-on reveal', accent: false, emphasis: false },
  { id: 'word_pop', label: 'Word pop — build word-by-word', accent: false, emphasis: false },
  { id: 'slide_stack', label: 'Slide stack — rows slide in', accent: false, emphasis: false },
  { id: 'kinetic_emphasis', label: 'Kinetic emphasis — highlight one word', accent: true, emphasis: true },
  { id: 'lower_third_reveal', label: 'Lower-third reveal — bar → name → title', accent: false, emphasis: false },
  { id: 'caption_karaoke', label: 'Karaoke caption — fill word-by-word', accent: true, emphasis: false },
] as const
type TemplateId = (typeof TEMPLATES)[number]['id']

function templateFromInput(value: string, fallback: TemplateId): TemplateId {
  for (const option of TEMPLATES) {
    if (option.id === value) return option.id
  }
  return fallback
}
/** Placement/look mode: a fixed preset position, an animated template, or free. */
type Mode = 'preset' | 'animated' | 'free'

/** The verb result we surface as the post-fire receipt (title.add). */
interface TitleResult {
  title_track: string
  asset_id: string
  clip_id: string
  preset: string
  template?: string | null
  range_ms: [number, number]
}

const DEFAULT_DUR_S = 3 // a title card / lower-third reads for ~3s by default

export default function TitleDrawer({ project, defaultInMs, onClose }: TitleDrawerProps) {
  const [text, setText] = useState('')
  const [preset, setPreset] = useState<Preset>('lower_third')
  // Look mode: a fixed preset position, an animated template, or free
  // placement. The pad drags the anchor in free mode; the 3x3 grid snaps it.
  const [mode, setMode] = useState<Mode>('preset')
  const blocking = mode !== 'free'
  const overlay = useBlockingOverlay<HTMLElement>(onClose, blocking)
  // Animated-template controls: the template + its optional accent / word.
  const [template, setTemplate] = useState<TemplateId>('typewriter')
  const [accent, setAccent] = useState('#FFD24A')
  const [emphasis, setEmphasis] = useState('')
  const tplInfo = TEMPLATES.find((t) => t.id === template)
  const [posX, setPosX] = useState(0.5)
  const [posY, setPosY] = useState(0.85) // a lower-third-ish default
  const [align, setAlign] = useState<TitleAlign>('center')
  const padRef = useRef<HTMLDivElement>(null)
  // Times in SECONDS for the human; converted to ms for the verb. Default span =
  // [playhead, playhead+3s].
  const [inS, setInS] = useState<number>(Math.max(0, defaultInMs / 1000))
  const [outS, setOutS] = useState<number>(Math.max(0, defaultInMs / 1000) + DEFAULT_DUR_S)
  const [busy, setBusy] = useState(false)
  const [result, setResult] = useState<TitleResult | null>(null)
  const [err, setErr] = useState<string | null>(null)

  // Free placement deliberately leaves the Preview interactive, so only that
  // non-modal mode keeps a small document-level Escape owner.
  useEffect(() => {
    if (blocking) return
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        e.preventDefault()
        onClose()
      }
    }
    // Capture keeps Escape reliable even when focus is on the interactive
    // Preview ghost or another editor surface that stops bubbling shortcuts.
    window.addEventListener('keydown', onKey, { capture: true })
    return () => window.removeEventListener('keydown', onKey, { capture: true })
  }, [blocking, onClose])

  // #193b: drag the title DIRECTLY on the Preview. While in "Place anywhere" mode we
  // broadcast the current placement; the Preview renders a draggable ghost of the title
  // on the frame and sends back normalized x/y as the user drags it. So the user places
  // the title on the actual picture, not just the abstract pad.
  useEffect(() => {
    if (mode !== 'free') {
      document.dispatchEvent(new CustomEvent('cut:title-place', { detail: { active: false } }))
      return
    }
    document.dispatchEvent(
      new CustomEvent('cut:title-place', {
        detail: { active: true, x: posX, y: posY, text: text.trim() || 'Title', align },
      }),
    )
  }, [mode, posX, posY, text, align])
  // Drop the ghost when the drawer closes.
  useEffect(() => {
    return () => {
      document.dispatchEvent(new CustomEvent('cut:title-place', { detail: { active: false } }))
    }
  }, [])
  // The Preview drags the ghost → update our position (drives the pad + the fired x/y).
  useEffect(() => {
    const onMove = (e: Event) => {
      const d = (e as CustomEvent).detail as { x?: number; y?: number }
      if (typeof d?.x === 'number') setPosX(clamp01(d.x))
      if (typeof d?.y === 'number') setPosY(clamp01(d.y))
    }
    document.addEventListener('cut:title-place-move', onMove)
    return () => document.removeEventListener('cut:title-place-move', onMove)
  }, [])

  const inMs = Math.round(inS * 1000)
  const outMs = Math.round(outS * 1000)
  const rangeValid = outMs > inMs
  const canFire = !!project && text.trim().length > 0 && rangeValid

  // Position pad: set the anchor from a pointer position (capture keeps the drag
  // alive outside the pad). Y is NOT inverted — top of the pad = top of frame.
  const setPosFromEvent = (clientX: number, clientY: number) => {
    const r = padRef.current?.getBoundingClientRect()
    if (!r || r.width === 0) return
    setPosX(clamp01((clientX - r.left) / r.width))
    setPosY(clamp01((clientY - r.top) / r.height))
  }
  const onPadDown = (e: React.PointerEvent) => {
    e.currentTarget.setPointerCapture(e.pointerId)
    setPosFromEvent(e.clientX, e.clientY)
  }
  const onPadMove = (e: React.PointerEvent) => {
    if (e.buttons !== 1) return // only while the primary button is held
    setPosFromEvent(e.clientX, e.clientY)
  }

  const fire = async () => {
    if (!canFire) return
    setBusy(true)
    setErr(null)
    setResult(null)
    try {
      const label = text.trim().slice(0, 40)
      // Args differ by mode: animated → a template (+ accent/emphasis where the
      // look uses them); free → x/y/align (engine: x+y present → free_title);
      // preset → the fixed preset position (default).
      const modeArgs =
        mode === 'free'
          ? { x: round3(posX), y: round3(posY), align }
          : mode === 'animated'
            ? {
                template,
                ...(tplInfo?.accent ? { accent } : {}),
                ...(template === 'kinetic_emphasis' && emphasis.trim()
                  ? { emphasis: emphasis.trim() }
                  : {}),
              }
            : { preset }
      const rationale =
        mode === 'free'
          ? `user: free title "${label}" @ (${posX.toFixed(2)},${posY.toFixed(2)}) ${inS}s–${outS}s`
          : mode === 'animated'
            ? `user: ${template} title "${label}" ${inS}s–${outS}s`
            : `user: ${preset} "${label}" @ ${inS}s–${outS}s`
      const r = await callVerb('title.add', {
        text: text.trim(),
        range_ms: [inMs, outMs],
        ...modeArgs,
        rationale,
      })
      if (r.ok) {
        setResult(r.result as TitleResult)
        // Flip the Preview to COMPOSED so the title overlay is visible (the raw
        // proxy never shows overlays) — makes the receipt below true.
        document.dispatchEvent(new CustomEvent('cut:show-composed'))
      } else {
        setErr(`${r.error?.code ?? 'failed'}: ${r.error?.message ?? 'title.add failed'}`)
      }
    } catch {
      setErr('server unreachable')
    } finally {
      setBusy(false)
    }
  }

  return (
    <div
      className="cd-scrim"
      data-cut-title-scrim
      // #193b: in "Place anywhere" mode the user drags the title ghost on the Preview BEHIND
      // this drawer — a normal full-screen modal scrim would intercept that drag (and close the
      // drawer). So in free mode the scrim goes transparent + pointer-events:none (drags pass
      // through to the ghost) and doesn't close on click; the drawer panel re-enables its own
      // pointer events below. Other modes keep the standard dim-and-click-to-close modal.
      onMouseDown={blocking ? overlay.onScrimMouseDown : undefined}
      style={mode === 'free' ? { pointerEvents: 'none', background: 'transparent' } : undefined}
    >
      <aside
        ref={overlay.dialogRef}
        className="cd-drawer"
        data-cut-title
        data-cut-title-open="true"
        role="dialog"
        aria-modal={blocking ? 'true' : 'false'}
        aria-label="Add title"
        data-cut-blocking-overlay={blocking || undefined}
        tabIndex={-1}
        onKeyDown={blocking ? overlay.onDialogKeyDown : undefined}
        style={mode === 'free' ? { pointerEvents: 'auto' } : undefined}
      >
        <header className="cd-head">
          <div>
            <h2 className="cd-title">Title</h2>
            <p className="cd-sub">Add an animated lower-third or intro card over a timed span.</p>
          </div>
          <button className="cd-btn cd-btn--ghost" data-cut-title-close onClick={onClose}>
            Close
          </button>
        </header>

        <div className="cd-body">
          {!project ? (
            <div className="cd-empty" data-cut-title-empty>
              Create a project in Projects first.
            </div>
          ) : (
            <>
              {/* text */}
              <label className="cd-field">
                <span className="cd-field-label">Text</span>
                <input
                  className="cd-input"
                  data-cut-title-text
                  autoFocus
                  placeholder="e.g. Jane Doe · Founder"
                  value={text}
                  onChange={(e) => setText(e.target.value)}
                  onKeyDown={(e) => { if (e.key === 'Enter' && canFire) void fire() }}
                />
              </label>

              {/* placement mode — preset position, animated reveal, or free drag */}
              <div className="cd-field">
                <span className="cd-field-label" data-cut-title-placement-label>Placement</span>
                <div className="cd-seg" role="tablist" data-cut-title-placement-mode>
                  <button
                    role="tab"
                    aria-selected={mode === 'preset'}
                    className={`cd-seg-btn ${mode === 'preset' ? 'cd-seg-btn--on' : ''}`}
                    data-cut-title-mode="preset"
                    onClick={() => setMode('preset')}
                  >
                    Preset spot
                  </button>
                  <button
                    role="tab"
                    aria-selected={mode === 'animated'}
                    className={`cd-seg-btn ${mode === 'animated' ? 'cd-seg-btn--on' : ''}`}
                    data-cut-title-mode="animated"
                    onClick={() => setMode('animated')}
                  >
                    Animated reveal
                  </button>
                  <button
                    role="tab"
                    aria-selected={mode === 'free'}
                    className={`cd-seg-btn ${mode === 'free' ? 'cd-seg-btn--on' : ''}`}
                    data-cut-title-mode="free"
                    onClick={() => setMode('free')}
                  >
                    Place anywhere
                  </button>
                </div>
              </div>
              {mode !== 'free' && (
                /* Discoverability: make title placement understandable without
                   where I want"). Positioning lived only under the un-obvious "Free" tab → point at it. */
                <p className="cd-note" data-cut-title-place-hint>
                  Want it in a specific spot? Switch to the <strong>Place anywhere</strong> tab above and drag the title to any corner, edge, or point on the frame.
                </p>
              )}

              {mode === 'preset' ? (
                /* preset picker (the grouped chooser — the polished standard) */
                <label className="cd-field">
                  <span className="cd-field-label">Preset placement</span>
                  <select
                    className="cd-sel"
                    data-cut-title-preset
                    value={preset}
                    onChange={(e) => setPreset(presetFromInput(e.target.value, preset))}
                  >
                    {PRESETS.map((p) => (
                      <option key={p.id} value={p.id}>{p.label}</option>
                    ))}
                  </select>
                </label>
              ) : mode === 'animated' ? (
                /* Animated-text template: a keyframed look + its params. */
                <div className="cd-field">
                  <span className="cd-field-label" data-cut-title-style-label>Style</span>
                  <select
                    className="cd-sel"
                    data-cut-title-template
                    value={template}
                    onChange={(e) => setTemplate(templateFromInput(e.target.value, template))}
                  >
                    {TEMPLATES.map((t) => (
                      <option key={t.id} value={t.id}>{t.label}</option>
                    ))}
                  </select>
                  {/* accent color — only for looks that highlight a word */}
                  {tplInfo?.accent && (
                    <label className="cd-field cd-field--inline" style={{ marginTop: 8 }}>
                      <span className="cd-field-label">Accent</span>
                      <input
                        className="cd-input cd-input--mono"
                        data-cut-title-accent
                        type="text"
                        spellCheck={false}
                        placeholder="#FFD24A"
                        value={accent}
                        onChange={(e) => setAccent(e.target.value)}
                        style={{ maxWidth: 120 }}
                      />
                    </label>
                  )}
                  {/* emphasis word — only for kinetic_emphasis */}
                  {tplInfo?.emphasis && (
                    <label className="cd-field" style={{ marginTop: 8 }}>
                      <span className="cd-field-label">Emphasize word (optional — else the longest)</span>
                      <input
                        className="cd-input"
                        data-cut-title-emphasis
                        type="text"
                        placeholder="e.g. FREE"
                        value={emphasis}
                        onChange={(e) => setEmphasis(e.target.value)}
                      />
                    </label>
                  )}
                  <p className="cd-note">
                    For two-line lower-third reveals, separate the lines with <code>|</code> (name<code>|</code>role).
                  </p>
                </div>
              ) : (
                /* free placement — drag the dot on the frame, or snap to a grid
                   anchor; align sets the text's horizontal anchor at the point. */
                <div className="cd-field">
                  <span className="cd-field-label">Position — drag the dot, or snap to a corner</span>
                  <div
                    className="cd-pospad"
                    data-cut-action="title-pad"
                    data-cut-title-pad
                    ref={padRef}
                    onPointerDown={onPadDown}
                    onPointerMove={onPadMove}
                  >
                    {/* 3×3 quick-anchor snap targets */}
                    {ANCHORS.map((a) => (
                      <button
                        key={a.id}
                        className="cd-pospad-anchor"
                        data-cut-title-anchor={a.id}
                        style={{ left: `${a.x * 100}%`, top: `${a.y * 100}%` }}
                        title={`Snap to ${a.id}`}
                        onPointerDown={(e) => e.stopPropagation()}
                        onClick={() => { setPosX(a.x); setPosY(a.y); setAlign(a.align) }}
                      />
                    ))}
                    {/* live anchor dot + the title text preview */}
                    <div
                      className="cd-pospad-dot"
                      data-cut-title-dot
                      style={{ left: `${posX * 100}%`, top: `${posY * 100}%` }}
                    >
                      <span
                        className="cd-pospad-text"
                        style={{ textAlign: align, transform: `translate(${align === 'left' ? '0' : align === 'right' ? '-100%' : '-50%'}, -50%)` }}
                      >
                        {text.trim() || 'Title'}
                      </span>
                    </div>
                  </div>
                  <div className="cd-row cd-pospad-readout">
                    <span className="cd-note cd-note--mono" data-cut-title-pos>x {posX.toFixed(2)} · y {posY.toFixed(2)}</span>
                    <label className="cd-field cd-field--inline">
                      <span className="cd-field-label">Align</span>
                      <select
                        className="cd-sel cd-sel--sm"
                        data-cut-title-align
                        value={align}
                        onChange={(e) => setAlign(alignFromInput(e.target.value, align))}
                      >
                        <option value="left">left</option>
                        <option value="center">center</option>
                        <option value="right">right</option>
                      </select>
                    </label>
                  </div>
                </div>
              )}

              {/* in/out time (seconds) */}
              <div className="cd-row">
                <label className="cd-field">
                  <span className="cd-field-label">In (s)</span>
                  <input
                    className="cd-input cd-input--mono"
                    data-cut-title-in
                    type="number"
                    min={0}
                    step={0.1}
                    value={inS}
                    onChange={(e) => setInS(Math.max(0, Number(e.target.value) || 0))}
                  />
                </label>
                <label className="cd-field">
                  <span className="cd-field-label">Out (s)</span>
                  <input
                    className="cd-input cd-input--mono"
                    data-cut-title-out
                    type="number"
                    min={0}
                    step={0.1}
                    value={outS}
                    onChange={(e) => setOutS(Math.max(0, Number(e.target.value) || 0))}
                  />
                </label>
              </div>
              {!rangeValid && (
                <p className="cd-note" data-cut-title-rangewarn>Out must be after In.</p>
              )}

              <p className="cd-note">The title appears on its own top layer in the timeline and preview.</p>

              <button
                className="cd-btn cd-btn--primary"
                data-cut-title-apply
                disabled={busy || !canFire}
                onClick={() => void fire()}
              >
                {busy ? 'Adding…' : 'Add title'}
              </button>

              {err && (
                <div className="cd-err" data-cut-title-error role="alert">{err}</div>
              )}

              {result && (
                <div className="cd-result" data-cut-title-result>
                  <div className="cd-result-head">title placed · {result.title_track}</div>
                  <dl className="cd-result-grid">
                    <dt>{result.template ? 'template' : 'preset'}</dt>
                    <dd data-cut-title-result-preset>{result.template || result.preset}</dd>
                    <dt>span</dt>
                    <dd>{(result.range_ms[0] / 1000).toFixed(1)}–{(result.range_ms[1] / 1000).toFixed(1)}s</dd>
                    <dt>clip</dt>
                    <dd>{result.clip_id}</dd>
                  </dl>
                  <div className="cd-result-foot">
                    See the <strong>{result.title_track}</strong> track on the timeline; scrub to the span to view it.
                  </div>
                </div>
              )}
            </>
          )}
        </div>
      </aside>
    </div>
  )
}
