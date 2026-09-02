import type { ScreenRecordStatusResult } from '../../lib/clientResults'
import { recordingAudioMeterPresentation, type RecordingAudioMeterPresentation } from './recordingAudioMeterPresentation'
import './recordingAudioMeters.css'

interface RecordingAudioMetersProps {
  meters: ScreenRecordStatusResult['audio_meters'] | null
  statusError?: string | null
}

function AudioMeterRow({ meter }: { meter: RecordingAudioMeterPresentation }) {
  const compact = meter.state === 'not_requested' || meter.state === 'unavailable'
  if (compact) {
    return (
      <div
        className="rec-audio-meter rec-audio-meter--compact"
        data-cut-rec-audio-meter={meter.label.toLowerCase()}
        data-cut-rec-audio-meter-state={meter.state}
      >
        <div className="rec-audio-meter__heading">
          <strong>{meter.label}</strong>
          <span>{meter.state.replaceAll('_', ' ')}</span>
        </div>
        <small role="status" data-cut-rec-audio-meter-detail>{meter.detail}</small>
      </div>
    )
  }
  return (
    <div className="rec-audio-meter" data-cut-rec-audio-meter={meter.label.toLowerCase()} data-cut-rec-audio-meter-state={meter.state}>
      <div className="rec-audio-meter__heading">
        <strong>{meter.label}</strong>
        <span data-cut-rec-audio-meter-state>{meter.levelText ?? meter.state.replaceAll('_', ' ')}</span>
      </div>
      <div
        className="rec-audio-meter__track"
        data-cut-rec-audio-meter-level
        role="meter"
        aria-label={`${meter.label} level`}
        aria-valuemin={0}
        aria-valuemax={100}
        aria-valuenow={meter.fillPercent}
        aria-valuetext={meter.levelText ?? meter.detail}
      >
        <span className="rec-audio-meter__fill" style={{ width: `${meter.fillPercent}%` }} />
      </div>
      <small role="status" data-cut-rec-audio-meter-detail>{meter.detail}</small>
      {meter.clipping && <small data-cut-rec-audio-meter-clipping>Clipping</small>}
    </div>
  )
}

/**
 * Receives only the active-capture status projection. The parent owns whether
 * it is mounted; this component never opens or tests an audio input itself.
 */
export function RecordingAudioMeters({ meters, statusError = null }: RecordingAudioMetersProps) {
  if (!meters && !statusError) return null
  return (
    <section className="rec-audio-meters" data-cut-rec-audio-meters aria-label="Recording audio levels">
      {statusError && <p className="rec-audio-meters__error" data-cut-rec-audio-meter-status-error role="status">{statusError}</p>}
      {meters && <>
        <AudioMeterRow meter={recordingAudioMeterPresentation('Microphone', meters.microphone)} />
        <AudioMeterRow meter={recordingAudioMeterPresentation('System audio', meters.system_audio)} />
      </>}
    </section>
  )
}
