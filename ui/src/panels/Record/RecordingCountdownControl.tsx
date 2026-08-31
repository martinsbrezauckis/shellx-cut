import {
  RECORDING_COUNTDOWN_CHOICES,
  type RecordingCountdownSeconds,
} from './recordingCountdown'

interface RecordingCountdownControlProps {
  value: RecordingCountdownSeconds
  disabled: boolean
  onValueChange: (seconds: RecordingCountdownSeconds) => void
}

/** A small preflight choice, deliberately kept in the existing setup scan path. */
export function RecordingCountdownControl({
  value,
  disabled,
  onValueChange,
}: RecordingCountdownControlProps) {
  return (
    <div className="rec__field rec__field--countdown" data-cut-rec-countdown-control>
      <span className="rec__label">Countdown</span>
      <div className="rec__seg" role="group" aria-label="Recording countdown">
        {RECORDING_COUNTDOWN_CHOICES.map((seconds) => (
          <button
            key={seconds}
            type="button"
            className={`rec__seg-btn${value === seconds ? ' rec__seg-btn--on' : ''}`}
            data-cut-rec-countdown={seconds === 0 ? 'off' : seconds}
            aria-pressed={value === seconds}
            disabled={disabled}
            onClick={() => onValueChange(seconds)}
          >
            {seconds === 0 ? 'Off' : `${seconds}s`}
          </button>
        ))}
      </div>
      <p className="rec__source-note" data-cut-rec-countdown-note>
        Starts after the count so you can switch to the app you want to record.
      </p>
    </div>
  )
}
