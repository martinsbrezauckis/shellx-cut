import { callVerb } from '../lib/client'
import type { WorkspaceMode } from '../layout/useLayout'

/** The workspace modes in production order. */
export const WORKSPACE_MODES: { id: WorkspaceMode; label: string; hint: string }[] = [
  { id: 'edit', label: 'Edit', hint: 'The full timeline editor' },
  { id: 'record', label: 'Record', hint: 'Capture your screen → polished clip' },
]

/** Export menu entries, each mapping to one public verb call. */
export const EXPORT_OPTIONS = [
  { id: 'video', group: 'deliver', label: 'Video (.mp4) — render the timeline', defaultPath: 'ShellX Cut render.mp4', filters: [{ name: 'MP4 video', extensions: ['mp4'] }], run: (path?: string) => callVerb('render.final', { preset: 'standard', ...(path ? { path } : {}) }) },
  { id: 'audio', group: 'deliver', label: 'Audio (.mp3) — timeline audio only', defaultPath: 'audio.mp3', filters: [{ name: 'MP3 audio', extensions: ['mp3'] }], run: (path?: string) => callVerb('export.audio', { format: 'mp3', ...(path ? { path } : {}) }) },
  { id: 'gif', group: 'deliver', label: 'GIF (looping — first 15s)', defaultPath: 'clip.gif', filters: [{ name: 'GIF image', extensions: ['gif'] }], run: (path?: string) => callVerb('export.gif', { ...(path ? { path } : {}) }) },
  { id: 'frame', group: 'deliver', label: 'Still frame at playhead (→ Assets)', defaultPath: 'frame.jpg', filters: [{ name: 'JPEG image', extensions: ['jpg', 'jpeg'] }], run: (path?: string) => callVerb('export.frame', { at_ms: 0, ...(path ? { path } : {}) }) },
  { id: 'pub_youtube', group: 'publish', label: 'YouTube (1080p 16:9)', defaultPath: 'youtube.mp4', filters: [{ name: 'MP4 video', extensions: ['mp4'] }], run: (path?: string) => callVerb('export.publish', { platform: 'youtube', ...(path ? { path } : {}) }) },
  { id: 'pub_tiktok', group: 'publish', label: 'TikTok / Shorts (9:16)', defaultPath: 'tiktok.mp4', filters: [{ name: 'MP4 video', extensions: ['mp4'] }], run: (path?: string) => callVerb('export.publish', { platform: 'tiktok', ...(path ? { path } : {}) }) },
  { id: 'pub_reels', group: 'publish', label: 'Instagram Reels (9:16)', defaultPath: 'reels.mp4', filters: [{ name: 'MP4 video', extensions: ['mp4'] }], run: (path?: string) => callVerb('export.publish', { platform: 'reels', ...(path ? { path } : {}) }) },
  { id: 'pub_x', group: 'publish', label: 'X / Twitter (16:9)', defaultPath: 'x-twitter.mp4', filters: [{ name: 'MP4 video', extensions: ['mp4'] }], run: (path?: string) => callVerb('export.publish', { platform: 'x', ...(path ? { path } : {}) }) },
  { id: 'fcpxml', group: 'interchange', label: 'Final Cut Pro XML', defaultPath: 'timeline.fcpxml', filters: [{ name: 'Final Cut Pro XML', extensions: ['fcpxml'] }], run: (path?: string) => callVerb('export.xml', { format: 'fcpxml', ...(path ? { path } : {}) }) },
  { id: 'premiere', group: 'interchange', label: 'Premiere XML', defaultPath: 'premiere.xml', filters: [{ name: 'Premiere XML', extensions: ['xml'] }], run: (path?: string) => callVerb('export.xml', { format: 'premiere', ...(path ? { path } : {}) }) },
  { id: 'resolve', group: 'interchange', label: 'Resolve XML', defaultPath: 'resolve.fcpxml', filters: [{ name: 'Resolve XML', extensions: ['fcpxml', 'xml'] }], run: (path?: string) => callVerb('export.xml', { format: 'resolve', ...(path ? { path } : {}) }) },
  { id: 'otio', group: 'interchange', label: 'OpenTimelineIO (.otio)', defaultPath: 'timeline.otio', filters: [{ name: 'OpenTimelineIO', extensions: ['otio'] }], run: (path?: string) => callVerb('export.otio', { ...(path ? { path } : {}) }) },
  { id: 'edl', group: 'interchange', label: 'CMX3600 EDL (.edl)', defaultPath: 'timeline.edl', filters: [{ name: 'CMX3600 EDL', extensions: ['edl'] }], run: (path?: string) => callVerb('export.edl', { ...(path ? { path } : {}) }) },
  { id: 'srt', group: 'text', label: 'Captions (.srt)', defaultPath: 'captions.srt', filters: [{ name: 'SRT captions', extensions: ['srt'] }], run: (path?: string) => callVerb('export.srt', { ...(path ? { path } : {}) }) },
  { id: 'vtt', group: 'text', label: 'Captions (.vtt — web)', defaultPath: 'captions.vtt', filters: [{ name: 'WebVTT captions', extensions: ['vtt'] }], run: (path?: string) => callVerb('export.vtt', { ...(path ? { path } : {}) }) },
  { id: 'ass', group: 'text', label: 'Captions (.ass — styled / karaoke)', defaultPath: 'captions.ass', filters: [{ name: 'ASS captions', extensions: ['ass'] }], run: (path?: string) => callVerb('export.ass', { ...(path ? { path } : {}) }) },
  { id: 'chapters', group: 'text', label: 'Chapters (YouTube/podcast)', defaultPath: 'chapters.txt', filters: [{ name: 'Chapter list', extensions: ['txt'] }], run: (path?: string) => callVerb('export.chapters', { ...(path ? { path } : {}) }) },
  { id: 'transcript', group: 'text', label: 'Transcript (.md)', defaultPath: 'transcript.md', filters: [{ name: 'Markdown transcript', extensions: ['md'] }], run: (path?: string) => callVerb('export.transcript', { format: 'md', timestamps: true, ...(path ? { path } : {}) }) },
] as const

