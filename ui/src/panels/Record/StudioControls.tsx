import {
  STUDIO_BACKGROUND_PRESETS,
  studioBackgroundPreset,
  type StudioBackground,
  type StudioCameraPosition,
  type StudioCameraShape,
  type StudioState,
} from './studioTypes'
import type { ReactNode } from 'react'
import { CameraControl, type CameraCapability } from './CameraControl'
import { RecordingSettingsTabs, type RecordingSettingsSection } from './RecordingSettingsTabs'
import './cameraControl.css'
import './recordingSettingsTabs.css'

interface StudioControlsProps {
  videoTimerControl: ReactNode
  captureTimingControl: ReactNode
  videoQualityControl: ReactNode
  studio: StudioState
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
  onCameraReset: () => void
}

export function StudioControls({
  videoTimerControl,
  captureTimingControl,
  videoQualityControl,
  studio,
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
  onCameraReset,
}: StudioControlsProps) {
  const pages: Record<RecordingSettingsSection, ReactNode> = {
    camera: (
      <CameraControl
        capability={cameraCapability}
        enabled={studio.camera.enabled}
        deviceId={cameraDeviceId}
        disabled={configurationDisabled}
        rawCapture={rawCapture}
        onEnabled={onCameraEnabled}
        onDevice={onCameraDevice}
        layout={studio.camera}
        liveAdjustDisabled={liveAdjustDisabled}
        onPosition={onCameraPosition}
        onShape={onCameraShape}
        onSize={onCameraSize}
        onReset={onCameraReset}
      />
    ),
    background: (
      <label className="rec-studio-controls__group" data-cut-studio-background={studio.background} data-cut-studio-preset={studio.background}>
        <span className="rec-studio-controls__label">Behind the recording</span>
        <select
          className="rec__select rec-studio-controls__select"
          data-cut-studio-background-select
          value={studio.background}
          disabled={liveAdjustDisabled}
          onChange={(event) => onBackground(event.target.value as StudioBackground)}
        >
          {STUDIO_BACKGROUND_PRESETS.map((preset) => <option key={preset.id} value={preset.id}>{preset.label}</option>)}
        </select>
        <small data-cut-studio-background-description>{studioBackgroundPreset(studio.background).description}</small>
      </label>
    ),
    timer: (
      <>
        <p className="rec-settings-tabs__note">The on-video timer appears in the finished video. It does not stop capture or control capture Pause.</p>
        {videoTimerControl}
      </>
    ),
    timing: captureTimingControl,
    quality: videoQualityControl,
  }

  return (
    <aside className="rec-studio-controls" aria-label="Recording settings">
      <div className="rec-studio-controls__head">
        <div>
          <h2>3 · Recording settings</h2>
        </div>
      </div>
      <p className="rec-settings-tabs__note">Choose a setting. Its controls appear below.</p>
      <RecordingSettingsTabs pages={pages} />
    </aside>
  )
}
