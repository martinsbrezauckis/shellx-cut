import { useState } from 'react'
import { callVerb, type VerbResult } from '../../lib/client'
import InspectorSection from '../../components/inspector/InspectorSection'
import PropertyRow from '../../components/inspector/PropertyRow'

/** edit.speed_ramp PRESET curves (variable-speed "speed ramp"). Each builds a
 *  piecewise-linear speed curve over the clip's SOURCE window [0, D] (D = source
 *  duration ms). Factors stay in the verb's 0.25-4.0 range; points are strictly
 *  ascending by at_ms. */
type RampPreset = 'slow_fast_slow' | 'fast_slow_fast' | 'ramp_up' | 'ramp_down'
const RAMP_PRESETS: { key: RampPreset; label: string; points: (d: number) => { at_ms: number; factor: number }[] }[] = [
  { key: 'slow_fast_slow', label: 'Slow → fast → slow', points: (d) => [{ at_ms: 0, factor: 0.5 }, { at_ms: Math.round(d / 2), factor: 2 }, { at_ms: d, factor: 0.5 }] },
  { key: 'fast_slow_fast', label: 'Fast → slow → fast', points: (d) => [{ at_ms: 0, factor: 2 }, { at_ms: Math.round(d / 2), factor: 0.5 }, { at_ms: d, factor: 2 }] },
  { key: 'ramp_up', label: 'Ramp up (slow → fast)', points: (d) => [{ at_ms: 0, factor: 0.5 }, { at_ms: d, factor: 2 }] },
  { key: 'ramp_down', label: 'Ramp down (fast → slow)', points: (d) => [{ at_ms: 0, factor: 2 }, { at_ms: d, factor: 0.5 }] },
]

function rampPresetFromInput(value: string, fallback: RampPreset): RampPreset {
  for (const option of RAMP_PRESETS) {
    if (option.key === value) return option.key
  }
  return fallback
}

/** A 3-point ramp needs the midpoint distinct from both ends; below this the
 *  curve presets are hidden because the clip is too short to ramp meaningfully. */
const RAMP_MIN_DUR_MS = 400

function applyVisual(p: Promise<VerbResult>): void {
  void p.then((r) => {
    if (r.ok) document.dispatchEvent(new CustomEvent('cut:show-composed'))
  })
}

export interface SpeedSectionProps {
  clipId: string
  speed: number
  reverse: boolean
  frozen: boolean
  speedRampApplied: boolean
  srcDurMs: number
}

/** SPEED / RETIME section (video clips). A continuous Speed PropertyRow plus a
 *  Reverse toggle, Freeze toggle, and mutually exclusive speed-ramp controls.
 *  This is the single selected-clip Inspector home for edit.speed,
 *  edit.reverse, edit.freeze, and edit.speed_ramp. */
