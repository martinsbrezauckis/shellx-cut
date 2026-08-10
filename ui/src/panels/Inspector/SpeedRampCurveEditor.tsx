import { useState } from 'react'
import {
  createSpeedRampDraft,
  insertSpeedRampPoint,
  SPEED_RAMP_MAX_POINTS,
  SPEED_RAMP_MIN_POINTS,
  type SpeedRampDraft,
  type StoredSpeedRamp,
  validateSpeedRampDraft,
} from './speedRampEditorModel'
import './speedRamp.css'

interface SpeedRampCurveEditorProps {
  srcDurMs: number
  stored: StoredSpeedRamp | null | undefined
  onApply: (points: Array<{ at_ms: number; factor: number }>, segments: number) => void
}

function replacePoint(
  draft: SpeedRampDraft,
  index: number,
  field: 'atSeconds' | 'factor',
  value: string,
): SpeedRampDraft {
  return {
    ...draft,
    points: draft.points.map((point, pointIndex) => (
      pointIndex === index ? { ...point, [field]: value } : point
    )),
  }
}

export default function SpeedRampCurveEditor({
  srcDurMs,
  stored,
  onApply,
}: SpeedRampCurveEditorProps) {
  const [draft, setDraft] = useState(() => createSpeedRampDraft(stored, srcDurMs))
  const validation = validateSpeedRampDraft(draft, srcDurMs)
  const canAdd = !!validation.points && draft.points.length < SPEED_RAMP_MAX_POINTS
  const durationSeconds = Math.max(0, srcDurMs / 1000)

  return (
    <div className="speed-ramp-editor" data-cut-speed-ramp-editor>
      <div className="speed-ramp-editor__head" aria-hidden="true">
        <span>Source time</span>
        <span>Speed</span>
        <span />
      </div>
      <div className="speed-ramp-editor__points">
        {draft.points.map((point, index) => (
          <div className="speed-ramp-editor__point" data-cut-speed-ramp-point={index} key={index}>
            <label className="speed-ramp-editor__field">
              <input
                className="insp__num speed-ramp-editor__input"
                data-cut-action="speed-ramp-point"
                data-cut-speed-ramp-at={index}
                type="number"
                min={0}
                max={durationSeconds}
                step={0.001}
                value={point.atSeconds}
                aria-label={`Point ${index + 1} source time in seconds`}
                aria-invalid={validation.invalidPoint === index ? 'true' : undefined}
                aria-describedby="cut-speed-ramp-validation"
                onChange={(event) => setDraft(replacePoint(draft, index, 'atSeconds', event.target.value))}
              />
              <span className="speed-ramp-editor__unit">s</span>
            </label>
            <label className="speed-ramp-editor__field">
              <input
                className="insp__num speed-ramp-editor__input"
                data-cut-action="speed-ramp-point"
                data-cut-speed-ramp-factor={index}
                type="number"
                min={0.25}
                max={4}
                step={0.05}
                value={point.factor}
                aria-label={`Point ${index + 1} speed factor`}
                aria-invalid={validation.invalidPoint === index ? 'true' : undefined}
                aria-describedby="cut-speed-ramp-validation"
                onChange={(event) => setDraft(replacePoint(draft, index, 'factor', event.target.value))}
              />
              <span className="speed-ramp-editor__unit">×</span>
            </label>
            <button
              type="button"
              className="speed-ramp-editor__remove"
              data-cut-action="speed-ramp-point"
              data-cut-speed-ramp-remove={index}
              disabled={draft.points.length <= SPEED_RAMP_MIN_POINTS}
              aria-label={`Remove speed-ramp point ${index + 1}`}
              onClick={() => setDraft({
                ...draft,
                points: draft.points.filter((_, pointIndex) => pointIndex !== index),
              })}
            >−</button>
          </div>
        ))}
      </div>
      <div className="speed-ramp-editor__footer">
        <button
          type="button"
          className="insp__btn insp__btn--mini"
          data-cut-action="speed-ramp-point"
          data-cut-speed-ramp-add
          disabled={!canAdd}
          title={draft.points.length >= SPEED_RAMP_MAX_POINTS
            ? `This editor supports up to ${SPEED_RAMP_MAX_POINTS} control points.`
            : validation.reason ?? 'Add a point in the largest source-time gap'}
          onClick={() => setDraft(insertSpeedRampPoint(draft, srcDurMs))}
        >Add point</button>
        <button
          type="button"
          className="insp__btn insp__btn--accent"
          data-cut-action="speed-ramp-custom-apply"
          data-cut-speed-ramp-apply-custom
          disabled={!validation.points}
          title={validation.reason ?? 'Apply this variable-speed curve'}
          onClick={() => {
            if (validation.points) onApply(validation.points, draft.segments)
          }}
        >Apply curve</button>
      </div>
      <p
        id="cut-speed-ramp-validation"
        className={`insp__hint${validation.reason ? ' insp__hint--error' : ''}`}
        data-cut-speed-ramp-validation={validation.reason ? 'invalid' : 'valid'}
        role="status"
      >
        {validation.reason ?? `${draft.points.length} points · current render detail retained`}
      </p>
    </div>
  )
}
