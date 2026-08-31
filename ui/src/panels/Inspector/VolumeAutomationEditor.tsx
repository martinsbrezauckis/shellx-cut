import { useEffect, useState } from 'react'
import type { Keyframe, KfInterp } from '../../lib/client'
import { useVolumeAutomation } from '../../app/VolumeAutomationContext'
import {
  clampAutomationPercent,
  clampAutomationTime,
  formatAutomationPercent,
  formatAutomationTime,
  removeVolumeAutomationPoint,
  replaceVolumeAutomationPoint,
  volumeAutomationInterpolation,
  VOLUME_AUTOMATION_MAX_PERCENT,
  VOLUME_AUTOMATION_INTERPOLATIONS,
} from './volumeAutomationModel'
import { seedVolumeAutomationPoints } from '../Timeline/volumeAutomationLaneModel'
export interface VolumeAutomationEditorProps {
  clipId: string
  /** Realized clip-local duration after any retime, as enforced by edit.keyframe. */
  durationMs: number
  keyframes: Keyframe[] | null | undefined
  /** Ephemeral project.state revision, carried separately from durable Project. */
  projectRevision?: string | null
  /** `edit.keyframe` fails closed while the clip has a non-linear speed ramp. */
  hasSpeedRamp?: boolean
  /** Static gain seeds the first automation track without changing playback. */
  staticGainDb?: number
}
/** Inspector-level manual volume automation: a real clip-local control-point editor, not a faux timeline lane. */
export default function VolumeAutomationEditor({
  clipId,
  durationMs,
  keyframes,
  projectRevision,
  hasSpeedRamp = false,
  staticGainDb = 0,
}: VolumeAutomationEditorProps) {
  const automation = useVolumeAutomation({ clipId, durationMs, keyframes, projectRevision, hasSpeedRamp, staticGainDb })
  const track = automation.track
  const [timeMs, setTimeMs] = useState(0)
  const [percent, setPercent] = useState(100)
  const [interp, setInterp] = useState<KfInterp>(track.interp)
  const [selectedPointMs, setSelectedPointMs] = useState<number | null>(null)
  const unavailableReason = automation.unavailableReason
  const automationAvailable = unavailableReason === null
  const busy = automation.mutationState.inFlight

  // A new clip needs a fresh drafting point; project refresh supplies the list.
  useEffect(() => {
    setTimeMs(0)
    setPercent(100)
    setInterp(track.interp)
    setSelectedPointMs(null)
  }, [clipId]) // eslint-disable-line react-hooks/exhaustive-deps

  useEffect(() => setInterp(track.interp), [track.interp])

  const commit = (points: { t_ms: number; value: number }[], nextInterp: KfInterp, rationale: string) =>
    automation.commit(points, nextInterp, rationale)

  const addOrUpdate = () => {
    if (busy || !automationAvailable) return
    const nextTime = clampAutomationTime(timeMs, durationMs)
    const nextPercent = clampAutomationPercent(percent)
    setTimeMs(nextTime)
    setPercent(nextPercent)
    setSelectedPointMs(nextTime)
    const seeded = seedVolumeAutomationPoints(track.points, durationMs, staticGainDb)
    const points = replaceVolumeAutomationPoint(seeded, nextTime, nextPercent)
    void commit(points, interp, `inspector: volume automation at ${nextTime}ms = ${nextPercent}%`)
  }

  const changeInterpolation = (value: string) => {
    if (busy || !automationAvailable) return
    const next = volumeAutomationInterpolation(value, interp)
    setInterp(next)
    if (track.points.length) void commit(track.points, next, `inspector: volume automation interpolation ${next}`)
  }

  const removePoint = (tMs: number) => {
    if (busy || !automationAvailable) return
    const points = removeVolumeAutomationPoint(track.points, tMs)
    if (selectedPointMs === tMs) setSelectedPointMs(null)
    void commit(points, interp, `inspector: remove volume automation point at ${tMs}ms`)
  }

  const selectPoint = (point: { t_ms: number; value: number }) => {
    setTimeMs(point.t_ms)
    setPercent(Math.round(point.value * 100))
    setSelectedPointMs(point.t_ms)
  }

  useEffect(() => {
    const editFromTimeline = (event: Event) => {
      const detail = (event as CustomEvent<{ clipId?: string; tMs?: number }>).detail
      if (detail?.clipId !== clipId || typeof detail.tMs !== 'number') return
      const point = track.points.find((candidate) => candidate.t_ms === detail.tMs)
      if (!point) return
      selectPoint(point)
      requestAnimationFrame(() => {
        document.querySelector<HTMLElement>(`[data-cut-volume-automation-point="${point.t_ms}"] [data-cut-action="volume-automation-point"]`)?.focus()
      })
    }
    document.addEventListener('cut:edit-volume-automation-point', editFromTimeline)
    return () => document.removeEventListener('cut:edit-volume-automation-point', editFromTimeline)
  }, [clipId, track.points])

  const basic = VOLUME_AUTOMATION_INTERPOLATIONS.filter((option) => option.group === 'Basic')
  const advanced = VOLUME_AUTOMATION_INTERPOLATIONS.filter((option) => option.group === 'Advanced')
  const maxTime = Math.max(0, Math.round(durationMs))

  return (
    <div
      className="insp__group insp__automation"
      data-cut-volume-automation
      data-cut-volume-automation-points={track.points.length}
      data-cut-volume-automation-interp={track.interp}
      data-cut-volume-automation-selected={selectedPointMs ?? undefined}
    >
      <div className="insp__group-title insp__group-title--sub">Volume automation</div>
      <p className="insp__hint">
        Add points to shape this clip’s level over time.
      </p>
      <p className="insp__hint">
        Automation overrides static Gain while points exist. Clear it to return to the Gain control above.
      </p>
      {track.points.length === 1 && <p className="insp__hint">One point holds its level for the whole clip. Add another point to make a ramp.</p>}
      {unavailableReason && <p className="insp__hint insp__hint--error" data-cut-volume-automation-unavailable>{unavailableReason}</p>}

      <div className="insp__row">
        <label className="insp__field">
          <span className="insp__label">Time in clip (seconds)</span>
          <input
            className="insp__text"
            data-cut-action="volume-automation-time"
            type="number"
            min={0}
            max={maxTime / 1000}
            step={0.01}
            value={timeMs / 1000}
            disabled={busy || !automationAvailable}
            onChange={(event) => setTimeMs(Math.round(Number(event.target.value) * 1000))}
          />
        </label>
        <label className="insp__field">
          <span className="insp__label">Level (%)</span>
          <input
            className="insp__text"
            data-cut-action="volume-automation-level"
            type="number"
            min={0}
            max={VOLUME_AUTOMATION_MAX_PERCENT}
            step={1}
            value={percent}
            disabled={busy || !automationAvailable}
            onChange={(event) => setPercent(Number(event.target.value))}
          />
        </label>
      </div>

      <div className="insp__row">
        <label className="insp__field insp__field--select">
          <span className="insp__label">Interpolation</span>
          <select
            className="insp__select"
            data-cut-action="volume-automation-interpolation"
            aria-label="Volume automation interpolation"
            value={interp}
            disabled={busy || !automationAvailable || track.points.length === 0}
            onChange={(event) => changeInterpolation(event.target.value)}
          >
            <optgroup label="Basic">
              {basic.map((option) => <option key={option.value} value={option.value}>{option.label}</option>)}
            </optgroup>
            <optgroup label="Advanced">
              {advanced.map((option) => <option key={option.value} value={option.value}>{option.label}</option>)}
            </optgroup>
          </select>
        </label>
      </div>
      <div className="insp__row insp__automation-actions">
        <button
          type="button"
          className="insp__btn insp__btn--accent"
          data-cut-action="volume-automation-add"
          disabled={busy || !automationAvailable}
          onClick={addOrUpdate}
        >
          Add / update point
        </button>
        <button
          type="button"
          className="insp__btn"
          data-cut-action="volume-automation-clear"
          disabled={busy || !automationAvailable || track.points.length === 0}
          onClick={() => void commit([], interp, 'inspector: clear volume automation')}
        >
          Clear automation
        </button>
      </div>

      {track.points.length > 0 ? (
        <ul className="insp__list" aria-label="Volume automation points">
          {track.points.map((point) => (
            <li className="insp__list-row" data-cut-volume-automation-point={point.t_ms} key={point.t_ms}>
              <button
                type="button"
                className={`insp__list-label insp__list-select${selectedPointMs === point.t_ms ? ' insp__list-select--active' : ''}`}
                data-cut-action="volume-automation-point"
                aria-label={`Edit volume point at ${formatAutomationTime(point.t_ms)}`}
                aria-pressed={selectedPointMs === point.t_ms}
                disabled={busy || !automationAvailable}
                onClick={() => selectPoint(point)}
              >
                {formatAutomationTime(point.t_ms)} · {formatAutomationPercent(point.value)}
              </button>
              <button
                type="button"
                className="insp__list-remove"
                data-cut-action="volume-automation-remove"
                aria-label={`Remove volume point at ${formatAutomationTime(point.t_ms)}`}
                disabled={busy || !automationAvailable}
                onClick={() => removePoint(point.t_ms)}
              >
                Remove
              </button>
            </li>
          ))}
        </ul>
      ) : (
        <p className="insp__hint" data-cut-volume-automation-empty>No automation points yet.</p>
      )}
      <p className={`insp__hint${automation.message.startsWith('Could not') || automation.message.startsWith('Project changed') ? ' insp__hint--error' : ''}`} data-cut-volume-automation-status role="status" aria-live="polite">{automation.message}</p>
    </div>
  )
}
