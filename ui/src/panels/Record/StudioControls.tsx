import {
  cameraPositionLabel,
  STUDIO_POSITIONS,
  cursorCorrelationLabel,
  STUDIO_BACKGROUND_PRESETS,
  studioBackgroundPreset,
  type StudioBackground,
  type StudioCameraPosition,
  type StudioCameraShape,
  type CursorCorrelation,
  type StudioRawStreams,
  type StudioState,
} from './studioTypes'
import type { ReactNode } from 'react'
import { CameraControl, type CameraCapability } from './CameraControl'

interface StudioControlsProps {
  sceneControl?: ReactNode
  studio: StudioState
  rawStreams: StudioRawStreams | null
  cursorCorrelation: CursorCorrelation | null
  onBackground: (background: StudioBackground) => void
  cameraCapability: CameraCapability
  cameraDeviceId: string | null
  configurationDisabled: boolean
  liveAdjustDisabled: boolean
  rawCapture: boolean
  onCameraEnabled: (enabled: boolean) => void
  onCameraDevice: (deviceId: string) => void
  onCameraPosition: (position: StudioCameraPosition) => void
  onCameraShape: (shape: StudioCameraShape) => void
  onCameraSize: (size: number) => void
}

export function StudioControls({
  sceneControl,
  studio,
  rawStreams,
  cursorCorrelation,
  onBackground,
  cameraCapability,
  cameraDeviceId,
  configurationDisabled,
  liveAdjustDisabled,
  rawCapture,
  onCameraEnabled,
  onCameraDevice,
  onCameraPosition,
  onCameraShape,
  onCameraSize,
}: StudioControlsProps) {
  const streamCount = rawStreams
    ? [rawStreams.screen, rawStreams.camera, rawStreams.mic, rawStreams.system, rawStreams.studio_events].filter(Boolean).length
    : 0

  return (
    <aside className="rec-studio-controls">
      {sceneControl}
      <CameraControl
        capability={cameraCapability}
        enabled={studio.camera.enabled}
        deviceId={cameraDeviceId}
        disabled={configurationDisabled}
        rawCapture={rawCapture}
        onEnabled={onCameraEnabled}
        onDevice={onCameraDevice}
      />

      {studio.camera.enabled && !rawCapture && (
        <div className="rec-studio-controls__group" data-cut-rec-camera-layout>
          <span className="rec-studio-controls__label">Camera layout</span>
          <div className="rec-studio-controls__positions" role="group" aria-label="Camera position">
            {STUDIO_POSITIONS.map((position) => (
              <button
                key={position}
                type="button"
                className={`rec-studio-controls__pos${studio.camera.position === position ? ' rec-studio-controls__pos--on' : ''}`}
                data-cut-rec-camera-position={position}
                aria-label={cameraPositionLabel(position)}
                aria-pressed={studio.camera.position === position}
                disabled={liveAdjustDisabled}
                onClick={() => onCameraPosition(position)}
              />
            ))}
          </div>
          <select
            className="rec__select rec-studio-controls__select"
            data-cut-rec-camera-shape
            value={studio.camera.shape}
            disabled={liveAdjustDisabled}
            onChange={(event) => onCameraShape(event.target.value as StudioCameraShape)}
          >
            <option value="circle">Circle</option>
            <option value="rounded_rect">Rounded rectangle</option>
          </select>
          <label>
            <span className="rec-studio-controls__label">Size</span>
            <input
              className="rec-studio-controls__range"
              data-cut-rec-camera-size
              type="range"
              min="0.12"
              max="0.5"
              step="0.01"
              value={studio.camera.size}
              disabled={liveAdjustDisabled}
              onChange={(event) => onCameraSize(Number(event.target.value))}
            />
          </label>
        </div>
      )}

      <label
        className="rec-studio-controls__group"
        data-cut-studio-background={studio.background}
        data-cut-studio-preset={studio.background}
      >
        <span className="rec-studio-controls__label">Background</span>
        <select
          className="rec__select rec-studio-controls__select"
          data-cut-studio-background-select
          value={studio.background}
          disabled={liveAdjustDisabled}
          onChange={(event) => onBackground(event.target.value as StudioBackground)}
        >
          {STUDIO_BACKGROUND_PRESETS.map((preset) => (
            <option key={preset.id} value={preset.id}>{preset.label}</option>
          ))}
        </select>
        <small data-cut-studio-background-description>{studioBackgroundPreset(studio.background).description}</small>
      </label>

      <div
        className="rec-studio-controls__group rec-studio-controls__streams"
        data-cut-studio-raw-streams={streamCount}
      >
        <span className="rec-studio-controls__label">Raw streams</span>
        <div className="rec-studio-controls__chips">
          <span data-on={rawStreams?.screen ? 'true' : 'false'}>Screen</span>
          <span data-on={rawStreams?.camera ? 'true' : 'false'}>Camera</span>
          <span data-on={rawStreams?.mic ? 'true' : 'false'}>Mic</span>
          <span data-on={rawStreams?.system ? 'true' : 'false'}>System</span>
          <span data-on={rawStreams?.studio_events ? 'true' : 'false'}>Events</span>
        </div>
      </div>

      <div
        className="rec-studio-controls__group"
        data-cut-rec-cursor-correlation={cursorCorrelation?.state ?? 'unavailable'}
      >
        <span className="rec-studio-controls__label">Pointer positions</span>
        <span role="status">{cursorCorrelationLabel(cursorCorrelation)}</span>
        {cursorCorrelation?.detail && <small>{cursorCorrelation.detail}</small>}
      </div>

      <div
        className="rec-studio-controls__group rec-studio-controls__hotkeys"
        data-cut-studio-hotkey-status={studio.hotkeyStatus}
      >
        <span className="rec-studio-controls__label">Hotkeys</span>
        <div className="rec-studio-controls__chips">
          <span data-on="true">F9 Rec</span>
          <span data-on="true">F12 Mark</span>
        </div>
      </div>
    </aside>
  )
}
