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

export function CameraControl({
  capability,
  enabled,
  deviceId,
  disabled,
  rawCapture,
  onEnabled,
  onDevice,
}: CameraControlProps) {
  const selected = capability.devices.find((device) => device.id === deviceId)
  const canEnable = capability.supported && capability.devices.length > 0 && !rawCapture
  const state = selected?.state ?? (capability.devices.length ? 'missing' : 'unavailable')

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

      {enabled && !rawCapture && (
        <label className="rec-studio-controls__group">
          <span className="rec-studio-controls__label">Camera source</span>
          <select
            className="rec__select rec-studio-controls__select"
            data-cut-rec-camera-device
            value={deviceId ?? ''}
            disabled={disabled}
            onChange={(event) => onDevice(event.target.value)}
          >
            {capability.devices.map((device) => (
              <option key={device.id} value={device.id}>{device.label}</option>
            ))}
          </select>
          <small data-cut-rec-camera-state={state}>{selected?.detail ?? capability.detail}</small>
        </label>
      )}

      {!canEnable && (
        <div className="rec-studio-controls__unavailable" data-cut-studio-camera-unavailable role="status">
          <strong>Camera unavailable</strong>
          <span>{rawCapture ? 'Switch to Auto-edit to record screen and camera as editable sources.' : capability.detail}</span>
        </div>
      )}
    </div>
  )
}
