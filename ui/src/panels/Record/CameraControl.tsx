import {
  cameraPositionLabel, STUDIO_POSITIONS,
  type StudioCameraPosition, type StudioCameraShape, type StudioCameraState,
} from './studioTypes'

export type CameraDeviceState =
  | 'ready'
  | 'permission_required'
  | 'permission_denied'
  | 'busy'
  | 'no_frame'
  | 'missing'
  | 'enumerated'

export interface CameraDeviceInfo {
  id: string
  label: string
  state: CameraDeviceState
  detail: string
}

export interface CameraCapability {
  supported: boolean
  devices: CameraDeviceInfo[]
  detail: string
}

interface CameraControlProps {
  capability: CameraCapability
  enabled: boolean
  deviceId: string | null
  disabled: boolean
  rawCapture: boolean
  onEnabled: (enabled: boolean) => void
  onDevice: (deviceId: string) => void
  layout: StudioCameraState
  liveAdjustDisabled: boolean
  onPosition: (position: StudioCameraPosition) => void
  onShape: (shape: StudioCameraShape) => void
  onSize: (size: number) => void
  onReset: () => void
}

export const NO_CAMERA_CAPABILITY: CameraCapability = {
  supported: false,
  devices: [],
  detail: 'Camera recording is unavailable in this build.',
}

export function cameraSelectionError(
  capability: CameraCapability,
  enabled: boolean,
  deviceId: string | null,
  rawCapture: boolean,
): string | null {
  if (!enabled || rawCapture) return null
  if (!capability.supported) return capability.detail
  const selected = capability.devices.find((device) => device.id === deviceId)
  if (!selected) return 'Choose an available camera before recording.'
  if (selected.state === 'ready' || selected.state === 'permission_required') return null
  return selected.detail || `${selected.label} is not ready for recording.`
}

/** A single guard and explanation for the visible controls and context menu. */
export function cameraLayoutUnavailableReason(
  capability: CameraCapability,
  enabled: boolean,
  deviceId: string | null,
  rawCapture: boolean,
  liveAdjustDisabled: boolean,
): string | null {
  if (rawCapture) return 'Switch to Auto-edit to adjust camera layout.'
  if (liveAdjustDisabled) return 'Wait for the current capture operation to finish.'
  if (!capability.supported) return capability.detail || 'Camera layout is unavailable in this build.'
  if (capability.devices.length === 0) return capability.detail || 'No camera source is available.'
  if (!enabled) return 'Turn on Camera to adjust its layout.'
  const selected = capability.devices.find((device) => device.id === deviceId)
  if (!selected) return 'Choose an available camera source to adjust its layout.'
  if (selected.state !== 'ready' && selected.state !== 'permission_required') {
    return selected.detail || `${selected.label} is not ready for camera layout.`
  }
  return null
}

