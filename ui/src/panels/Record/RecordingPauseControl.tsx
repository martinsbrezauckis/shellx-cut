import type { RecordingPauseCapability, RecordingPauseState } from './recordingPause'

interface RecordingPauseControlProps {
  capability: RecordingPauseCapability
  enabled: boolean
  state: RecordingPauseState
  message: string
  disabled: boolean
  onEnabled: (enabled: boolean) => void
  onControl: () => void
}

/** One novice opt-in plus one live Pause/Resume action, both capability-gated. */
export function RecordingPauseControl({
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
  const isPaused = state === 'paused' || state === 'resuming'
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
      {recording && (
        <div className="rec-pause__live" data-cut-rec-pause-state={state}>
          <button
            type="button"
            className="rec__export-btn rec-pause__action"
            data-cut-action="record-pause-resume"
            disabled={switching}
            onClick={onControl}
          >
            {switching ? (state === 'pausing' ? 'Pausing…' : 'Resuming…') : isPaused ? '▶ Resume recording' : 'Ⅱ Pause recording'}
          </button>
          <p className="rec__source-note" data-cut-rec-pause-status role="status">{message}</p>
        </div>
      )}
    </div>
  )
}
