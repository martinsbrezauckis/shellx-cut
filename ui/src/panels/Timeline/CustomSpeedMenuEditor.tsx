import { useState } from 'react'
import { Icon } from '../../icons'
import { parseSpeedFactor, SPEED_FACTOR_MAX, SPEED_FACTOR_MIN, speedFactorReason } from './speedFactor'

interface CustomSpeedMenuEditorProps {
  current: number
  onApply: (factor: number) => void
  onClose: () => void
}

/** Native context-menu editor for `edit.speed.factor`, sharing the schema
 * bounds used by the toolbar and Inspector rather than prompting for a string. */
export default function CustomSpeedMenuEditor({ current, onApply, onClose }: CustomSpeedMenuEditorProps) {
  const [draft, setDraft] = useState(String(current))
  const error = speedFactorReason(draft)
  const submit = () => {
    const factor = parseSpeedFactor(draft)
    if (factor === null) return
    onApply(factor)
    onClose()
  }
  return (
    <div className="tl-ctx__speed-editor" data-cut-custom-speed-editor>
      <label htmlFor="cut-context-speed" className="tl-ctx__speed-label">Custom speed</label>
      <div className="tl-ctx__speed-controls">
        <input
          id="cut-context-speed"
          className="tl-ctx__speed-input"
          data-cut-custom-speed-input
          type="number"
          min={SPEED_FACTOR_MIN}
          max={SPEED_FACTOR_MAX}
          step="any"
          value={draft}
          aria-describedby="cut-context-speed-help"
          aria-invalid={error ? 'true' : undefined}
          onChange={(event) => setDraft(event.target.value)}
          onKeyDown={(event) => {
            if (event.key === 'Enter') submit()
            if (event.key === 'Escape') onClose()
            event.stopPropagation()
          }}
        />
        <button
          className="tl-ctx__item tl-ctx__item--sub"
          data-cut-ctx="speed-custom"
          role="menuitem"
          disabled={!!error}
          title={error ?? `Apply ${parseSpeedFactor(draft)}× with pitch preserved`}
          onClick={submit}
        >
          <Icon name="return" size={14} /> Apply
        </button>
      </div>
      <span id="cut-context-speed-help" className="tl-ctx__speed-help" role="status">
        {error ?? `Pitch stays natural · ${SPEED_FACTOR_MIN}×–${SPEED_FACTOR_MAX}×`}
      </span>
    </div>
  )
}
