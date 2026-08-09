export type StudioCameraPosition = 'top_left' | 'top_right' | 'bottom_right' | 'bottom_left'
export type StudioCameraShape = 'circle' | 'rounded_rect'
export type StudioBackground = 'none' | 'blur_screen' | 'solid' | 'gradient'

export interface StudioCameraState {
  enabled: boolean
  visible: boolean
  position: StudioCameraPosition
  x: number
  y: number
  size: number
  shape: StudioCameraShape
}

export interface StudioState {
  camera: StudioCameraState
  background: StudioBackground
  hotkeyStatus: 'desktop-f9' | 'focused-only'
}

export interface StudioRawStreams {
  screen?: string | null
  camera?: string | null
  mic?: string | null
  system?: string | null
  /** Windows WASAPI first-packet timing sidecar; absent for legacy/non-Windows captures. */
  system_timing?: string | null
  studio_events?: string | null
}

export type CursorCoordinateState = 'exact' | 'approximate' | 'unavailable'

export interface CursorCorrelation {
  source: 'legacy_unknown' | 'rdevin_absolute' | 'wayland_pipewire_metadata' | 'wayland_evdev_relative'
  state: CursorCoordinateState
  exact_clicks: number
  approximate_clicks: number
  unavailable_clicks: number
  max_metadata_age_ms?: number
  detail?: string
}

export function cursorCorrelationLabel(correlation: CursorCorrelation | null): string {
  if (!correlation || correlation.state === 'unavailable') return 'Pointer positions unavailable'
  if (correlation.state === 'approximate') {
    if (correlation.unavailable_clicks > 0) {
      return `${correlation.unavailable_clicks} pointer position${correlation.unavailable_clicks === 1 ? '' : 's'} unavailable`
    }
    return `${correlation.approximate_clicks} pointer position${correlation.approximate_clicks === 1 ? '' : 's'} approximate`
  }
  return 'Pointer positions accurate'
}

export interface StudioEventPayload {
  source: 'camera' | 'recording' | 'background'
  kind: 'visibility' | 'transform' | 'marker' | 'style'
  visible?: boolean
  x?: number
  y?: number
  size?: number
  shape?: StudioCameraShape
  label?: string
  background?: StudioBackground
}

export const STUDIO_POSITIONS: StudioCameraPosition[] = [
  'top_left',
  'top_right',
  'bottom_right',
  'bottom_left',
]

export function clamp01(value: number): number {
  if (!Number.isFinite(value)) return 0
  return Math.min(1, Math.max(0, value))
}

export function clampCameraSize(value: number): number {
  if (!Number.isFinite(value)) return 0.22
  return Math.min(0.5, Math.max(0.12, value))
}

export function cameraPositionLabel(position: StudioCameraPosition): string {
  switch (position) {
    case 'top_left': return 'Top left'
    case 'top_right': return 'Top right'
    case 'bottom_right': return 'Bottom right'
    case 'bottom_left': return 'Bottom left'
  }
}

export function backgroundLabel(background: StudioBackground): string {
  switch (background) {
    case 'none': return 'None'
    case 'blur_screen': return 'Blur'
    case 'solid': return 'Solid'
    case 'gradient': return 'Gradient'
  }
}

export function placementForPosition(position: StudioCameraPosition, size: number): { x: number; y: number } {
  const margin = 0.04
  const normalizedWidth = clampCameraSize(size) * (9 / 16)
  const right = Math.max(margin, 1 - margin - normalizedWidth)
  const bottom = Math.max(margin, 1 - margin - clampCameraSize(size))
  switch (position) {
    case 'top_left': return { x: margin, y: margin }
    case 'top_right': return { x: right, y: margin }
    case 'bottom_right': return { x: right, y: bottom }
    case 'bottom_left': return { x: margin, y: bottom }
  }
}

export function closestPosition(x: number, y: number): StudioCameraPosition {
  const horizontal = x < 0.5 ? 'left' : 'right'
  const vertical = y < 0.5 ? 'top' : 'bottom'
  if (vertical === 'top' && horizontal === 'left') return 'top_left'
  if (vertical === 'top' && horizontal === 'right') return 'top_right'
  if (vertical === 'bottom' && horizontal === 'left') return 'bottom_left'
  return 'bottom_right'
}

export function defaultStudioState(): StudioState {
  const size = 0.22
  const position = 'bottom_right'
  const placement = placementForPosition(position, size)
  return {
    camera: {
      enabled: false,
      visible: true,
      position,
      x: placement.x,
      y: placement.y,
      size,
      shape: 'circle',
    },
    background: 'gradient',
    hotkeyStatus: 'desktop-f9',
  }
}
