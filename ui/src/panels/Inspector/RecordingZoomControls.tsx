import { useEffect, useState } from 'react'
import type { RecordingZoomSegment } from '../../lib/client'
import { speedPatch } from './recordingZoomModel'

export type ZoomPatch = Partial<RecordingZoomSegment> & { target?: 'pointer' }

interface CommitNumberProps {
  field: string
  label: string
  value: number
  min: number
  max: number
  step?: number
  range?: boolean
  commit: (value: number) => void
}

function CommitNumber({ field, label, value, min, max, step = 0.1, range = false, commit }: CommitNumberProps) {
  const [draft, setDraft] = useState(value)
  useEffect(() => setDraft(value), [value])

  const save = () => {
    if (Number.isFinite(draft) && draft >= min && draft <= max && draft !== value) commit(draft)
  }

  return (
    <label className="insp__zoom-field">
      <span className="insp__label">{label}</span>
      <input
        data-cut-recording-zoom-field={field}
        className={range ? 'insp__zoom-range' : 'insp__select'}
        aria-label={label}
        type={range ? 'range' : 'number'}
        min={min}
        max={max}
        step={step}
        value={draft}
        onChange={(event) => setDraft(event.currentTarget.valueAsNumber)}
        onPointerUp={range ? save : undefined}
        onBlur={range ? undefined : save}
        onKeyUp={(event) => {
          if (range && event.key.startsWith('Arrow')) save()
        }}
        onKeyDown={(event) => {
          if (!range && event.key === 'Enter') event.currentTarget.blur()
        }}
      />
      {range && <output>{draft}</output>}
    </label>
  )
}

function ZoomSpeed({ segment, update }: {
  segment: RecordingZoomSegment
  update: (patch: ZoomPatch) => void
}) {
  const choices = ['Slow', 'Normal', 'Fast']
  const selected = choices.find((choice) => {
    const ramps = speedPatch(choice)
    return segment.ramp_in_ms === ramps.ramp_in_ms && segment.ramp_out_ms === ramps.ramp_out_ms
  })

  return (
    <div className="insp__group">
      <span className="insp__label">Zoom speed</span>
      <div className="insp__zoom-segments" role="group" aria-label="Zoom speed">
        {choices.map((choice) => (
          <button
            key={choice}
            data-cut-recording-zoom-speed={choice.toLowerCase()}
            type="button"
            className={`insp__btn${selected === choice ? ' insp__btn--on' : ''}`}
            aria-pressed={selected === choice}
            onClick={() => update(speedPatch(choice))}
          >
            {choice}
          </button>
        ))}
      </div>
      {!selected && <p className="insp__hint">Custom ramp lengths</p>}
    </div>
  )
}

export default function RecordingZoomControls({ segment, durationMs, update, remove }: {
  segment: RecordingZoomSegment
  durationMs: number
  update: (patch: ZoomPatch) => void
  remove: () => void
}) {
  return (
    <div className="insp__group">
      <CommitNumber field="strength" label="Strength" range value={segment.scale} min={1.2} max={4}
        commit={(scale) => update({ scale })} />
      <button data-cut-action="recording-zoom-pointer" className="insp__btn" onClick={() => update({ target: 'pointer' })}>
        Where the pointer was
      </button>
      <p className="insp__hint">Choose a point to pin the target.</p>
      <div className="insp__zoom-grid" role="group" aria-label="Target grid">
        {['↖', '↑', '↗', '←', '•', '→', '↙', '↓', '↘'].map((symbol, index) => (
          <button
            className="insp__btn"
            key={symbol}
            data-cut-recording-zoom-target={index}
            aria-label={`Target row ${Math.floor(index / 3) + 1}, column ${index % 3 + 1}`}
            onClick={() => update({ cx: (index % 3 + 0.5) / 3, cy: (Math.floor(index / 3) + 0.5) / 3 })}
          >
            {symbol}
          </button>
        ))}
      </div>
      <CommitNumber field="target-x" label="Target X" range value={segment.cx} min={0} max={1} step={0.01}
        commit={(cx) => update({ cx })} />
      <CommitNumber field="target-y" label="Target Y" range value={segment.cy} min={0} max={1} step={0.01}
        commit={(cy) => update({ cy })} />
      <CommitNumber
        field="start"
        label="Start (recording seconds)"
        value={segment.start_ms / 1000}
        min={0}
        max={segment.end_ms / 1000 - 0.001}
        step={0.001}
        commit={(value) => update({ start_ms: Math.round(value * 1000) })}
      />
      <CommitNumber
        field="length"
        label="Length (seconds)"
        value={(segment.end_ms - segment.start_ms) / 1000}
        min={0.001}
        max={(durationMs - segment.start_ms) / 1000}
        step={0.001}
        commit={(value) => update({ end_ms: segment.start_ms + Math.round(value * 1000) })}
      />
      <ZoomSpeed segment={segment} update={update} />
      <button data-cut-action="recording-zoom-delete" className="insp__btn" onClick={remove}>Delete zoom</button>
    </div>
  )
}