export function CameraControl({
  capability,
  enabled,
  deviceId,
  disabled,
  rawCapture,
  onEnabled,
  onDevice,
  layout,
  liveAdjustDisabled,
  onPosition,
  onShape,
  onSize,
  onReset,
}: CameraControlProps) {
  const selected = capability.devices.find((device) => device.id === deviceId)
  const canEnable = capability.supported && capability.devices.length > 0 && !rawCapture
  const state = selected?.state ?? (capability.devices.length ? 'missing' : 'unavailable')
  const layoutReason = cameraLayoutUnavailableReason(capability, enabled, deviceId, rawCapture, liveAdjustDisabled)
  const layoutDisabled = Boolean(layoutReason)

  return (
    <div
      className="rec-studio-controls__camera"
      data-cut-studio-camera-status={state}
      data-cut-studio-camera-available={capability.supported ? 'true' : 'false'}
      data-cut-rec-camera-enabled={enabled ? 'true' : 'false'}
    >
      <label className="rec-studio-controls__camera-toggle">
        <span>
          <strong>Camera</strong>
          <small>{rawCapture ? 'Available in Auto-edit mode' : 'Keep a separate, editable camera take'}</small>
        </span>
        <input
          type="checkbox"
          data-cut-rec-camera-toggle
          checked={enabled && !rawCapture}
          disabled={disabled || rawCapture || (!enabled && !canEnable)}
          onChange={(event) => onEnabled(event.target.checked)}
        />
      </label>

      <label className="rec-studio-controls__group">
          <span className="rec-studio-controls__label">Camera source</span>
          <select
            className="rec__select rec-studio-controls__select"
            data-cut-rec-camera-device
            value={deviceId ?? ''}
            disabled={disabled || rawCapture || !capability.supported || capability.devices.length === 0 || !enabled}
            onChange={(event) => onDevice(event.target.value)}
          >
            {capability.devices.length === 0
              ? <option value="">No camera available</option>
              : !selected && <option value="">Choose camera</option>}
            {capability.devices.map((device) => (
              <option key={device.id} value={device.id}>{device.label}</option>
            ))}
          </select>
          <small data-cut-rec-camera-state={state}>{selected?.detail ?? (capability.devices.length ? 'Choose an available camera source.' : capability.detail)}</small>
      </label>

      <div className="rec-camera-layout" data-cut-rec-camera-layout data-cut-rec-camera-layout-disabled={layoutDisabled ? 'true' : 'false'}>
        {layoutReason && <p className="rec-camera-layout__reason" id="rec-camera-layout-reason" role="status" data-cut-rec-camera-layout-reason data-cut-studio-camera-unavailable={!canEnable ? '' : undefined}>{layoutReason}</p>}
        <span className="rec-studio-controls__label">Camera position</span>
        <div className="rec-camera-layout__corners" role="group" aria-label="Camera position" aria-describedby={layoutReason ? 'rec-camera-layout-reason' : undefined}>
          {STUDIO_POSITIONS.map((position) => (
            <button
              key={position}
              type="button"
              data-cut-rec-camera-position={position}
              aria-pressed={layout.position === position}
              disabled={layoutDisabled}
              title={layoutReason ?? `Move camera to ${cameraPositionLabel(position).toLowerCase()}`}
              onClick={() => onPosition(position)}
            >{({ top_left: '↖', top_right: '↗', bottom_right: '↘', bottom_left: '↙' })[position]} {cameraPositionLabel(position)}</button>
          ))}
        </div>
        <button type="button" className="rec-camera-layout__reset" data-cut-action="record-camera-layout-reset"
          disabled={layoutDisabled} title={layoutReason ?? 'Restore this scene’s camera layout'} onClick={onReset}>Reset camera layout</button>
        <span className="rec-studio-controls__label">Camera shape</span>
        <div className="rec-camera-layout__shapes" role="group" aria-label="Camera shape" aria-describedby={layoutReason ? 'rec-camera-layout-reason' : undefined}>
          {(['circle', 'rounded_rect'] as const).map((shape) => (
            <button key={shape} type="button" data-cut-rec-camera-shape={shape}
              aria-pressed={layout.shape === shape} disabled={layoutDisabled}
              title={layoutReason ?? `Set camera shape to ${shape === 'circle' ? 'circle' : 'rounded rectangle'}`}
              onClick={() => onShape(shape)}>{shape === 'circle' ? 'Circle' : 'Rounded rectangle'}</button>
          ))}
        </div>
        <label className="rec-camera-layout__size">
          <span className="rec-studio-controls__label">Camera size <output data-cut-rec-camera-size-value>{Math.round(layout.size * 100)}%</output></span>
          <input data-cut-rec-camera-size type="range" min="12" max="50" step="1"
            value={Math.round(layout.size * 100)} disabled={layoutDisabled}
            title={layoutReason ?? `Camera size ${Math.round(layout.size * 100)}%`}
            aria-describedby={layoutReason ? 'rec-camera-layout-reason' : undefined}
            onChange={(event) => onSize(Number(event.target.value) / 100)} />
        </label>
      </div>

    </div>
  )
}
