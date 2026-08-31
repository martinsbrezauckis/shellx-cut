import { useState } from 'react'
import { callVerb } from '../../lib/client'
import { TemporalRange } from '../../components/TemporalNavigation'

export interface CaptionEditSectionProps {
  clipId: string
  text: string
  rangeMs: [number, number]
  onSeek: (atMs: number) => void
}

/** Selected caption clip editor. Updates caption text in place via captions.set_text. */
export default function CaptionEditSection({ clipId, text, rangeMs, onSeek }: CaptionEditSectionProps) {
  const [draft, setDraft] = useState(text)
  const [note, setNote] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)
  const trimmed = draft.trim()
  const dirty = trimmed.length > 0 && trimmed !== text.trim()
  const save = async () => {
    if (!dirty || busy) return
    setBusy(true)
    setNote(null)
    const r = await callVerb('captions.set_text', {
      clip: clipId,
      text: trimmed,
      rationale: 'inspector: edit caption text',
    })
    setBusy(false)
    if (r.ok) {
      setNote('Caption updated')
      document.dispatchEvent(new CustomEvent('cut:show-composed'))
    } else {
      setNote(r.error?.message ?? r.error?.code ?? 'could not update caption')
    }
    setTimeout(() => setNote(null), 4000)
  }
  return (
    <div className="insp__group" data-cut-inspector-group="caption-edit">
      <div className="insp__group-title">Caption text</div>
      <div className="insp__field">
        <textarea
          className="insp__text insp__textarea"
          data-cut-caption-edit-text
          rows={2}
          value={draft}
          placeholder="Caption text…"
          onChange={(e) => setDraft(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === 'Enter' && (e.metaKey || e.ctrlKey)) void save()
          }}
        />
      </div>
      <div className="insp__row">
        <button
          type="button"
          className="insp__btn"
          data-cut-action="caption-save-text"
          disabled={!dirty || busy}
          title="Replace this caption's words. Cmd/Ctrl+Enter."
          onClick={() => void save()}
        >{busy ? 'Saving…' : 'Save text'}</button>
        <TemporalRange
          rangeMs={rangeMs}
          separator=" – "
          className="insp__hint"
          data={{ 'data-cut-caption-edit-range': clipId }}
          title="Seek to the start of this caption's exact timeline range"
          onActivate={onSeek}
        />
      </div>
      <p className="insp__hint">Restyle or reposition all captions in the project from the Captions panel (no selection); delete via the clip's right-click menu.</p>
      {note && <p className="insp__hint" data-cut-caption-edit-note>{note}</p>}
    </div>
  )
}
