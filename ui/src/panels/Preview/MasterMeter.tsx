// panels/Preview/MasterMeter — master output level meter (Audio Monitoring v2a).
//
// Role: a real dBFS level meter driven by a Web Audio AnalyserNode tapped off the
// timeline-audio <audio> element (the SAME full mix export.audio produces, so the
// meter reads the EXACT export level — WYSIWYG, never a JS re-mix). The AnalyserNode
// is created in Preview/index.tsx (it owns the <audio> + the play gesture that
// unlocks the AudioContext); this component only VISUALISES it.
//
// Design: the rAF metering loop lives HERE and writes straight to DOM via refs
// (bar height, peak-hold tick, data-* attributes) — it never calls setState, so a
// 60 fps meter does NOT re-render the Preview tree. Ballistics: instantaneous RMS
// for the bar, true-peak with a ~1.2 s peak-hold tick, a clip flag at ~0 dBFS.
//
// Verification hooks (Playwright): data-cut-meter-db (RMS dBFS), data-cut-meter-peak
// (held peak dBFS), data-cut-meter-clip (present when peak >= ~0 dBFS).

import { useEffect, useRef } from 'react'
import { blockLevels, dbToPct } from './meterMath'

export function MasterMeter({
  analyser,
  active,
}: {
  analyser: AnalyserNode | null
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
      root?.setAttribute('data-cut-meter-db', '-inf')
      root?.setAttribute('data-cut-meter-peak', '-inf')
      root?.removeAttribute('data-cut-meter-clip')
    }
    if (!analyser || !active) {
      reset()
      return
    }
    const buf = new Float32Array(analyser.fftSize)
    let raf = 0
    let heldDb = -Infinity
    let heldUntil = 0
    const tick = () => {
      analyser.getFloatTimeDomainData(buf)
      const { rmsDb, peakDb, clip } = blockLevels(buf)
      const now = performance.now()
      // Peak-hold: jump up immediately, hold ~1.2 s, then let the new peak win.
      if (peakDb >= heldDb || now > heldUntil) {
        heldDb = peakDb
        heldUntil = now + 1200
      }
      if (barRef.current) barRef.current.style.height = `${dbToPct(rmsDb)}%`
      if (peakRef.current) peakRef.current.style.bottom = `${dbToPct(heldDb)}%`
      if (root) {
        root.setAttribute('data-cut-meter-db', isFinite(rmsDb) ? rmsDb.toFixed(1) : '-inf')
        root.setAttribute('data-cut-meter-peak', isFinite(heldDb) ? heldDb.toFixed(1) : '-inf')
        if (clip) root.setAttribute('data-cut-meter-clip', '')
        else root.removeAttribute('data-cut-meter-clip')
      }
      raf = requestAnimationFrame(tick)
    }
    raf = requestAnimationFrame(tick)
    return () => cancelAnimationFrame(raf)
  }, [analyser, active])

  return (
    <div
      ref={rootRef}
      className="pv-meter"
      data-cut-meter
      data-cut-meter-db="-inf"
      title="Master output level (dBFS) — the mixed timeline audio, matching the export"
      aria-label="Master output level meter"
    >
      <div ref={barRef} className="pv-meter-bar" />
      <div ref={peakRef} className="pv-meter-peak" />
    </div>
  )
}
