import type { ProjectIdentity } from '../lib/projectIdentity'
import type { StudioBackground, StudioCameraShape } from '../panels/Record/studioTypes'
import type { RecordingCountdownSeconds } from '../panels/Record/recordingCountdown'

export interface RecordingStudioPreset {
  background: StudioBackground
  camera?: { x: number; y: number; size: number; shape: StudioCameraShape }
}

export interface RecordingPreset {
  schema: 'shellx-cut/recording-preset/1'
  source: { kind: 'display'; monitorId: string } | { kind: 'portal_display' } | { kind: 'window'; windowId: string }
  fps: number
  durationMs: number | null
  startCountdownSeconds?: RecordingCountdownSeconds
  audio: boolean
  systemAudio: boolean
  keys: boolean
  raw: boolean
  quality?: { output_size: 'source' | '1080p' | '720p'; profile: 'standard' | 'high' }
  cameraId?: string
  scenes?: unknown
  pause?: { mode: 'enabled' }
  studio?: RecordingStudioPreset
}

/** Camera-free takes keep the selected timer, but start on Screen. */
export function normalizeRecordingPresetForStart(preset: RecordingPreset): RecordingPreset {
  if (!preset.raw && preset.cameraId) return preset
  const { cameraId: _cameraId, scenes, studio, ...withoutCamera } = preset
  const candidate = scenes && typeof scenes === 'object' ? scenes as {
    initial_scene_id?: unknown
    presets?: Array<{ id?: unknown; layout?: { kind?: unknown } }>
  } : null
  const screenExists = Array.isArray(candidate?.presets)
    && candidate.presets.some(scene => scene.id === 'screen' && scene.layout?.kind === 'screen')
  return {
    ...withoutCamera,
    ...(studio ? { studio: { background: studio.background } } : {}),
    ...(screenExists ? { scenes: { ...candidate, initial_scene_id: 'screen' } } : {}),
  }
}

const KEY = 'shellx-cut.recording-preset.v1'

export function parseRecordingPreset(value: unknown): RecordingPreset | null {
  if (!value || typeof value !== 'object') return null
  const p = value as Partial<RecordingPreset>
  if (p.schema !== 'shellx-cut/recording-preset/1') return null
  if (!p.source || (p.source.kind === 'display'
    ? typeof p.source.monitorId !== 'string' || !p.source.monitorId.trim() || p.source.monitorId.trim() !== p.source.monitorId
    : p.source.kind === 'portal_display' ? false
    : p.source.kind === 'window'
      ? typeof p.source.windowId !== 'string' || !p.source.windowId.trim() || p.source.windowId.trim() !== p.source.windowId
      : true)) return null
  if (typeof p.fps !== 'number' || !Number.isFinite(p.fps) || p.fps < 1 || p.fps > 240) return null
  if (p.durationMs !== null && (typeof p.durationMs !== 'number' || !Number.isSafeInteger(p.durationMs) || p.durationMs <= 0)) return null
  if (p.startCountdownSeconds !== undefined && ![0, 3, 5].includes(p.startCountdownSeconds)) return null
  if (typeof p.audio !== 'boolean' || typeof p.systemAudio !== 'boolean' || typeof p.keys !== 'boolean' || typeof p.raw !== 'boolean') return null
  if (p.quality && (!['source', '1080p', '720p'].includes(p.quality.output_size) || !['standard', 'high'].includes(p.quality.profile))) return null
  if (p.cameraId !== undefined && (typeof p.cameraId !== 'string' || !p.cameraId.trim())) return null
  if (p.pause && p.pause.mode !== 'enabled') return null
  if (p.studio) {
    if (!['gradient', 'solid', 'blur_screen', 'none'].includes(p.studio.background)) return null
    const camera = p.studio.camera
    if (camera && (!p.cameraId || !Number.isFinite(camera.x) || !Number.isFinite(camera.y)
      || !Number.isFinite(camera.size) || camera.x < 0 || camera.x > 1 || camera.y < 0 || camera.y > 1
      || camera.size < 0.12 || camera.size > 0.5 || !['circle', 'rounded_rect'].includes(camera.shape))) return null
  }
  return {
    schema: p.schema,
    source: p.source.kind === 'display'
      ? { kind: 'display', monitorId: p.source.monitorId }
      : p.source.kind === 'portal_display'
        ? { kind: 'portal_display' }
        : { kind: 'window', windowId: p.source.windowId },
    fps: p.fps,
    durationMs: p.durationMs,
    ...(p.startCountdownSeconds !== undefined ? { startCountdownSeconds: p.startCountdownSeconds } : {}),
    audio: p.audio,
    systemAudio: p.systemAudio,
    keys: p.keys,
    raw: p.raw,
    ...(p.quality ? { quality: p.quality } : {}),
    ...(p.cameraId ? { cameraId: p.cameraId } : {}),
    ...(p.scenes ? { scenes: p.scenes } : {}),
    ...(p.pause ? { pause: p.pause } : {}),
    ...(p.studio ? { studio: p.studio } : {}),
  }
}

