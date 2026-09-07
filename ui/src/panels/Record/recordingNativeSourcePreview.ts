//! Pure selection and presentation boundaries for native source preview.

import type {
  ScreenRecordSourcePreviewCapability,
  ScreenRecordSourcePreviewFrame,
  ScreenRecordSourcePreviewStatus,
} from '../../lib/clientResults'
import type { MonitorInfo, WindowInfo } from './RecordingSourceControl'
import type { RecordingSourceKind } from './regionPickerModel'

export type RecordingSourcePreviewSelection =
  | { kind: 'monitor'; monitor_id: string }
  | { kind: 'window'; window_id: string }
  | { kind: 'portal' }

/** The sole preview target derived from the existing Screen & sound control. */
export interface RecordingSourcePreviewTarget {
  source: RecordingSourcePreviewSelection
  label: string
}

export interface RecordingSourcePreviewPresentation {
  available: boolean
  state: ScreenRecordSourcePreviewStatus['state']
  detail: string
  frameUrl: string | null
}

/** Older or malformed capability envelopes never gain a guessed source mode. */
export function recordingSourcePreviewCapability(value: unknown): ScreenRecordSourcePreviewCapability {
  if (value && typeof value === 'object') {
    const capability = value as Record<string, unknown>
    if (capability.state === 'available'
      && (capability.source_selection === 'exact' || capability.source_selection === 'portal')) {
      return { state: 'available', source_selection: capability.source_selection }
    }
    if (capability.state === 'unsupported' && typeof capability.prerequisite === 'string') {
      return { state: 'unsupported', prerequisite: capability.prerequisite }
    }
  }
  return {
    state: 'unsupported',
    prerequisite: 'This recorder did not advertise a supported native source-preview selection mode.',
  }
}

export function recordingSourcePreviewSelectionKey(source: RecordingSourcePreviewSelection): string {
  switch (source.kind) {
    case 'monitor': return `monitor:${source.monitor_id}`
    case 'window': return `window:${source.window_id}`
    case 'portal': return 'portal'
  }
}

/**
 * Selects only an opaque current identity for an exact-capability backend. A
 * Portal target is possible only when the server capability explicitly says
 * so; user-agent and OS guesses are deliberately not inputs here.
 */
export function recordingSourcePreviewTarget(
  capability: ScreenRecordSourcePreviewCapability | null,
  sourceKind: RecordingSourceKind,
  monitors: readonly MonitorInfo[],
  monitorIdx: number | null,
  windows: readonly WindowInfo[],
  windowTargetId: string | null,
): RecordingSourcePreviewTarget | null {
  if (!capability || capability.state !== 'available' || sourceKind === 'region') return null
  if (capability.source_selection === 'portal') {
    return { source: { kind: 'portal' }, label: 'Choose the preview source in the system dialog.' }
  }
  if (sourceKind === 'window') {
    const window = windowTargetId && windows.find((candidate) => candidate.id === windowTargetId)
    return window ? { source: { kind: 'window', window_id: window.id }, label: 'Preview selected window.' } : null
  }
  // `null` means current primary/first; a non-null index must still resolve
  // against the current Doctor enumeration before its opaque id is admitted.
  const monitor = monitorIdx === null
    ? monitors.find((candidate) => candidate.primary) ?? monitors[0]
    : monitors.find((candidate) => candidate.index === monitorIdx)
  return monitor?.id
    ? { source: { kind: 'monitor', monitor_id: monitor.id }, label: 'Preview selected display.' }
    : null
}

/**
 * Presents only an admitted memory snapshot. A mismatched, absent, or malformed
 * frame is never turned into a browser image, so this model cannot invent pixels.
 */
export function recordingSourcePreviewPresentation(
  capability: ScreenRecordSourcePreviewCapability | null,
  status: ScreenRecordSourcePreviewStatus,
  frame: ScreenRecordSourcePreviewFrame | null,
): RecordingSourcePreviewPresentation {
  if (capability === null) {
    return {
      available: false,
      state: 'idle',
      detail: 'Checking native source-preview support.',
      frameUrl: null,
    }
  }
  if (capability.state === 'unsupported') {
    return {
      available: false,
      state: 'unavailable',
      detail: capability.prerequisite,
      frameUrl: null,
    }
  }
  const currentFrame = frame
    && status.state === 'ready'
    && status.has_frame
    && status.generation !== null
    && frame.generation === status.generation
    && frame.mime === 'image/bmp'
    && frame.bytes > 0
    && frame.bytes <= 4 * 1024 * 1024
    && frame.base64.length > 0
  return {
    available: true,
    state: status.state,
    detail: currentFrame ? 'Receiving a native source frame.' : stateDetail(status),
    frameUrl: currentFrame ? `data:image/bmp;base64,${frame.base64}` : null,
  }
}

function stateDetail(status: ScreenRecordSourcePreviewStatus): string {
  if (status.state === 'unavailable' && status.unavailable_reason) return status.unavailable_reason
  switch (status.state) {
    case 'idle': return 'Choose one current source to begin preview.'
    case 'starting': return 'Waiting for the native source to deliver a frame.'
    case 'ready': return 'Preview synchronization is pending.'
    case 'paused': return 'Preview is paused and its native session was released.'
    case 'hidden': return 'Preview is hidden and will not resume automatically.'
    case 'permission_required': return 'Screen-sharing permission is required.'
    case 'permission_denied': return 'Screen-sharing permission was denied.'
    case 'source_lost': return 'The selected native source stopped delivering frames.'
    case 'unavailable': return 'Native source preview is unavailable on this backend.'
    case 'stopped': return 'Preview is stopped and its source was released.'
  }
}
