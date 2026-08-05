import { useState } from 'react'
import { callVerb, type VerbArgs } from '../../lib/client'

const SHAPE_KINDS = ['rect', 'ellipse', 'line', 'arrow'] as const
type ShapeKind = (typeof SHAPE_KINDS)[number]

function shapeKindFromInput(value: string, fallback: ShapeKind): ShapeKind {
  for (const option of SHAPE_KINDS) {
    if (option === value) return option
  }
  return fallback
}

export interface ShapeEditSectionProps {
  clipId: string
  kind: string
  label: string
  color: string
}

/** Selected shape clip editor. Re-renders the placed shape in place via shape.update. */
export default function ShapeEditSection({
  clipId,
  kind,
  label,
  color,
}: ShapeEditSectionProps) {
  const [kindDraft, setKindDraft] = useState<ShapeKind>(shapeKindFromInput(kind, 'rect'))
  const [labelDraft, setLabelDraft] = useState(label)
  const [colorDraft, setColorDraft] = useState(color)
  const [note, setNote] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)
  const dirty =
    kindDraft !== kind ||
    labelDraft.trim() !== label.trim() ||
    (colorDraft.trim() !== '' && colorDraft.trim() !== color.trim())
  const save = async () => {
    if (!dirty || busy) return
    setBusy(true)
    setNote(null)
    const payload: VerbArgs['shape.update'] = {
      clip: clipId,
      rationale: 'inspector: edit shape',
    }
    if (kindDraft !== kind) payload.shape = kindDraft
    if (labelDraft.trim() !== label.trim()) payload.label = labelDraft.trim()
    if (colorDraft.trim() !== '' && colorDraft.trim() !== color.trim()) {
      payload.fill = colorDraft.trim()
    }
    const r = await callVerb('shape.update', payload)
    setBusy(false)
    if (r.ok) {
      setNote('Shape updated')
      document.dispatchEvent(new CustomEvent('cut:show-composed'))
    } else {
      setNote(r.error?.message ?? r.error?.code ?? 'could not update shape')
    }
    setTimeout(() => setNote(null), 4000)
  }
  return (
    <div className="insp__group" data-cut-inspector-group="shape-edit">
      <div className="insp__group-title">Shape</div>
      <div className="insp__field">
        <label className="insp__label" htmlFor="shape-kind">Type</label>
        <select
          id="shape-kind"
          className="insp__select"
          data-cut-shape-edit-kind
          value={kindDraft}
          onChange={(e) => setKindDraft(shapeKindFromInput(e.target.value, kindDraft))}
        >
          {SHAPE_KINDS.map((k) => (
            <option key={k} value={k}>{k}</option>
          ))}
        </select>
      </div>
      <div className="insp__field">
        <label className="insp__label" htmlFor="shape-label">Label</label>
        <input
          id="shape-label"
          className="insp__text"
          data-cut-shape-edit-label
          type="text"
          value={labelDraft}
          placeholder="Label text… (optional)"
          onChange={(e) => setLabelDraft(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === 'Enter' && (e.metaKey || e.ctrlKey)) void save()
          }}
        />
      </div>
      <div className="insp__field">
        <label className="insp__label" htmlFor="shape-color">Color</label>
        <input
          id="shape-color"
          className="insp__text"
          data-cut-shape-edit-color
          type="text"
          value={colorDraft}
          placeholder="#RRGGBB"
          onChange={(e) => setColorDraft(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === 'Enter' && (e.metaKey || e.ctrlKey)) void save()
          }}
        />
      </div>
      <div className="insp__row">
        <button
          type="button"
          className="insp__btn"
          data-cut-action="shape-save"
          disabled={!dirty || busy}
          title="Re-render this shape with the edited properties. Cmd/Ctrl+Enter."
          onClick={() => void save()}
        >{busy ? 'Saving…' : 'Save shape'}</button>
      </div>
      <p className="insp__hint">Re-renders this shape overlay in place at the same duration. Reposition + restyle finer details (stroke, opacity, geometry) from the Shape drawer.</p>
      {note && <p className="insp__hint" data-cut-shape-edit-note>{note}</p>}
    </div>
  )
}
