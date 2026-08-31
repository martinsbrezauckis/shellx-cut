import type {
  StudioCameraPosition,
  StudioCameraShape,
  StudioState,
} from './studioTypes'
import { placementForPosition } from './studioTypes'

export type RecordingSceneCorner = 'top_right' | 'bottom_right'
export type RecordingSceneShape = 'circle' | 'rounded_rect'

/** Matches recorder-core's tagged scene-layout DTO. */
export type RecordingSceneLayout =
  | { kind: 'screen' }
  | {
    kind: 'presenter_pip'
    corner: RecordingSceneCorner
    size_percent: number
    shape: RecordingSceneShape
  }

/** Matches recorder-core's capture-wide timer DTO. */
export type RecordingSceneTimer =
  | { kind: 'off' }
  | { kind: 'elapsed' }
  | { kind: 'countdown'; duration_ms: number }

/** Public preset data. There is deliberately no local background persistence claim. */
export interface RecordingScenePreset {
  id: string
  name: string
  /** Non-zero recorder-core PresetRevision. */
  preset_revision: number
  layout: RecordingSceneLayout
}

export interface RecordingSceneStartConfig {
  /** Non-zero recorder-core CatalogRevision, never a fingerprint string. */
  catalog_revision: number
  initial_scene_id: string
  presets: RecordingScenePreset[]
  /** One timer belongs to the complete capture, never an individual scene. */
  timer: RecordingSceneTimer
}

export interface RecordingSceneActivateArgs {
  capture_id: string
  scene_id: string
  /** Non-zero recorder-core PresetRevision for this exact scene. */
  preset_revision: number
}

/** The durable projection returned after start admission or a live mutation. */
export interface RecordingSceneAcknowledgement {
  scene_id: string
  preset_revision: number
  /** Only this explicit acknowledgement permits the UI to say the switch saved. */
  saved: boolean
  logical_media_time_ms: number
  scene: {
    active_scene_id: string
    composition: unknown
    timer: unknown
  }
}

export type RecordingSceneActivateResult = RecordingSceneAcknowledgement
export type RecordingSceneStartResult = RecordingSceneAcknowledgement

export type RecordingSceneTimerAction = 'pause' | 'resume' | 'reset' | 'restart' | 'end'

export interface RecordingSceneTimerArgs {
  capture_id: string
  action: RecordingSceneTimerAction
}

export interface RecordingSceneTimerResult {
  action: RecordingSceneTimerAction
  state: 'running' | 'paused' | 'ended'
  logical_media_time_ms: number
  scene: {
    active_scene_id: string
    composition: unknown
    timer: unknown
  }
}

export interface RecordingSceneCapability {
  supported: boolean
  catalog_revision: number | null
  detail: string
}

export const RECORDING_SCENE_CATALOG_REVISION = 1

export const RECORDING_SCENE_PRESETS: RecordingScenePreset[] = [
  {
    id: 'screen',
    name: 'Screen only',
    preset_revision: 1,
    layout: { kind: 'screen' },
  },
  {
    id: 'presenter-corner',
    name: 'Presenter corner',
    preset_revision: 2,
    layout: { kind: 'presenter_pip', corner: 'bottom_right', size_percent: 22, shape: 'circle' },
  },
  {
    id: 'demo-corner',
    name: 'Demo corner',
    preset_revision: 3,
    layout: { kind: 'presenter_pip', corner: 'top_right', size_percent: 18, shape: 'rounded_rect' },
  },
]

export const NO_RECORDING_SCENE_CAPABILITY: RecordingSceneCapability = {
  supported: false,
  catalog_revision: null,
  detail: 'This recorder does not confirm recording-scene support yet.',
}

function object(value: unknown): Record<string, unknown> | null {
  return value !== null && typeof value === 'object' && !Array.isArray(value)
    ? value as Record<string, unknown>
    : null
}