export function loadRecordingPreset(): RecordingPreset | null {
  try {
    const preset = parseRecordingPreset(JSON.parse(localStorage.getItem(KEY) ?? 'null'))
    return preset ? normalizeRecordingPresetForStart(preset) : null
  } catch { return null }
}

export function saveRecordingPreset(preset: RecordingPreset): void {
  const safe = parseRecordingPreset(normalizeRecordingPresetForStart(preset))
  if (!safe) throw new Error('Cannot save an invalid recording setup')
  localStorage.setItem(KEY, JSON.stringify(safe))
}

export function sameProjectIdentity(a: ProjectIdentity | undefined, b: ProjectIdentity | undefined): boolean {
  return Boolean(a && b && a.origin_path_sha256 === b.origin_path_sha256 && a.project_name === b.project_name)
}

/** Linux has no in-app monitor identity; its current Doctor admits the portal picker. */
export function doctorAllowsPortalDisplay(doctor: unknown): boolean {
  if (!doctor || typeof doctor !== 'object') return false
  const d = doctor as { start_allowed?: boolean; monitors?: unknown; cards?: Array<{ name?: string }> }
  return d.start_allowed === true
    && Array.isArray(d.monitors) && d.monitors.length === 0
    && Array.isArray(d.cards)
    && d.cards.some((card) => card.name === 'gstreamer')
    && d.cards.some((card) => card.name === 'wayland_input')
}

/** First-use F9 needs one current screen source without opening Record. */
export function firstUseRecordingPreset(doctor: unknown): RecordingPreset | null {
  if (!doctor || typeof doctor !== 'object') return null
  const d = doctor as { start_allowed?: boolean; ready?: boolean; monitors?: unknown }
  if ((d.start_allowed ?? d.ready) !== true) return null
  const monitors = Array.isArray(d.monitors) ? d.monitors as Array<{ id?: unknown; primary?: unknown }> : []
  const named = monitors.filter((m) => m && typeof m.id === 'string' && m.id.trim() === m.id && m.id.length > 0)
  const primary = named.find((m) => m.primary === true) ?? named[0]
  const source: RecordingPreset['source'] | null = primary
    ? { kind: 'display', monitorId: primary.id as string }
    : doctorAllowsPortalDisplay(d) ? { kind: 'portal_display' } : null
  if (!source) return null
  return {
    schema: 'shellx-cut/recording-preset/1', source, fps: 30,
    durationMs: null, startCountdownSeconds: 0,
    audio: false, systemAudio: false, keys: false, raw: false,
    studio: { background: 'gradient' },
  }
}

export function validateRecordingPreset(preset: RecordingPreset, doctor: unknown): string | null {
  if (!doctor || typeof doctor !== 'object') return 'Recorder checks did not return a current result.'
  const d = doctor as Record<string, unknown>
  if ((d.start_allowed ?? d.ready) !== true) return 'Screen capture is unavailable. Check Recorder setup and permissions.'
  if (preset.source.kind === 'portal_display') {
    if (!doctorAllowsPortalDisplay(d)) return 'The Linux screen-sharing portal is not available. Review Recorder setup.'
  } else if (preset.source.kind === 'display') {
    const id = preset.source.monitorId
    if (!Array.isArray(d.monitors) || !d.monitors.some((m: { id?: string }) => m.id === id)) return 'The saved display is no longer available. Choose a current display.'
  } else {
    if (d.window_capture_supported === false) return 'Window capture is unavailable on this machine. Open Record and choose Display.'
    const id = preset.source.windowId
    if (!Array.isArray(d.windows) || !d.windows.some((w: { id?: string }) => w.id === id)) return 'The saved window is no longer available. Choose a current window.'
  }
  if (preset.quality && !(d.quality as { supported?: boolean } | undefined)?.supported) return 'The selected recording quality is unavailable on this machine.'
  if (preset.cameraId) {
    const camera = d.camera as { supported?: boolean; devices?: Array<{ id?: string }> } | undefined
    if (!camera?.supported || !camera.devices?.some((device) => device.id === preset.cameraId)) return 'The saved camera is no longer available. Choose a current camera.'
  }
  if (preset.pause && !(d.pause as { supported?: boolean } | undefined)?.supported) return 'Pause and resume is unavailable on this machine.'
  if (preset.scenes && !(d.scenes as { supported?: boolean } | undefined)?.supported) return 'Recording scenes are unavailable. Review the setup.'
  return null
}
