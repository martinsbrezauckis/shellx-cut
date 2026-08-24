import { useEffect, useMemo, useRef, useState } from 'react'
import type { Keyframe, KfInterp } from '../../lib/client'
import { runUserVerb } from '../../lib/userActionFeedback'
import {
  clampAutomationPercent,
  clampAutomationTime,
  createVolumeAutomationMutationController,
  formatAutomationPercent,
  formatAutomationTime,
  nextVolumeAutomationRequestId,
  removeVolumeAutomationPoint,
  replaceVolumeAutomationPoint,
  volumeAutomationInterpolation,
  volumeAutomationKeyframesFingerprint,
  volumeAutomationNeedsAuthoritativeRefresh,
  volumeAutomationTrack,
  volumeAutomationUnavailableReason,
  type VolumeAutomationTransientState,
  VOLUME_AUTOMATION_MAX_PERCENT,
  VOLUME_AUTOMATION_INTERPOLATIONS,
} from './volumeAutomationModel'
export interface VolumeAutomationEditorProps {
  clipId: string
  /** Realized clip-local duration after any retime, as enforced by edit.keyframe. */
  durationMs: number
  keyframes: Keyframe[] | null | undefined
  /** Ephemeral project.state revision, carried separately from durable Project. */
  projectRevision?: string | null
  /** `edit.keyframe` fails closed while the clip has a non-linear speed ramp. */
  hasSpeedRamp?: boolean
  /** Keeps the sibling static-Gain controls safe before project refresh catches up. */
  onAutomationStateChange?: (state: VolumeAutomationTransientState) => void
}
/** Inspector-level manual volume automation: a real clip-local control-point editor, not a faux timeline lane. */
export default function VolumeAutomationEditor({
  clipId,
  durationMs,
  keyframes,
  projectRevision,
  hasSpeedRamp = false,
  onAutomationStateChange,
}: VolumeAutomationEditorProps) {
  const serverTrack = useMemo(() => volumeAutomationTrack(keyframes), [keyframes])
  const keyframesFingerprint = useMemo(() => volumeAutomationKeyframesFingerprint(keyframes), [keyframes])
  const mutationController = useRef(createVolumeAutomationMutationController(projectRevision))
  const [mutationState, setMutationState] = useState(() => mutationController.current.state())
  // Keep one successful edit projected until its equal-or-newer refresh, so a normal
  // follow-up click uses the complete SET-semantics track from the last save.
  const [projectedTrack, setProjectedTrack] = useState<{
    projectRevision: string
    track: ReturnType<typeof volumeAutomationTrack>
  } | null>(null)
  const track = projectedTrack?.projectRevision === mutationState.projectRevision ? projectedTrack.track : serverTrack
  const hasProjectedTrack = projectedTrack?.projectRevision === mutationState.projectRevision
  const [timeMs, setTimeMs] = useState(0)
  const [percent, setPercent] = useState(100)
  const [interp, setInterp] = useState<KfInterp>(track.interp)
  const [selectedPointMs, setSelectedPointMs] = useState<number | null>(null)
  const [note, setNote] = useState('')
  const [error, setError] = useState('')
  const [refreshRequired, setRefreshRequired] = useState(false)
  const refreshRequiredRef = useRef(false)
  const unavailableReason = volumeAutomationUnavailableReason(durationMs, hasSpeedRamp)
    ?? (mutationState.projectRevision ? null : 'Volume automation is waiting for the current project revision.')
  const automationAvailable = unavailableReason === null
  const busy = mutationState.inFlight
  const reportAutomationState = (
    inFlight: boolean,
    effectivePointCount: number,
    projected: boolean,
    needsRefresh = refreshRequiredRef.current,
    currentProjectRevision = mutationController.current.state().projectRevision,
  ) => {
    onAutomationStateChange?.({ clipId, inFlight, effectivePointCount, projected, projectRevision: currentProjectRevision, refreshRequired: needsRefresh })
  }
  useEffect(() => {
    const observed = mutationController.current.observeAuthoritative(projectRevision)
    setMutationState(observed.state)
    if (refreshRequiredRef.current && observed.applied) {
      refreshRequiredRef.current = false
      setRefreshRequired(false)
    }
    if (!observed.applied) return
    setProjectedTrack((current) => {
      if (!current || current.projectRevision !== observed.state.projectRevision) return null
      return JSON.stringify(current.track) === JSON.stringify(serverTrack) ? null : current
    })
  }, [keyframesFingerprint, projectRevision, serverTrack])
  useEffect(() => () => { mutationController.current.dispose() }, [])

  useEffect(() => {
    onAutomationStateChange?.({
      clipId,
      inFlight: busy,
      effectivePointCount: track.points.length,
      projected: hasProjectedTrack,
      projectRevision: mutationState.projectRevision,
      refreshRequired,
    })
  }, [busy, clipId, hasProjectedTrack, onAutomationStateChange, refreshRequired, track.points.length])

  useEffect(() => () => {
    onAutomationStateChange?.({ clipId, inFlight: false, effectivePointCount: 0, projected: false, refreshRequired: false })
  }, [clipId, onAutomationStateChange])

  // A new clip needs a fresh drafting point; project refresh supplies the list.
  useEffect(() => {
    setTimeMs(0)
    setPercent(100)
    setInterp(track.interp)
    setSelectedPointMs(null)
    setNote('')
    setError('')
  }, [clipId]) // eslint-disable-line react-hooks/exhaustive-deps

  useEffect(() => setInterp(track.interp), [track.interp])

  const commit = async (points: { t_ms: number; value: number }[], nextInterp: KfInterp, rationale: string) => {
    // Synchronize before the synchronous lock. An older snapshot keeps
    // the active expected_revision rather than reopening this editor.
    const observed = mutationController.current.observeAuthoritative(projectRevision)
    setMutationState(observed.state)
    const requestId = nextVolumeAutomationRequestId()
    const controls = mutationController.current.begin(requestId)
    if (!controls) {
      const current = mutationController.current.state()
      setMutationState(current)
      if (!current.inFlight && !current.projectRevision) {
        setError('Volume automation needs the current project revision before it can save.')
      }
      return null
    }
    setMutationState(mutationController.current.state())
    // Reach VolumeSection's ref before its Gain input can rerender disabled.
    reportAutomationState(true, points.length, true, false)
    setNote('')
    setError('')
    let response: Awaited<ReturnType<typeof runUserVerb>> = null
    try {
      response = await runUserVerb(
        'edit.keyframe',
        { clip: clipId, param: 'volume', points, interp: nextInterp, rationale, ...controls },
        'Could not update volume automation.',
      )
    } finally {
      const completion = mutationController.current.complete(requestId, {
        ok: response?.ok === true,
        projectRevision: response?.project_revision,
        errorCode: response?.error?.code,
      })
      if (completion.owned) {
        setMutationState(completion.state)
        if (response?.ok && completion.status === 'saved') {
          setProjectedTrack({ projectRevision: completion.state.projectRevision!, track: { points, interp: nextInterp } })
          refreshRequiredRef.current = false
          setRefreshRequired(false)
          reportAutomationState(false, points.length, true, false)
          if (points.length === 0) setSelectedPointMs(null)
          setNote(points.length
            ? `${points.length} volume point${points.length === 1 ? '' : 's'} · ${nextInterp.replaceAll('_', ' ')}`
            : 'Volume automation cleared — static Gain applies again.')
        } else if (volumeAutomationNeedsAuthoritativeRefresh(response !== null, completion)) {
          mutationController.current.requireAuthoritativeAfter(controls.expected_revision)
          setMutationState(mutationController.current.state())
          refreshRequiredRef.current = true
          setRefreshRequired(true)
          setProjectedTrack(null)
          reportAutomationState(false, serverTrack.points.length, false, true)
          setError('Could not confirm volume automation. Refresh project state before changing static Gain or automation.')
        } else if (completion.status === 'conflict' || completion.status === 'external-refresh') {
          setProjectedTrack(null)
          refreshRequiredRef.current = false
          setRefreshRequired(false)
          reportAutomationState(false, serverTrack.points.length, false, false)
          setError(completion.state.projectRevision
            ? 'Project changed while saving. Volume automation was reloaded; review the current points before trying again.'
            : 'Project changed while saving. Refresh project state before changing volume automation.')
        } else {
          refreshRequiredRef.current = false
          setRefreshRequired(false)
          reportAutomationState(false, serverTrack.points.length, false, false)
          setError('Could not save volume automation. Your point edits are still in the fields; try again.')
        }
      }
    }
    return response
  }

  const addOrUpdate = () => {
    if (busy || !automationAvailable) return
    const nextTime = clampAutomationTime(timeMs, durationMs)
    const nextPercent = clampAutomationPercent(percent)
    setTimeMs(nextTime)
    setPercent(nextPercent)
    setSelectedPointMs(nextTime)
    const points = replaceVolumeAutomationPoint(track.points, nextTime, nextPercent)
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
    setError('')
    setNote(`Editing point at ${formatAutomationTime(point.t_ms)}.`)
  }

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
      <p className={`insp__hint${error ? ' insp__hint--error' : ''}`} data-cut-volume-automation-status role="status" aria-live="polite">{error || note}</p>
    </div>
  )
}
