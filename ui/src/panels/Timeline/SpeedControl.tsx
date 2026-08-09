import { useState } from 'react'
import { parseSpeedFactor, SPEED_FACTOR_MAX, SPEED_FACTOR_MIN, SPEED_FACTOR_STEP } from './speedFactor'

interface SpeedControlProps {
  disabled: boolean
  current: number | undefined
  onSet: (factor: number) => void
}

/** Toolbar speed control (edit.speed): presets plus a custom factor for the
 * selected media clip(s). 1x clears the retime. */
export default function SpeedControl({ disabled, current, onSet }: SpeedControlProps) {
  const [val, setVal] = useState('')
  const commit = () => {
    const f = parseSpeedFactor(val)
    if (f !== null) {
      onSet(f)
      setVal('')
    }
  }
  return (
    <div className="tl-speed" data-cut-speed-control title="Change the selected clip speed while preserving pitch">
      <span className="tl-speed__label">Speed</span>
      {[0.5, 1, 2].map((p) => (
        <button
          key={p}
          type="button"
          className={`tl-tool tl-speed__preset ${!disabled && current === p ? 'tl-tool--on' : ''}`}
          data-cut-action="speed-preset"
          data-cut-speed-preset={p}
          disabled={disabled}
          title={p === 1 ? 'Normal speed (clear retime)' : p > 1 ? `${p}× — faster` : `${p}× — slow motion`}
          onClick={() => onSet(p)}
        >
          {p}×
        </button>
      ))}
      <input
        className="tl-speed__input"
        data-cut-speed-input
        type="number"
        min={SPEED_FACTOR_MIN}
        max={SPEED_FACTOR_MAX}
        step={SPEED_FACTOR_STEP}
        disabled={disabled}
        placeholder={current === undefined ? '—' : `${current}×`}
        value={val}
        onChange={(e) => setVal(e.target.value)}
        onKeyDown={(e) => { if (e.key === 'Enter') commit() }}
        title="Custom factor 0.25–4 (Enter to apply)"
      />
    </div>
  )
}
