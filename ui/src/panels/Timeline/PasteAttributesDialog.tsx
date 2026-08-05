// PasteAttributesDialog.tsx — the "Paste attributes…" checkbox dialog.
// Role: pick WHICH attribute categories of the copied clip to paste onto the
// selected clips, then dispatch ONE edit.paste_attributes call (a pure
// orchestrator server-side: auto-checkpoint + existing replay-safe sub-verbs).
// The dialog reports the verb's honest outcome inline (applied / skipped /
// failed + revert hint) instead of closing blind.
// Callers: panels/Timeline/index.tsx (context menu + Ctrl+Alt+V). Deps:
// lib/client callVerb + timeline.css (tl-ctx shell patterns).

import { useState } from 'react'
import { callVerb } from '../../lib/client'
import { useBlockingOverlay } from '../../components/overlay/useBlockingOverlay'

type Category = 'grade' | 'transform' | 'speed' | 'volume' | 'effects'

const CATEGORIES: Array<{ key: Category; label: string; hint: string }> = [
  { key: 'grade', label: 'Color grade', hint: 'contrast / brightness / saturation / gamma / temperature / LUT' },
  { key: 'transform', label: 'Transform & crop', hint: 'position, scale, opacity + source crop' },
  { key: 'speed', label: 'Speed', hint: 'playback speed factor' },
  { key: 'volume', label: 'Volume & fades', hint: 'clip gain + fade in/out' },
  { key: 'effects', label: 'Effects & EQ', hint: 'the full effects list + audio EQ' },
]

interface PasteAttributesDialogProps {
  fromClip: string
  toClips: string[]
  onClose: () => void
}

export default function PasteAttributesDialog({ fromClip, toClips, onClose }: PasteAttributesDialogProps) {
  const overlay = useBlockingOverlay<HTMLDivElement>(onClose)
  const [which, setWhich] = useState<Set<Category>>(new Set(['grade', 'transform', 'speed', 'volume', 'effects']))
  const [busy, setBusy] = useState(false)
  const [note, setNote] = useState<string | null>(null)

  const toggle = (c: Category) =>
    setWhich((w) => {
      const n = new Set(w)
      if (n.has(c)) n.delete(c)
      else n.add(c)
      return n
    })

  const apply = async () => {
    if (which.size === 0 || busy) return
    setBusy(true)
    setNote(null)
    const r = await callVerb('edit.paste_attributes', {
      from_clip: fromClip,
      to_clips: toClips,
      which: [...which],
      rationale: `user paste attributes (${[...which].join('+')}) onto ${toClips.length} clip(s)`,
    })
    setBusy(false)
    if (!r.ok) {
      setNote(r.error?.message ?? 'paste failed')
      return
    }
    const res = r.result as { status?: string; failed_step?: string; skipped?: string[]; revert_hint?: string }
    if (res.status === 'failed') {
      setNote(`Failed at ${res.failed_step} — ${res.revert_hint ?? 'revert to the checkpoint to undo the partial paste'}`)
      return
    }
    // success: surface what was skipped (if anything), then close shortly.
    if (res.skipped && res.skipped.length > 0) {
      setNote(`Applied. Skipped: ${res.skipped.join('; ')}`)
      window.setTimeout(onClose, 1600)
    } else {
      onClose()
    }
  }

  return (
    <>
      <div className="tl-ctx-backdrop" data-cut-pa-backdrop data-cut-overlay-part onMouseDown={overlay.onScrimMouseDown} />
      <div
        ref={overlay.dialogRef}
        className="tl-ctx tl-pa"
        role="dialog"
        aria-modal="true"
        aria-label="paste attributes"
        data-cut-paste-attributes
        data-cut-blocking-overlay
        data-cut-pa-source={fromClip}
        data-cut-pa-target-count={toClips.length}
        tabIndex={-1}
        onKeyDown={overlay.onDialogKeyDown}
      >
        <div className="tl-pa__title">
          Paste attributes
          <span className="tl-pa__sub" data-cut-pa-summary>
            Choose what to copy onto {toClips.length} selected clip{toClips.length === 1 ? '' : 's'}.
          </span>
        </div>
        {CATEGORIES.map((c) => (
          <label key={c.key} className="tl-pa__row" title={c.hint}>
            <input
              type="checkbox"
              data-cut-pa-check={c.key}
              checked={which.has(c.key)}
              onChange={() => toggle(c.key)}
            />
            <span className="tl-pa__label">{c.label}</span>
            <span className="tl-pa__hint">{c.hint}</span>
          </label>
        ))}
        {note && <div className="tl-pa__note" data-cut-pa-note>{note}</div>}
        <div className="tl-pa__actions">
          <button className="tl-ctx__item" data-cut-pa-cancel onClick={onClose}>Cancel</button>
          <button
            className="tl-ctx__item tl-pa__apply"
            data-cut-pa-apply
            disabled={which.size === 0 || busy}
            onClick={() => void apply()}
          >
            {busy ? 'Applying…' : 'Apply'}
          </button>
        </div>
      </div>
    </>
  )
}
