// A recording this window did not start (for example one an agent started
// through screen_record.start) must still be visible to the person, with the
// recording bar and Stop. While this window holds no capture it asks the
// engine which recording runs and hands an unknown one to `adopt`.

import { useEffect, useRef } from 'react'
import { callVerb } from '../lib/client'

export interface ActiveRecording {
  captureId: string
  ready: boolean
  sinceFirstFrameMs: number | null
}

/** A running project recording from a `screen_record.active` result. */
export function activeRecording(result: unknown): ActiveRecording | null {
  if (!result || typeof result !== 'object') return null
  const r = result as { active?: unknown; kind?: unknown; capture_id?: unknown; ready?: unknown; since_first_screen_frame_ms?: unknown }
  if (r.active !== true || r.kind !== 'recording' || typeof r.capture_id !== 'string' || !r.capture_id) return null
  return {
    captureId: r.capture_id,
    ready: r.ready === true,
    sinceFirstFrameMs: typeof r.since_first_screen_frame_ms === 'number' ? r.since_first_screen_frame_ms : null,
  }
}

export function useActiveRecordingAdoption({ idle, adopt, intervalMs = 1500 }: {
  /** True while this window holds no capture and is not starting or stopping one. */
  idle: () => boolean
  adopt: (recording: ActiveRecording) => void
  intervalMs?: number
}): void {
  const idleRef = useRef(idle)
  idleRef.current = idle
  const adoptRef = useRef(adopt)
  adoptRef.current = adopt
  useEffect(() => {
    let checking = false
    const timer = window.setInterval(() => {
      if (checking || !idleRef.current()) return
      checking = true
      void callVerb('screen_record.active', {})
        .then((reply) => {
          const recording = reply.ok ? activeRecording(reply.result) : null
          if (recording && idleRef.current()) adoptRef.current(recording)
        })
        .catch(() => {})
        .finally(() => { checking = false })
    }, intervalMs)
    return () => window.clearInterval(timer)
  }, [intervalMs])
}
