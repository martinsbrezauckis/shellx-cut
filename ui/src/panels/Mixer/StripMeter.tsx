// panels/Mixer/StripMeter — a per-track level meter for the mixer (Audio
// Monitoring v2b).
//
// Role: shows ONE audio track's live level by sampling its decoded STEM
// (export.audio{track} — the track's exact contribution to the mix, proven to sum
// to the full mix bit-for-bit) at the current playhead time. No extra audio
// playback: it reads the decoded buffer's samples around `getTimeMs()`, so it works
// headless and never adds sound. The rAF loop lives here and writes DOM via refs
// (no setState → a 60 fps meter never re-renders the mixer).
//
// Verification hook (Playwright): data-cut-strip-meter-db (RMS dBFS at the playhead).

import { useEffect, useRef } from 'react'
import { sampleBufferLevels, dbToPct } from '../Preview/meterMath'

export function StripMeter({
  channels,
  sampleRate,
  getTimeMs,
  active,
}: {
  /** Decoded stem channels (null while the stem is still rendering/decoding). */
  channels: Float32Array[] | null
  sampleRate: number
  /** Current timeline time in ms (dead-reckoned from the playhead by the mixer). */
  getTimeMs: () => number
  active: boolean
}) {
  const rootRef = useRef<HTMLDivElement>(null)
  const barRef = useRef<HTMLDivElement>(null)
  const peakRef = useRef<HTMLDivElement>(null)

  useEffect(() => {
    const root = rootRef.current
    const reset = () => {
      if (barRef.current) barRef.current.style.height = '0%'
      if (peakRef.current) peakRef.current.style.bottom = '0%'
      root?.setAttribute('data-cut-strip-meter-db', '-inf')
      root?.removeAttribute('data-cut-strip-meter-clip')
    }
    if (!channels || !active) {
      reset()
      return
    }
    let raf = 0
    let heldDb = -Infinity
    let heldUntil = 0
    const tick = () => {
      const { rmsDb, peakDb, clip } = sampleBufferLevels(channels, sampleRate, getTimeMs())
      const now = performance.now()
      if (peakDb >= heldDb || now > heldUntil) {
        heldDb = peakDb
        heldUntil = now + 1200
      }
      if (barRef.current) barRef.current.style.height = `${dbToPct(rmsDb)}%`
      if (peakRef.current) peakRef.current.style.bottom = `${dbToPct(heldDb)}%`
      if (root) {
        root.setAttribute('data-cut-strip-meter-db', isFinite(rmsDb) ? rmsDb.toFixed(1) : '-inf')
        if (clip) root.setAttribute('data-cut-strip-meter-clip', '')
        else root.removeAttribute('data-cut-strip-meter-clip')
      }
      raf = requestAnimationFrame(tick)
    }
    raf = requestAnimationFrame(tick)
    return () => cancelAnimationFrame(raf)
  }, [channels, sampleRate, getTimeMs, active])

  return (
    <div ref={rootRef} className="mx-meter" data-cut-strip-meter data-cut-strip-meter-db="-inf">
      <div ref={barRef} className="mx-meter-bar" />
      <div ref={peakRef} className="mx-meter-peak" />
    </div>
  )
}