export const EXPORT_GROUPS = [
  { id: 'deliver', label: 'Deliver' },
  { id: 'publish', label: 'Publish' },
  { id: 'interchange', label: 'Interchange' },
  { id: 'text', label: 'Text and captions' },
] as const

export const ASYNC_RENDER_IDS = new Set<string>(['video', 'pub_youtube', 'pub_tiktok', 'pub_reels', 'pub_x'])

export const PRESETS = ['draft', 'standard', 'high'] as const
export type Preset = (typeof PRESETS)[number]

export const PROFILES = ['auto', 'talking_head', 'silent_screen_demo'] as const
export type Profile = (typeof PROFILES)[number]

export const ASPECTS = ['project', '16:9', '9:16', '1:1', '4:5'] as const
export type Aspect = (typeof ASPECTS)[number]

export const REFRAME_PRESETS = ['talking_head', 'sports', 'pets', 'cars', 'general'] as const
export type ReframePreset = (typeof REFRAME_PRESETS)[number]

export const FORMATS = ['h264', 'hevc', 'vp9', 'prores', 'av1'] as const
export type FileFormat = (typeof FORMATS)[number]

export const FORMAT_LABELS: Record<FileFormat, string> = {
  h264: 'Video (.mp4) — H.264',
  hevc: 'HEVC (.mp4) — smaller',
  vp9: 'WebM (.webm) — VP9',
  prores: 'ProRes (.mov) — pro',
  av1: 'AV1 (.mp4) — best, slow on CPU',
}

export const LOUDNESS = ['off', '-14', '-16', '-23'] as const
export type Loudness = (typeof LOUDNESS)[number]

export const LOUDNESS_LABELS: Record<Loudness, string> = {
  off: 'Off — no normalization',
  '-14': '−14 LUFS — social (YouTube/Spotify)',
  '-16': '−16 LUFS — podcast (Apple)',
  '-23': '−23 LUFS — EBU R128 broadcast',
}

export function selectedOption<T extends string>(options: readonly T[], value: string, fallback: T): T {
  for (const option of options) {
    if (option === value) return option
  }
  return fallback
}
