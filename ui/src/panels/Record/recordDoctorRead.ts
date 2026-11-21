import type { CameraCapability } from './CameraControl'
import type { MonitorInfo, WindowInfo } from './RecordingSourceControl'
import type { RecordCard } from './recordingUiModel'

/** What a status read knows about screen capture without capturing. */
export type ScreenCheck = 'checked' | 'not_checked' | 'permission_needed' | 'not_applicable'

export interface RecordDoctorRead {
  screen_check?: ScreenCheck
  cards: RecordCard[]
  ready: boolean
  start_allowed?: boolean
  monitors?: MonitorInfo[]
  windows?: WindowInfo[]
  window_capture_supported?: boolean
  camera?: CameraCapability
  video_encoder?: unknown
  quality?: unknown
  scenes?: unknown
  pause?: unknown
}

const object = (value: unknown): value is Record<string, unknown> =>
  value !== null && typeof value === 'object' && !Array.isArray(value)

/** Validate the fields Record dereferences before a Doctor read can admit Start. */
export function recordDoctorRead(value: unknown): RecordDoctorRead | null {
  if (!object(value) || typeof value.ready !== 'boolean' || !Array.isArray(value.cards)) return null
  if (value.start_allowed !== undefined && typeof value.start_allowed !== 'boolean') return null
  if (value.window_capture_supported !== undefined && typeof value.window_capture_supported !== 'boolean') return null
  if (!value.cards.every((card) => object(card) && typeof card.name === 'string'
    && typeof card.status === 'string' && typeof card.detail === 'string')) return null
  if (value.monitors !== undefined && (!Array.isArray(value.monitors)
    || !value.monitors.every((monitor) => object(monitor) && Number.isSafeInteger(monitor.index)
      && typeof monitor.name === 'string' && typeof monitor.primary === 'boolean'
      && (monitor.id === undefined || typeof monitor.id === 'string')))) return null
  if (value.windows !== undefined && (!Array.isArray(value.windows)
    || !value.windows.every((window) => object(window) && typeof window.id === 'string'
      && typeof window.title === 'string' && typeof window.app === 'string'))) return null
  if (value.camera !== undefined && (!object(value.camera) || typeof value.camera.supported !== 'boolean'
    || typeof value.camera.detail !== 'string' || !Array.isArray(value.camera.devices)
    || !value.camera.devices.every((device) => object(device) && typeof device.id === 'string'
      && typeof device.label === 'string' && typeof device.state === 'string'
      && typeof device.detail === 'string'))) return null
  return value as unknown as RecordDoctorRead
}
