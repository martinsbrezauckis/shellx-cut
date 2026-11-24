import type { ReactNode } from 'react'
import { recordingStandsStill, type RecordingPauseCapability, type RecordingPauseState } from './recordingPause'
import { Availability } from '../../lib/availability'
import { RecordingPauseControl } from './RecordingPauseControl'

interface RecordingLiveControlsProps {
  elapsed: string
  sceneName: string
  audioMeters: ReactNode
  captureSafety: ReactNode
  /** Explicit recovery controls appear directly below transport while a Stop retains ownership. */
  recoveryControls?: ReactNode
  pauseCapability: RecordingPauseCapability
  pauseEnabled: boolean
  pauseState: RecordingPauseState
  pauseMessage: string
  markerPending: boolean
  markerConfirmation?: string | null
  /** The capture remains owned after an unacknowledged Stop; the next Stop retries it. */
  stopRetry: boolean
  onMarker: () => void
  onPauseControl: () => void
  onStop: () => void
}

/**
 * The active transport owns marker, pause, and Stop. Camera and video-timer
 * controls remain in the right recording-settings rail.
 */
export function RecordingLiveControls({
  elapsed,
  sceneName,
  audioMeters,
  captureSafety,
  recoveryControls,
  pauseCapability,
  pauseEnabled,
  pauseState,
  pauseMessage,
  markerPending,
  markerConfirmation = null,
  stopRetry,
  onMarker,
  onPauseControl,
  onStop,
}: RecordingLiveControlsProps) {
  const pauseAvailable = pauseEnabled && (
    pauseState === 'recording' || pauseState === 'paused' || pauseState === 'pausing' || pauseState === 'resuming'
  )
  const pauseReason = pauseMessage
  // A paused take says so in words and with a pause mark, not the red recording dot; its timer stands still.
  const paused = recordingStandsStill(pauseState)
  return (
    <section className="rec-live-controls" data-cut-rec-live-controls aria-label="Live recording controls">
      <div className="rec-live-controls__transport">
        <div className={`rec-live-controls__state${paused ? ' rec-live-controls__state--paused' : ''}`} data-cut-rec-hud>
          <span className={paused ? 'rec__hud-pause' : 'rec__hud-dot'} aria-hidden="true" />
          <span>
            <strong data-cut-rec-elapsed>{elapsed}</strong>
            <small>{paused ? 'Paused' : 'Recording'} · {sceneName}</small>
          </span>
        </div>
        <div className="rec-live-controls__actions" role="group" aria-label="Recording actions">
          <button
            type="button"
            className="rec__export-btn rec-live-controls__marker"
            data-cut-action="record-marker"
            disabled={markerPending}
            onClick={onMarker}
          >
            {markerPending ? 'Saving marker…' : 'Flag marker (F12)'}
          </button>
          {markerConfirmation && (
            <span
              className="rec-live-controls__marker-feedback"
              data-cut-rec-marker-feedback
              role="status"
              aria-live="polite"
            >
              {markerConfirmation}
            </span>
          )}
          <button type="button" className="rec__stop" data-cut-action="record-stop"
            data-cut-rec-stop-retry={stopRetry ? 'true' : undefined} onClick={onStop}>
            ■ {stopRetry ? 'Retry Stop' : 'Stop'} (F9)
          </button>
          <Availability rule="record.pause">
          {pauseAvailable ? (
            <RecordingPauseControl
              mode="live"
              capability={pauseCapability}
              enabled={pauseEnabled}
              state={pauseState}
              message={pauseMessage}
              disabled={false}
              onEnabled={() => undefined}
              onControl={onPauseControl}
            />
          ) : (
            <span className="rec-live-controls__unavailable" data-cut-rec-live-pause-unavailable role="status">
              <strong>Pause unavailable</strong>
              <small>{pauseReason}</small>
            </span>
          )}
          </Availability>
        </div>
      </div>
      {recoveryControls && <div className="rec-live-controls__recovery">{recoveryControls}</div>}
      {captureSafety && <div className="rec-live-controls__safety">{captureSafety}</div>}
      {audioMeters && <div className="rec-live-controls__meters" data-cut-rec-live-audio-meters>{audioMeters}</div>}
    </section>
  )
}
