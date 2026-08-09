import { useState } from 'react'
import { callVerb } from '../../lib/client'

export interface TitleEditSectionProps {
  clipId: string
  text: string
}

/** Selected title clip editor. Re-renders the placed title in place via title.update. */
export default function TitleEditSection({ clipId, text }: TitleEditSectionProps) {
  const [draft, setDraft] = useState(text)
  const [note, setNote] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)
  const trimmed = draft.trim()
  const dirty = trimmed.length > 0 && trimmed !== text.trim()
  const save = async () => {
    if (!dirty || busy) return
    setBusy(true)
    setNote(null)
    const r = await callVerb('title.update', {
      clip: clipId,
      text: trimmed,
      rationale: 'inspector: edit title text',
    })
    setBusy(false)
    if (r.ok) {
      setNote('Title updated')
      document.dispatchEvent(new CustomEvent('cut:show-composed'))
    } else {
      setNote(r.error?.message ?? r.error?.code ?? 'could not update title')
    }
    setTimeout(() => setNote(null), 4000)
  }
  return (
    <div className="insp__group" data-cut-inspector-group="title-edit">
      <div className="insp__group-title">Title text</div>
      <div className="insp__field">
        <textarea
          className="insp__text insp__textarea"
          data-cut-title-edit-text
          rows={2}
          value={draft}
          placeholder="Title text…"
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
          data-cut-action="title-save-text"
          disabled={!dirty || busy}
          title="Re-render this title with new words. Cmd/Ctrl+Enter."
          onClick={() => void save()}
        >{busy ? 'Saving…' : 'Save text'}</button>
      </div>
      <p className="insp__hint">Restyle (preset, color, animation, position) from the Title drawer; this re-renders the title's words in place at the same duration.</p>
      {note && <p className="insp__hint" data-cut-title-edit-note>{note}</p>}
    </div>
  )
}
