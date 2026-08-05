// HighlightOverlay — the agent-driven element highlight (ui.highlight verb).
// Role: when an agent calls ui.highlight{selector|clip|panel, label, description},
// the server relays a ui_command to this client; App stores the spec and mounts
// this overlay, which RESOLVES the target element, draws an animated outline over
// its bounding box, and shows a label/description chip beside it — so it is
// visible on screen WHICH control the agent is driving and WHY (guided demos +
// debugging; the shellX / ShellX Canvas debug-highlight, brought to Cut).
//
// The overlay FOLLOWS the target (re-measures on scroll/resize + a rAF tick while
// active) so it stays glued even as the timeline scrolls or a drawer animates.
// Auto-clears after duration_ms (0 = stay until cleared/replaced). Pure view, no
// verbs, no state mutation; clicks pass through except for the dismiss button.
//
// Callers: App.tsx (mounts when a highlight spec is active). Deps: highlight.css.

import { useEffect, useRef, useState } from 'react'
import { Icon } from './icons'
import './highlight.css'

export interface HighlightSpec {
  /** Resolve the target by exactly one of these. */
  selector?: string
  clip?: string
  panel?: string
  label?: string
  description?: string
  /** Auto-clear after N ms (0 = stay until cleared/replaced). Default 3500. */
  duration_ms?: number
  /** Scroll the target into view first (default true). */
  scroll?: boolean
  /** Monotonic nonce so re-highlighting the SAME target re-triggers the effect. */
  n: number
}

interface Box { top: number; left: number; width: number; height: number }

/** Build the CSS selector for a spec (selector wins, then clip, then panel). */
function specSelector(s: HighlightSpec): string | null {
  if (s.selector) return s.selector
  if (s.clip) return `[data-cut-clip="${CSS.escape(s.clip)}"]`
  if (s.panel) return `[data-cut-panel="${CSS.escape(s.panel)}"]`
  return null
}

export default function HighlightOverlay({ spec, onClear }: { spec: HighlightSpec | null; onClear: () => void }) {
  const [box, setBox] = useState<Box | null>(null)
  const [missing, setMissing] = useState(false)
  const rafRef = useRef<number>(0)

  useEffect(() => {
    if (!spec) return
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key === 'Escape') onClear()
    }
    window.addEventListener('keydown', onKeyDown)
    return () => window.removeEventListener('keydown', onKeyDown)
  }, [spec, onClear])

  useEffect(() => {
    if (!spec) { setBox(null); setMissing(false); return }
    const sel = specSelector(spec)
    const el = sel ? document.querySelector<HTMLElement>(sel) : null
    if (!el) {
      // Honest: if the target isn't found, show a centered "not found" chip
      // briefly rather than silently doing nothing (the agent gets visible feedback).
      setMissing(true)
      setBox(null)
      const t = setTimeout(onClear, 1800)
      return () => clearTimeout(t)
    }
    setMissing(false)
    if (spec.scroll !== false) el.scrollIntoView({ block: 'nearest', inline: 'nearest', behavior: 'smooth' })

    // Follow the target: re-measure on a rAF tick (covers timeline scroll, drawer
    // animations, layout shifts) — cheap, only while a highlight is active.
    const measure = () => {
      const r = el.getBoundingClientRect()
      setBox({ top: r.top, left: r.left, width: r.width, height: r.height })
      rafRef.current = requestAnimationFrame(measure)
    }
    measure()

    const dur = spec.duration_ms ?? 3500
    const clearTimer = dur > 0 ? setTimeout(onClear, dur) : null
    return () => {
      cancelAnimationFrame(rafRef.current)
      if (clearTimer) clearTimeout(clearTimer)
    }
  }, [spec, onClear])

  if (!spec) return null

  if (missing) {
    return (
      <div className="hl-root" data-cut-highlight-missing>
        <div className="hl-chip hl-chip--center">
          <button
            className="hl-close"
            type="button"
            data-cut-highlight-close
            aria-label="Close highlight"
            onClick={onClear}
          >
            <Icon name="close" size={14} />
          </button>
          <div className="hl-chip-copy">
            <div className="hl-chip-label">{spec.label ?? 'Highlight'}</div>
            <div className="hl-chip-desc">target not found{spec.selector ? `: ${spec.selector}` : ''}</div>
          </div>
        </div>
      </div>
    )
  }
  if (!box) return null

  // Ring geometry. Normally the outline sits GROW px OUTSIDE the control so it
  // frames it. But for a control flush against the window edge (top bar, left
  // rail, the right-edge Inspect strip) an outset ring would draw its line at a
  // negative coord / past innerWidth — that whole side is painted off-window and
  // never shows. This was the ShellX Canvas edge-clip bug. Fix: clamp every side
  // to stay >= MARGIN inside the viewport, so on an edge the line simply hugs in
  // a few px instead of disappearing. The visible rectangle is always complete.
  const GROW = 3
  const MARGIN = 5 // keep the 2px border + pulse spread fully on-window
  const ringTop = Math.max(MARGIN, box.top - GROW)
  const ringLeft = Math.max(MARGIN, box.left - GROW)
  const ringRight = Math.min(window.innerWidth - MARGIN, box.left + box.width + GROW)
  const ringBottom = Math.min(window.innerHeight - MARGIN, box.top + box.height + GROW)
  const ringStyle = {
    top: ringTop,
    left: ringLeft,
    width: Math.max(0, ringRight - ringLeft),
    height: Math.max(0, ringBottom - ringTop),
  }

  // Place the chip below the box if there's room, else above; clamp horizontally.
  const PAD = 8
  const below = box.top + box.height + PAD
  const above = box.top - PAD
  const preferBelow = below + 80 < window.innerHeight
  const chipTop = preferBelow ? below : Math.max(8, above - 72)
  const chipLeft = Math.max(8, Math.min(box.left, window.innerWidth - 320))

  const showChip = Boolean(spec.label || spec.description || spec.duration_ms === 0)

  return (
    <div className="hl-root" data-cut-highlight={spec.selector ?? spec.clip ?? spec.panel ?? ''}>
      {/* the outline (a glowing ring that pulses) — clamped to stay on-window */}
      <div className="hl-ring" style={ringStyle} aria-hidden="true" />
      {/* the label/description chip */}
      {showChip && (
        <div className="hl-chip" style={{ top: chipTop, left: chipLeft }} data-cut-highlight-chip>
          <button
            className="hl-close"
            type="button"
            data-cut-highlight-close
            aria-label="Close highlight"
            onClick={onClear}
          >
            <Icon name="close" size={14} />
          </button>
          <div className="hl-chip-copy">
            <div className="hl-chip-label">{spec.label ?? 'Highlight'}</div>
            {spec.description && <div className="hl-chip-desc">{spec.description}</div>}
          </div>
        </div>
      )}
    </div>
  )
}
