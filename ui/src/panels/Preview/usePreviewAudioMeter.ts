import { useCallback, useEffect, useRef, useState, type RefObject } from 'react'

declare global {
  interface Window {
    webkitAudioContext?: typeof AudioContext
  }
}

export function usePreviewAudioMeter(audioRef: RefObject<HTMLAudioElement | null>) {
  // --- master output meter (Audio Monitoring v2a) ---------------------------
  // Tap a Web Audio AnalyserNode off the SAME <audio> that plays the export mix,
  // so the meter reads the EXACT export level (WYSIWYG — no JS re-mix). A
  // MediaElementAudioSourceNode can be created only ONCE per element AND it
  // REROUTES the element's audio through the graph, so we build it lazily inside
  // the <audio>'s onPlay (a gesture-unlocked moment → the AudioContext can resume
  // and the element keeps making sound). If Web Audio is unavailable / capture is
  // blocked, we never capture the element, so v1 playback is unaffected (no meter).
  const meterCtxRef = useRef<AudioContext | null>(null)
  const meterSrcRef = useRef<MediaElementAudioSourceNode | null>(null)
  const meterMountedRef = useRef(true)
  const [meterAnalyser, setMeterAnalyser] = useState<AnalyserNode | null>(null)
  const setupMeter = useCallback(() => {
    if (!meterMountedRef.current) return
    if (meterCtxRef.current) {
      void meterCtxRef.current.resume().catch(() => {})
      return
    }
    const el = audioRef.current
    if (!el) return
    const Ctx = window.AudioContext || window.webkitAudioContext
    if (!Ctx) return
    let ctx: AudioContext | null = null
    let src: MediaElementAudioSourceNode | null = null
    try {
      ctx = new Ctx()
      src = ctx.createMediaElementSource(el)
      const an = ctx.createAnalyser()
      an.fftSize = 1024
      an.smoothingTimeConstant = 0.4
      src.connect(an)
      an.connect(ctx.destination)
      meterCtxRef.current = ctx
      meterSrcRef.current = src
      setMeterAnalyser(an)
      void ctx.resume().catch(() => {})
    } catch {
      if (meterCtxRef.current === ctx) meterCtxRef.current = null
      if (meterSrcRef.current === src) meterSrcRef.current = null
      try { src?.disconnect() } catch { /* partial graph already disconnected */ }
      if (ctx && ctx.state !== 'closed') void ctx.close().catch(() => {})
      // Meter setup is optional; release any partially constructed graph.
    }
  }, [])
  useEffect(() => {
    meterMountedRef.current = true
    return () => {
      meterMountedRef.current = false
      const ctx = meterCtxRef.current
      meterCtxRef.current = null
      const src = meterSrcRef.current
      meterSrcRef.current = null
      try { src?.disconnect() } catch { /* graph already disconnected */ }
      if (ctx && ctx.state !== 'closed') void ctx.close().catch(() => {})
    }
  }, [])
  return { meterAnalyser, setupMeter }
}