export default function SpeedSection({
  clipId,
  speed,
  reverse,
  frozen,
  speedRampApplied,
  srcDurMs,
}: SpeedSectionProps) {
  const [rampPreset, setRampPreset] = useState<RampPreset>('slow_fast_slow')
  const rampBlocked = speed !== 1 || reverse || frozen
  const rampTooShort = srcDurMs < RAMP_MIN_DUR_MS
  const applyRamp = () => {
    const d = Math.round(srcDurMs)
    const preset = RAMP_PRESETS.find((p) => p.key === rampPreset)
    if (!preset) return
    applyVisual(callVerb('edit.speed_ramp', { clip: clipId, points: preset.points(d), rationale: `inspector: speed ramp ${rampPreset}` }))
  }
  const clearRamp = () => applyVisual(callVerb('edit.speed_ramp', { clip: clipId, points: [], rationale: 'inspector: clear speed ramp' }))
  const activeTiming = [
    speedRampApplied ? 'Speed ramp' : '',
    speed !== 1 ? `${speed}×` : '',
    reverse ? 'Reverse' : '',
    frozen ? 'Frozen' : '',
  ].filter(Boolean)

  return (
    <InspectorSection
      title="Speed & timing"
      sectionKey="speed"
      defaultCollapsed
      summary={activeTiming.length > 0 ? activeTiming.join(' · ') : '1× · forward'}
      summaryTone={activeTiming.length > 0 ? 'active' : 'neutral'}
      bypassed={speed === 1 && !reverse && !frozen}
      onToggleBypass={() => {
        applyVisual(callVerb('edit.speed', { clip: clipId, factor: 1, rationale: 'inspector: clear speed' }))
        if (reverse) applyVisual(callVerb('edit.reverse', { clip: clipId, enabled: false, rationale: 'inspector: clear reverse' }))
        if (frozen) applyVisual(callVerb('edit.freeze', { clip: clipId, enabled: false, rationale: 'inspector: clear freeze' }))
      }}
      onReset={() => applyVisual(callVerb('edit.speed', { clip: clipId, factor: 1, rationale: 'inspector: reset speed' }))}
    >
      <PropertyRow
        label="Speed" propKey="speed" unit="×"
        value={speed} min={0.25} max={4} step={0.05} default={1}
        onCommit={(v) => applyVisual(callVerb('edit.speed', { clip: clipId, factor: v, rationale: `inspector: speed ${v}×` }))}
      />
      <div className="insp__row">
        <button
          type="button"
          className={`insp__btn${reverse ? ' insp__btn--on' : ''}`}
          data-cut-prop="speed-reverse"
          data-cut-speed-reverse-on={reverse ? 'true' : 'false'}
          title={reverse ? 'Play this clip forward again' : 'Play this clip backward'}
          onClick={() => applyVisual(callVerb('edit.reverse', { clip: clipId, enabled: !reverse, rationale: 'inspector: reverse' }))}
        >
          {reverse ? 'Un-reverse' : 'Reverse'}
        </button>
        <button
          type="button"
          className={`insp__btn${frozen ? ' insp__btn--on' : ''}`}
          data-cut-prop="speed-freeze"
          data-cut-speed-freeze-on={frozen ? 'true' : 'false'}
          title={frozen ? 'Release the freeze-frame and play normally' : 'Hold the first frame for the whole clip slot'}
          onClick={() => applyVisual(callVerb('edit.freeze', { clip: clipId, enabled: !frozen, rationale: 'inspector: freeze frame' }))}
        >
          {frozen ? 'Un-freeze' : 'Freeze'}
        </button>
      </div>
      <div className="insp__group-title insp__group-title--sub">Speed ramp</div>
      {rampBlocked ? (
        <p className="insp__hint" data-cut-speed-ramp-blocked>
          Reset Speed to 1× and clear Reverse / Freeze first — a speed ramp can't combine with them.
        </p>
      ) : rampTooShort ? (
        <p className="insp__hint" data-cut-speed-ramp-blocked>Clip is too short to ramp.</p>
      ) : (
        <div className="insp__row" data-cut-speed-ramp>
          <select
            className="insp__select"
            data-cut-speed-ramp-preset
            value={rampPreset}
            title="Variable-speed curve to apply over this clip"
            onChange={(e) => setRampPreset(rampPresetFromInput(e.target.value, rampPreset))}
          >
            {RAMP_PRESETS.map(({ key, label }) => (<option key={key} value={key}>{label}</option>))}
          </select>
          <button
            type="button"
            className="insp__btn"
            data-cut-action="speed-ramp"
            title="Apply a variable-speed curve so the clip speeds up and slows down over its length"
            onClick={applyRamp}
          >Apply ramp</button>
          <button
            type="button"
            className="insp__btn"
            data-cut-action="speed-ramp-clear"
            title="Remove the speed ramp (back to constant speed)"
            onClick={clearRamp}
          >Clear</button>
        </div>
      )}
    </InspectorSection>
  )
}
