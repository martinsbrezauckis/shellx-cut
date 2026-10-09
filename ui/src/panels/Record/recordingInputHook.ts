/** Registration observation only; it does not prove event delivery or coordinates. */
export interface RecordingInputHook {
  state: 'unobserved' | 'registered' | 'unavailable'
  backend: 'rdevin_windows' | 'rdevin_macos' | 'rdevin_x11' | 'wayland_evdev' | 'unknown'
  reason?: 'startup_failed'
  capture_keys: boolean
}

export const UNOBSERVED_INPUT_HOOK: RecordingInputHook = {
  state: 'unobserved', backend: 'unknown', capture_keys: false,
}

/** Legacy and malformed payloads cannot assert a native startup failure. */
export function recordingInputHook(value: unknown): RecordingInputHook {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return UNOBSERVED_INPUT_HOOK
  const input = value as Record<string, unknown>
  if (Object.keys(input).some(key => !['state', 'backend', 'reason', 'capture_keys'].includes(key))) return UNOBSERVED_INPUT_HOOK
  if (typeof input.capture_keys !== 'boolean') return UNOBSERVED_INPUT_HOOK
  const rdevin = input.backend === 'rdevin_windows' || input.backend === 'rdevin_macos' || input.backend === 'rdevin_x11'
  const noReason = input.reason === undefined || input.reason === null
  const valid = input.state === 'registered' ? rdevin && noReason
    : input.state === 'unavailable' ? rdevin && input.reason === 'startup_failed'
      : input.state === 'unobserved' && (input.backend === 'unknown' || input.backend === 'wayland_evdev') && noReason
  if (!valid) return UNOBSERVED_INPUT_HOOK
  return { state: input.state as RecordingInputHook['state'], backend: input.backend as RecordingInputHook['backend'],
    capture_keys: input.capture_keys, ...(input.state === 'unavailable' ? { reason: 'startup_failed' as const } : {}) }
}

export function recordingInputHookWarning(input: RecordingInputHook, saved: boolean): string | null {
  if (input.state !== 'unavailable') return null
  return `Mouse input could not start. This recording may lack pointer animation, click highlights and automatic zoom.${input.capture_keys ? ' Opted-in key capture was also unavailable.' : ''} ${saved ? 'Video saved.' : 'Video recording continues.'}`
}