/** Parse only an explicit, versioned Doctor capability; unknown data stays unavailable. */
export function recordingSceneCapability(value: unknown): RecordingSceneCapability {
  const candidate = object(value)
  if (!candidate || candidate.supported !== true || !positiveRevision(candidate.catalog_revision)) {
    return NO_RECORDING_SCENE_CAPABILITY
  }
  return {
    supported: true,
    catalog_revision: candidate.catalog_revision,
    detail: typeof candidate.detail === 'string' && candidate.detail
      ? candidate.detail
      : 'Live scene switches are available for this recording.',
  }
}

export function recordingSceneById(id: string): RecordingScenePreset {
  return RECORDING_SCENE_PRESETS.find((scene) => scene.id === id) ?? RECORDING_SCENE_PRESETS[0]
}

/** Reject incomplete or mismatched wire acknowledgements before changing UI state. */
export function recordingSceneAcknowledged(
  value: unknown,
  expected: RecordingScenePreset,
): value is RecordingSceneAcknowledgement {
  const candidate = object(value)
  const scene = object(candidate?.scene)
  return candidate?.saved === true
    && candidate.scene_id === expected.id
    && candidate.preset_revision === expected.preset_revision
    && typeof candidate.logical_media_time_ms === 'number'
    && Number.isSafeInteger(candidate.logical_media_time_ms)
    && candidate.logical_media_time_ms >= 0
    && scene?.active_scene_id === expected.id
}

export function recordingSceneTimerAcknowledged(
  value: unknown,
  action: RecordingSceneTimerAction,
): value is RecordingSceneTimerResult {
  const candidate = object(value)
  const scene = object(candidate?.scene)
  return candidate?.action === action
    && (candidate.state === 'running' || candidate.state === 'paused' || candidate.state === 'ended')
    && typeof candidate.logical_media_time_ms === 'number'
    && Number.isSafeInteger(candidate.logical_media_time_ms)
    && candidate.logical_media_time_ms >= 0
    && typeof scene?.active_scene_id === 'string'
}

export function sceneUsesCamera(scene: RecordingScenePreset): boolean {
  return scene.layout.kind === 'presenter_pip'
}

function positiveRevision(value: unknown): value is number {
  return typeof value === 'number' && Number.isSafeInteger(value) && value > 0
}

export function recordingSceneStartConfig(
  catalogRevision: number,
  initialSceneId: string,
  timer: RecordingSceneTimer,
): RecordingSceneStartConfig {
  const initialScene = recordingSceneById(initialSceneId)
  return {
    catalog_revision: catalogRevision,
    initial_scene_id: initialScene.id,
    presets: RECORDING_SCENE_PRESETS,
    timer,
  }
}

function formatTimer(totalSeconds: number): string {
  const minutes = Math.floor(totalSeconds / 60)
  return `${minutes}:${(totalSeconds % 60).toString().padStart(2, '0')}`
}

/** Copy for configuration is local until `screen_record.start` returns success. */
export function recordingSceneTimerLabel(timer: RecordingSceneTimer): string {
  if (timer.kind === 'off') return 'No recording timer will be requested.'
  if (timer.kind === 'elapsed') return 'Count up will begin when the recording starts.'
  return `Countdown will start at ${formatTimer(Math.ceil(timer.duration_ms / 1_000))}; it will not stop recording.`
}

function cameraPosition(corner: RecordingSceneCorner): StudioCameraPosition {
  return corner === 'top_right' ? 'top_right' : 'bottom_right'
}

function cameraShape(shape: RecordingSceneShape): StudioCameraShape {
  return shape === 'rounded_rect' ? 'rounded_rect' : 'circle'
}

/** Apply just the named camera composition, preserving independent Studio choices. */
export function studioStateForRecordingScene(scene: RecordingScenePreset, current: StudioState): StudioState {
  if (scene.layout.kind === 'screen') {
    return { ...current, camera: { ...current.camera, enabled: false, visible: false } }
  }
  const size = scene.layout.size_percent / 100
  const position = cameraPosition(scene.layout.corner)
  const placement = placementForPosition(position, size)
  return {
    ...current,
    camera: {
      ...current.camera,
      ...placement,
      position,
      size,
      shape: cameraShape(scene.layout.shape),
      enabled: true,
      visible: true,
    },
  }
}
