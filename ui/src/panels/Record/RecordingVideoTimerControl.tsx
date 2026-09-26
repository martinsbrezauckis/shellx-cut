import { useEffect, useState } from 'react'
import {
  RECORDING_TIMER_PRESET_MINUTES,
  recordingSceneTimerLabel,
  recordingTimerClock,
  recordingTimerDraft,
  type RecordingSceneTimer,
  type RecordingSceneTimerAction,
  type RecordingSceneTimerKind,
} from './recordingScenes'
import type { RecordingSceneTimerStatus } from './useRecordingScenes'

interface RecordingVideoTimerControlProps {
  timer: RecordingSceneTimer
  status: RecordingSceneTimerStatus
  recording: boolean
  disabled: boolean
  unavailableReason?: string | null
  liveSupported: boolean
  onTimerChange: (timer: RecordingSceneTimer) => void
  onTimerControl: (action: RecordingSceneTimerAction) => void
}

/** Video overlay timing is separate from the capture length and capture Pause. */
export function RecordingVideoTimerControl({
  timer,
  status,
  recording,
  disabled,
  unavailableReason,
  liveSupported,
  onTimerChange,
  onTimerControl,
}: RecordingVideoTimerControlProps) {
  const [draftKind, setDraftKind] = useState<RecordingSceneTimerKind>(timer.kind)
  const [draftClock, setDraftClock] = useState(timer.kind === 'countdown' ? recordingTimerClock(timer.duration_ms) : '05:00')
  const [draftError, setDraftError] = useState<string | null>(null)

  useEffect(() => {
    setDraftKind(timer.kind)
    if (timer.kind === 'countdown') setDraftClock(recordingTimerClock(timer.duration_ms))
    setDraftError(null)
  }, [timer])

  const apply = () => {
    const result = recordingTimerDraft(draftKind, draftClock)
    if (result.error || !result.timer) {
      setDraftError(result.error)
      return
    }
    setDraftError(null)
    onTimerChange(result.timer)
  }
  const unavailable = unavailableReason ?? (recording && !liveSupported
    ? 'Live video timer controls are unavailable because this recorder does not support scene events.'
    : null)
  const actionReason = (action: RecordingSceneTimerAction): string | null => {
    if (unavailable) return unavailable
    if (timer.kind === 'off') return 'The video timer is off for this recording.'
    if (status.state === 'switching') return 'Wait for the recorder to confirm the last timer change.'
    if (action === 'pause' && status.state === 'paused') return 'The timer is already paused.'
    if (action === 'resume' && (status.state === 'running' || status.state === 'awaiting_ack')) return 'Pause the timer before resuming it.'
    if (status.state === 'ended' && (action === 'pause' || action === 'resume' || action === 'end')) return 'The timer has ended. Reset or restart it.'
    return null
  }

  return (
    <div className="rec-video-timer" data-cut-rec-video-timer={timer.kind}>
      {!recording ? (
        <>
          <div className="rec-video-timer__choices" role="group" aria-label="Video timer mode">
            {(['off', 'elapsed', 'countdown'] as const).map((kind) => (
              <button key={kind} type="button" data-cut-rec-video-timer-mode={kind}
                aria-pressed={draftKind === kind} disabled={disabled}
                onClick={() => { setDraftKind(kind); setDraftError(null) }}>
                {kind === 'off' ? 'Off' : kind === 'elapsed' ? 'Count up' : 'Count down'}
              </button>
            ))}
          </div>
          <p className="rec-settings-tabs__note" data-cut-rec-video-timer-current>Active on-video timer: {recordingSceneTimerLabel(timer)}</p>
          {draftKind === 'countdown' && (
            <div className="rec-video-timer__duration">
              <span className="rec-studio-controls__label">Count down duration</span>
              <div className="rec-video-timer__choices" role="group" aria-label="Suggested countdown durations">
                {RECORDING_TIMER_PRESET_MINUTES.map((minutes) => (
                  <button key={minutes} type="button" data-cut-rec-video-timer-preset={minutes}
                    aria-pressed={draftClock === recordingTimerClock(minutes * 60_000)} disabled={disabled}
                    onClick={() => { setDraftClock(recordingTimerClock(minutes * 60_000)); setDraftError(null) }}>{minutes} min</button>
                ))}
              </div>
              <label className="rec-video-timer__clock">
                <span>Duration · MM:SS or HH:MM:SS</span>
                <input type="text" inputMode="numeric" autoComplete="off" spellCheck={false} maxLength={8}
                  value={draftClock} placeholder="05:00" aria-invalid={Boolean(draftError)}
                  data-cut-rec-video-timer-clock disabled={disabled}
                  onChange={(event) => { setDraftClock(event.target.value); setDraftError(null) }}
                  onKeyDown={(event) => { if (event.key === 'Enter') { event.preventDefault(); apply() } }} />
              </label>
            </div>
          )}
          <button type="button" className="rec-video-timer__apply" data-cut-rec-video-timer-apply
            disabled={disabled} onClick={apply}>Apply timer</button>
          <p className="rec-settings-tabs__note" role={draftError ? 'alert' : 'status'} data-cut-rec-video-timer-draft={draftError ? 'invalid' : 'pending'}>
            {draftError ?? 'Changes take effect for the next recording after Apply or Enter.'}
          </p>
        </>
      ) : (
        <>
          <span className="rec-studio-controls__label">While recording · video timer only</span>
          <div className="rec-video-timer__actions" role="group" aria-label="Live video timer">
            {(['pause', 'resume', 'reset', 'restart', 'end'] as const).map((action) => {
              const reason = actionReason(action)
              return (
                <button key={action} type="button" data-cut-rec-video-timer-action={action}
                  disabled={disabled || Boolean(reason)} title={reason ?? `Ask the recorder to ${action} the video timer.`}
                  onClick={() => onTimerControl(action)}>{action === 'end' ? 'End timer' : action[0].toUpperCase() + action.slice(1)}</button>
              )
            })}
          </div>
          {unavailable && <p className="rec-settings-tabs__note" role="status">{unavailable}</p>}
        </>
      )}
      <p className="rec-settings-tabs__note" role="status" data-cut-rec-video-timer-state={status.state}>{status.message}</p>
      <p className="rec-settings-tabs__note">Reaching zero does not stop recording. Capture length and the start countdown are separate settings.</p>
    </div>
  )
}
