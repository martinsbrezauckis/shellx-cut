import type { RecordingCadence } from './recordingCadence'
import {
  finalQualityLabel,
  OUTPUT_SIZES,
  QUALITY_PROFILES,
  type RecordingOutputSize,
  type RecordingQualityCapability,
  type RecordingQualityProfile,
  type RecordingQualityResolution,
} from './recordingQuality'

interface RecordingQualityControlProps {
  capability: RecordingQualityCapability
  outputSize: RecordingOutputSize
  profile: RecordingQualityProfile
  resolution: RecordingQualityResolution | null
  cadence: RecordingCadence | null
  disabled: boolean
  unavailableReason?: string | null
  onOutputSizeChange: (value: RecordingOutputSize) => void
  onProfileChange: (value: RecordingQualityProfile) => void
}

/** Compact novice choices; the actual dimensions remain final-verifier facts. */
export function RecordingQualityControl({
  capability,
  outputSize,
  profile,
  resolution,
  cadence,
  disabled,
  unavailableReason,
  onOutputSizeChange,
  onProfileChange,
}: RecordingQualityControlProps) {
  const sizes = OUTPUT_SIZES.filter((value) => capability.output_sizes.includes(value))
  const profiles = QUALITY_PROFILES.filter((value) => capability.profiles.includes(value))
  return (
    <div className="rec__field rec__field--quality" data-cut-rec-quality-control>
      <span className="rec__label">Quality</span>
      <div className="rec__quality-controls">
        <div className="rec__quality-choice" role="group" aria-label="Recording output size" data-cut-rec-quality-size>
          {sizes.map((value) => (
            <button
              key={value}
              type="button"
              className={`rec__seg-btn${outputSize === value ? ' rec__seg-btn--on' : ''}`}
              data-cut-rec-quality-size={value}
              aria-pressed={outputSize === value}
              disabled={disabled || Boolean(unavailableReason)}
              onClick={() => onOutputSizeChange(value)}
            >
              {value === 'source' ? 'Source' : value}
            </button>
          ))}
        </div>
        <div className="rec__quality-choice" role="group" aria-label="Recording quality profile" data-cut-rec-quality-profile>
          {profiles.map((value) => (
            <button
              key={value}
              type="button"
              className={`rec__seg-btn${profile === value ? ' rec__seg-btn--on' : ''}`}
              data-cut-rec-quality-profile={value}
              aria-pressed={profile === value}
              disabled={disabled || Boolean(unavailableReason)}
              onClick={() => onProfileChange(value)}
            >
              {value === 'standard' ? 'Standard' : 'High'}
            </button>
          ))}
        </div>
        <p className="rec__source-note" data-cut-rec-quality-summary>
          {unavailableReason ?? (resolution ? finalQualityLabel(resolution, cadence) : 'Final dimensions are verified after recording.')}
        </p>
        {resolution && (
          <details className="rec__quality-advanced" data-cut-rec-quality-advanced>
            <summary data-cut-action="rec-quality-advanced">Advanced facts</summary>
            <p>Final encoder: {resolution.encoder}.</p>
          </details>
        )}
      </div>
    </div>
  )
}
