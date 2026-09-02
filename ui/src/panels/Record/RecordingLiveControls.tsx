import type { ReactNode } from 'react'
import type { RecordingPauseCapability, RecordingPauseState } from './recordingPause'
import { RecordingPauseControl } from './RecordingPauseControl'

interface RecordingLiveControlsProps {
  elapsed: string
  sceneName: string
  sceneControl: ReactNode
  audioMeters: ReactNode
  captureSafety: ReactNode
  pauseCapability: RecordingPauseCapability
  pauseEnabled: boolean
  pauseState: RecordingPauseState
  pauseMessage: string
  markerPending: boolean
  onMarker: () => void
  onPauseControl: () => void
  onStop: () => void
}

/**
 * The active-recorder surface has one home for live mutations. Scene controls
 * are supplied by the existing durable scene component; marker, pause, and
 * stop remain direct calls into their established recorder paths.
 */
export function RecordingLiveControls({
  elapsed,
  sceneName,
  sceneControl,
  audioMeters,
  captureSafety,
  pauseCapability,
  pauseEnabled,
  pauseState,
  pauseMessage,
  markerPending,
  onMarker,
  onPauseControl,
  onStop,
}: RecordingLiveControlsProps) {
  const pauseAvailable = pauseEnabled && (
    pauseState === 'recording' || pauseState === 'paused' || pauseState === 'pausing' || pauseState === 'resuming'
  )
  const pauseReason = pauseEnabled
    ? pauseMessage
    : pauseCapability.supported
      ? 'Pause & resume was not enabled before this recording started.'
      : pauseCapability.detail
  return (
    <section className="rec-live-controls" data-cut-rec-live-controls aria-label="Live recording controls">
      <div className="rec-live-controls__transport">
        <div className="rec-live-controls__state" data-cut-rec-hud>
          <span className="rec__hud-dot" aria-hidden="true" />
          <span>
            <strong data-cut-rec-elapsed>{elapsed}</strong>
            <small>Recording · {sceneName}</small>
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
          <button
            type="button"
            className="rec__stop"
            data-cut-action="record-stop"
            onClick={onStop}
          >
            ■ Stop (F9)
          </button>
        </div>
      </div>
      <div className="rec-live-controls__scenes" data-cut-rec-live-scenes>
        {sceneControl}
      </div>
      {captureSafety && <div className="rec-live-controls__safety">{captureSafety}</div>}
      {audioMeters && <div className="rec-live-controls__meters" data-cut-rec-live-audio-meters>{audioMeters}</div>}
    </section>
  )
}
