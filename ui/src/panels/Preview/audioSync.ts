export const MONITOR_AUDIO_RESYNC_DRIFT_S = 0.75
export const MONITOR_AUDIO_FORCE_RESYNC_DRIFT_S = 2.0
export const MONITOR_AUDIO_RESYNC_COOLDOWN_MS = 1500

export interface MonitorAudioResyncArgs {
  audioTimeS: number
  playheadMs: number
  nowMs: number
  lastResyncAtMs: number
  driftS?: number
  forceDriftS?: number
  cooldownMs?: number
}

export function monitorAudioResyncTarget({
  audioTimeS,
  playheadMs,
  nowMs,
  lastResyncAtMs,
  driftS = MONITOR_AUDIO_RESYNC_DRIFT_S,
  forceDriftS = MONITOR_AUDIO_FORCE_RESYNC_DRIFT_S,
  cooldownMs = MONITOR_AUDIO_RESYNC_COOLDOWN_MS,
}: MonitorAudioResyncArgs): number | null {
  if (!Number.isFinite(audioTimeS) || !Number.isFinite(playheadMs)) return null
  const targetS = playheadMs / 1000
  const drift = Math.abs(audioTimeS - targetS)
  if (drift <= driftS) return null
  const recentlyResynced = lastResyncAtMs > 0 && Number.isFinite(nowMs) && nowMs - lastResyncAtMs < cooldownMs
  if (recentlyResynced && drift <= forceDriftS) return null
  return targetS
}
