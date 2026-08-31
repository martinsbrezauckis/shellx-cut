// Shared, presentation-safe model for a future native region picker.
//
// This module intentionally does not contain an opaque native monitor id, a
// screenshot, or a capture ticket. The native picker must keep those values at
// the desktop/server boundary until it has issued an exact one-use selection.

export type RecordingSourceKind = 'display' | 'window' | 'region'

export interface RegionPickerDisplay {
  /** Presentation-only label. It never identifies the native target. */
  readonly label: string
  /** A UI-local display key, supplied only by a future capability bridge. */
  readonly uiId: string
  readonly width: number
  readonly height: number
}

export interface RegionRect {
  readonly x: number
  readonly y: number
  readonly width: number
  readonly height: number
}

export interface RegionSuggestion {
  readonly displayUiId: string
  readonly rect: RegionRect
}

export type RegionPickerCapability =
  | {
      readonly availability: 'available'
      readonly displays: readonly RegionPickerDisplay[]
      /** A previous region is only a suggestion; it is never applied automatically. */
      readonly lastRegion?: RegionSuggestion
    }
  | {
      readonly availability: 'unavailable'
      readonly reason: string
    }

/**
 * Current product truth. The macOS crop transport remains deliberately
 * private pending compiled/native qualification, so the recorder must not
 * collect a region that it cannot faithfully pass to capture.
 */
export const REGION_PICKER_UNAVAILABLE: RegionPickerCapability = {
  availability: 'unavailable',
  reason: 'Region capture is not available in this release. Choose Display or Window instead.',
}

export const REGION_PICKER_STEP = 2
export const REGION_PICKER_FAST_STEP = 20
export const REGION_PICKER_MIN_SIZE = 32

function even(value: number): number {
  return Math.round(value / 2) * 2
}

function finite(value: number): number {
  return Number.isFinite(value) ? value : 0
}

/** Keep a region inside one selected display; cross-display selection has no representation. */
export function fitRegionToDisplay(
  rect: RegionRect,
  display: RegionPickerDisplay,
): RegionRect | null {
  const displayWidth = Math.floor(finite(display.width))
  const displayHeight = Math.floor(finite(display.height))
  if (displayWidth < REGION_PICKER_MIN_SIZE || displayHeight < REGION_PICKER_MIN_SIZE) return null

  const width = Math.min(displayWidth, Math.max(REGION_PICKER_MIN_SIZE, even(finite(rect.width))))
  const height = Math.min(displayHeight, Math.max(REGION_PICKER_MIN_SIZE, even(finite(rect.height))))
  const maxX = Math.max(0, displayWidth - width)
  const maxY = Math.max(0, displayHeight - height)
  return {
    x: Math.min(maxX, Math.max(0, even(finite(rect.x)))),
    y: Math.min(maxY, Math.max(0, even(finite(rect.y)))),
    width,
    height,
  }
}

/** Move a selected region without resizing it or allowing it to leave its display. */
export function nudgeRegion(
  rect: RegionRect,
  display: RegionPickerDisplay,
  direction: 'left' | 'right' | 'up' | 'down',
  fast = false,
): RegionRect | null {
  const amount = fast ? REGION_PICKER_FAST_STEP : REGION_PICKER_STEP
  const delta = direction === 'left' ? [-amount, 0]
    : direction === 'right' ? [amount, 0]
      : direction === 'up' ? [0, -amount]
        : [0, amount]
  return fitRegionToDisplay({ ...rect, x: rect.x + delta[0], y: rect.y + delta[1] }, display)
}

/** Translate a display-pixel selection into a safe, easy-to-render preview rectangle. */
export function regionPreviewStyle(rect: RegionRect, display: RegionPickerDisplay): Record<string, string> {
  return {
    left: `${(rect.x / display.width) * 100}%`,
    top: `${(rect.y / display.height) * 100}%`,
    width: `${(rect.width / display.width) * 100}%`,
    height: `${(rect.height / display.height) * 100}%`,
  }
}
