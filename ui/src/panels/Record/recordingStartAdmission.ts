import { useSyncExternalStore } from 'react'

export type RecordingStartAdmission = 'none' | 'unknown'

let admission: RecordingStartAdmission = 'none'
const listeners = new Set<() => void>()

function subscribe(listener: () => void): () => void {
  listeners.add(listener)
  return () => listeners.delete(listener)
}

function snapshot(): RecordingStartAdmission {
  return admission
}

/**
 * A malformed successful start may already own native capture. This app-session
 * latch survives Record unmount/remount and intentionally has no setup reset.
 * This session can reset only by restarting ShellX Cut. Diagnostics may inspect
 * the recorder but cannot clear uncertainty; local setup controls never reset it.
 */
export function markRecordingStartAdmissionUnknown(): void {
  if (admission === 'unknown') return
  admission = 'unknown'
  listeners.forEach((listener) => listener())
}

export function useRecordingStartAdmission(): RecordingStartAdmission {
  return useSyncExternalStore(subscribe, snapshot, snapshot)
}
