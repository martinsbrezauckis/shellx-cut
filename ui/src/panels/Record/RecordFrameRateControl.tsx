import {
  parseRecordingFrameRate,
  recordingFrameRateReason,
  RECORDING_FRAME_RATE_MAX,
  RECORDING_FRAME_RATE_MIN,
  RECORDING_FRAME_RATE_PRESETS,
} from './recordingFrameRate'

interface RecordFrameRateControlProps {
  value: number
  customValue: string
  disabled: boolean
  onValueChange: (value: number) => void
  onCustomValueChange: (value: string) => void
}

function frameRateLabel(value: number): string {
  return String(value)
}

export function RecordFrameRateControl({
  value,
  customValue,
  disabled,
  onValueChange,
  onCustomValueChange,
}: RecordFrameRateControlProps) {
  const customError = customValue.trim() ? recordingFrameRateReason(customValue) : null

  const choosePreset = (frameRate: number) => {
    onCustomValueChange('')
    onValueChange(frameRate)
  }

  const updateCustomValue = (nextValue: string) => {
    onCustomValueChange(nextValue)
    const parsed = parseRecordingFrameRate(nextValue)
    if (parsed !== null) onValueChange(parsed)
  }

  return (
    <div className="rec__field rec__field--fps" data-cut-rec-fps-control>
      <span className="rec__label">Frame rate</span>
      <div className="rec__seg" aria-label="Common recording frame rates">
        {RECORDING_FRAME_RATE_PRESETS.map((frameRate) => (
          <button
            key={frameRate}
            type="button"
            className={`rec__seg-btn${value === frameRate ? ' rec__seg-btn--on' : ''}`}
            data-cut-rec-fps={frameRate}
            aria-label={`${frameRate} FPS`}
            aria-pressed={value === frameRate}
            disabled={disabled}
            onClick={() => choosePreset(frameRate)}
          >
            {frameRate}
          </button>
        ))}
      </div>
      <label className="rec__fps-custom" data-cut-rec-fps-custom>
        <span>Custom</span>
        <input
          className="rec__fps-input"
          data-cut-rec-fps-custom-input
          type="number"
          min={RECORDING_FRAME_RATE_MIN}
          max={RECORDING_FRAME_RATE_MAX}
          step="any"
          inputMode="decimal"
          placeholder="FPS"
          value={customValue}
          disabled={disabled}
          aria-label="Custom recording frame rate in FPS"
          aria-describedby="cut-rec-fps-status"
          aria-invalid={customError ? 'true' : undefined}
          onChange={(event) => updateCustomValue(event.target.value)}
        />
        <span>FPS</span>
      </label>
      <p
        id="cut-rec-fps-status"
        className={`rec__fps-status${customError ? ' rec__fps-status--error' : ''}`}
        data-cut-rec-fps-current={frameRateLabel(value)}
        data-cut-rec-fps-error={customError ? 'true' : 'false'}
        role={customError ? 'alert' : 'status'}
      >
        {customError
          ? `${customError} Requested ${frameRateLabel(value)} FPS remains selected until you correct it.`
          : `Requested ${frameRateLabel(value)} FPS. Custom accepts ${RECORDING_FRAME_RATE_MIN}–${RECORDING_FRAME_RATE_MAX} FPS.`}
      </p>
    </div>
  )
}
