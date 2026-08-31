export interface RecordCard {
  name: string
  status: string
  detail: string
}

const RECORD_CARD_LABELS: Record<string, string> = {
  ffmpeg: 'Media tools',
  screen_capture: 'Screen capture',
  system_audio: 'System audio',
  input_hook: 'Pointer and keys',
  gstreamer: 'Linux capture',
  wayland_input: 'Linux input',
}

export function recordCardLabel(name: string): string {
  return RECORD_CARD_LABELS[name] ?? name.replaceAll('_', ' ')
}

export const DUR_PRESETS: { label: string; ms: number | null }[] = [
  { label: 'No limit', ms: null },
  { label: '10s', ms: 10_000 },
  { label: '30s', ms: 30_000 },
  { label: '1 min', ms: 60_000 },
  { label: '2 min', ms: 120_000 },
]

export function fmtElapsed(totalSec: number): string {
  const m = Math.floor(totalSec / 60)
  const s = totalSec % 60
  return `${m}:${s.toString().padStart(2, '0')}`
}

export function outputFileLabel(path: string): string {
  return path.split(/[\\/]/).filter(Boolean).at(-1) || 'Chosen file'
}

export function failureReason(error: unknown): string {
  if (error instanceof TypeError) return 'server unreachable'
  return error instanceof Error && error.message ? error.message : 'server unreachable'
}
