import type { RecordingPauseCapability, RecordingPauseState } from './recordingPause'

interface RecordingPauseControlProps {
  /** Setup exposes admission; live mode renders the acknowledged action only. */
  mode?: 'setup' | 'live'
  capability: RecordingPauseCapability
  enabled: boolean
  state: RecordingPauseState
  message: string
  disabled: boolean
  onEnabled: (enabled: boolean) => void
  onControl: () => void
}

function PauseResumeButton({
  state, compact, onControl,
}: {
  state: RecordingPauseState
  compact: boolean
  onControl: () => void
}) {
  const switching = state === 'pausing' || state === 'resuming'
  const isPaused = state === 'paused' || state === 'resuming'
  const action = isPaused ? 'Resume' : 'Pause'
  return (
    <button
      type="button"
      className="rec__export-btn rec-pause__action"
      data-cut-action="record-pause-resume"
      disabled={switching}
      onClick={onControl}
    >
      {switching ? (state === 'pausing' ? 'Pausing…' : 'Resuming…') : `${isPaused ? '▶' : 'Ⅱ'} ${action}${compact ? '' : ' recording'}`}
    </button>
  )
}

/** One novice opt-in plus one live Pause/Resume action, both capability-gated. */
export function RecordingPauseControl({
  mode = 'setup',
  capability,
  enabled,
  state,
  message,
  disabled,
  onEnabled,
  onControl,
}: RecordingPauseControlProps) {
  const switching = state === 'pausing' || state === 'resuming'
  const recording = state === 'recording' || state === 'paused' || switching
  if (mode === 'live') {
    if (!recording) return null
    return (
      <div className="rec-pause rec-pause--live-only" data-cut-rec-pause-state={state}>
        <PauseResumeButton state={state} compact onControl={onControl} />
        <span className="rec-pause__live-status" data-cut-rec-pause-status role="status">{message}</span>
      </div>
    )
  }

  return (
    <div className="rec-pause" data-cut-rec-pause-supported={capability.supported ? 'true' : 'false'}>
      <label className="rec__toggle rec-pause__enable" data-cut-rec-pause-enable>
        <input
          type="checkbox"
          data-cut-action="record-pause-enable"
          checked={enabled}
          disabled={disabled || !capability.supported || recording}
          onChange={(event) => onEnabled(event.target.checked)}
        />
        Pause &amp; resume this recording
      </label>
      <p className="rec__source-note" data-cut-rec-pause-detail>{capability.detail}</p>
      {enabled && !recording && (
        <p className="rec__source-note rec__source-note--unavailable" data-cut-rec-pause-constraints>
          Uses one exact display at a whole-number frame rate. Scenes, camera, keystrokes, Window capture, and Quality are unavailable for this recording.
        </p>
      )}
    </div>
  )
}
