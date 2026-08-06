// TrimPopover.tsx — the pro-trim popover: SLIP / SLIDE / ROLL steppers.
// Role: the discoverable human surface for the trim trio (edit.slip /
// edit.slide_edit / edit.roll). Frame-accurate stepper rows — each click
// dispatches ONE op (individually undoable, review-rail visible); the verb's
// honest refusal (no neighbor / no headroom / crossfaded cut) surfaces as an
// inline note instead of a silent no-op. Roll targets the cut at the CLIP'S
// RIGHT EDGE (clip end → next clip).
// Callers: panels/Timeline/index.tsx (clip context menu → "Trim…").
// Deps: lib/client callVerb + timeline.css (tl-ctx shell + tl-pa patterns).

import { useState } from 'react'
import { callVerb } from '../../lib/client'
import { useBlockingOverlay } from '../../components/overlay/useBlockingOverlay'

interface TrimPopoverProps {
  x: number
  y: number
  clipId: string
  trackId: string
  /** EDITORIAL end of the clip = the roll target cut (clip → next).
   * edit.roll keys at_ms on the engine's cumulative clip-duration-sum clock,
   * which diverges from the drawn (laid) end after an upstream crossfade —
   * the caller (openTrimPopover) supplies editorialStartMs + durMs. */
  clipEndMs: number
  fps: number
  onClose: () => void
}

function clampPopover(el: HTMLDivElement, x: number, y: number): void {
  const margin = 8
  const rect = el.getBoundingClientRect()
  el.style.left = `${Math.max(margin, Math.min(x, window.innerWidth - rect.width - margin))}px`
  el.style.top = `${Math.max(margin, Math.min(y, window.innerHeight - rect.height - margin))}px`
}

export default function TrimPopover({ x, y, clipId, trackId, clipEndMs, fps, onClose }: TrimPopoverProps) {
  const overlay = useBlockingOverlay<HTMLDivElement>(onClose)
  const [note, setNote] = useState<string | null>(null)
  // The roll cut moves as we roll it — track the live seam position locally so
  // repeated clicks keep addressing the same (moved) cut.
  const [seamMs, setSeamMs] = useState(clipEndMs)
  const frame = Math.max(1, Math.round(1000 / (fps || 30)))

  const run = async (kind: 'slip' | 'slide' | 'roll', frames: number) => {
    const by = frames * frame
    setNote(null)
    if (kind === 'slip') {
      const r = await callVerb('edit.slip', { clip: clipId, by_ms: by, rationale: `user slip ${clipId} ${frames > 0 ? '+' : ''}${frames}f` })
      if (!r.ok) setNote(r.error?.message ?? 'slip refused')
    } else if (kind === 'slide') {
      const r = await callVerb('edit.slide_edit', { clip: clipId, by_ms: by, rationale: `user slide ${clipId} ${frames > 0 ? '+' : ''}${frames}f` })
      if (!r.ok) setNote(r.error?.message ?? 'slide refused')
    } else {
      const r = await callVerb('edit.roll', { track: trackId, at_ms: seamMs, by_ms: by, rationale: `user roll cut @ ${seamMs}ms ${frames > 0 ? '+' : ''}${frames}f` })
      if (!r.ok) setNote(r.error?.message ?? 'roll refused')
      else setSeamMs((s) => s + by)
    }
  }

  const row = (kind: 'slip' | 'slide' | 'roll', label: string, hint: string) => (
    <div className="tl-trimpop__row" data-cut-trim-row={kind} title={hint}>
      <span className="tl-trimpop__label">{label}</span>
      <span className="tl-trimpop__btns">
        {[-10, -1, 1, 10].map((f) => (
          <button
            key={f}
            className="tl-trimpop__step"
            data-cut-trim-step={`${kind}:${f}`}
            title={`${label} ${f > 0 ? '+' : ''}${f} frame${Math.abs(f) === 1 ? '' : 's'}`}
            onClick={() => void run(kind, f)}
          >
            {f > 0 ? `+${f}` : f}
          </button>
        ))}
      </span>
    </div>
  )

  return (
    <>
      <div className="tl-ctx-backdrop" data-cut-trim-backdrop data-cut-overlay-part onMouseDown={overlay.onScrimMouseDown} />
      <div
        className="tl-ctx tl-trimpop"
        role="dialog"
        aria-modal="true"
        aria-label="trim clip"
        data-cut-trim-popover
        data-cut-blocking-overlay
        tabIndex={-1}
        onKeyDown={overlay.onDialogKeyDown}
        style={{ left: x, top: y }}
        ref={(el) => {
          overlay.dialogRef.current = el
          if (el) clampPopover(el, x, y)
        }}
      >
        <div className="tl-pa__title">
          Trim <code>{clipId}</code>
          <span className="tl-pa__sub">frame steps at {fps || 30} fps — every click is one undoable edit</span>
        </div>
        {row('slip', 'Slip', 'Shift WHICH source content plays — position and length stay (edit.slip)')}
        {row('slide', 'Slide', 'Move this clip between its neighbors — they absorb the change (edit.slide_edit)')}
        {row('roll', 'Roll cut →', 'Move the cut between this clip and the NEXT one (edit.roll)')}
        {note && <div className="tl-pa__note" data-cut-trim-note>{note}</div>}
        <div className="tl-pa__actions">
          <button className="tl-ctx__item" data-cut-trim-close onClick={onClose}>Done</button>
        </div>
      </div>
    </>
  )
}
