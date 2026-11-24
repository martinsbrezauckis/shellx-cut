import { useCallback, useEffect, useLayoutEffect, useRef, type CSSProperties, type RefObject } from 'react'
import type { DeckSlot, DeckState, SlotIndex } from './baseDeckPlan'
import { releaseMediaElement } from './mediaElementRelease'
import type { DecodeReporter } from './usePreviewDecodeHealth'

/** A parked element whose seek, decode prime, or load has not finished after
 * this long is reloaded while still hidden (WebKitGTK stuck-seek recovery). */
const PARK_STUCK_MS = 1200
const PRIME_MAX_PLAY_MS = 80

const HIDDEN: CSSProperties = { opacity: 0, zIndex: 0 }
const FRONT_LAYER: CSSProperties = { zIndex: 2 }

export interface ParkedMediaOptions {
  video: HTMLVideoElement
  wantS: number
  frameMs: number
  delayFrames: number
  activeRef: RefObject<boolean>
}

/** Shared hidden park/prime controller used by BaseVideoDeck standby and
 * OverlayVideo preroll. Seeks reliably after metadata/in-flight seeks, pulses a
 * brief hidden decode frame once seeked so WebKitGTK/GStreamer commits its
 * pipeline position and texture before becoming visible, and recovers if
 * stuck. Cleanup never pauses an element that just became active/front. */
export function mountParkedMediaSync({ video: v, wantS, frameMs, delayFrames, activeRef }: ParkedMediaOptions): () => void {
  const stepS = Math.max(0.02, frameMs / 1000)
  const primeS = Math.max(0, wantS)
  const seekTolS = Math.max(frameMs / 2000, 0.015)
  const settleTolS = Math.max(stepS * 3, 0.12)
  let cancelled = false
  let priming = false
  let primed = false
  let primeOrigin = wantS
  let primeStartedAt = 0
  let unsettledSince = performance.now()
  let rafId = 0
  let watchdogId = 0

  const cancelRaf = () => {
    if (rafId) { window.cancelAnimationFrame(rafId); rafId = 0 }
  }
  const stopPrimeToPause = (markPrimed: boolean) => {
    priming = false
    if (markPrimed) primed = true
    cancelRaf()
    if (!activeRef.current && !v.paused) v.pause()
  }
  const recoverReload = () => {
    priming = false
    primed = false
    cancelRaf()
    unsettledSince = performance.now()
    if (!activeRef.current) v.load()
  }
  const tickPrime = () => {
    rafId = 0
    if (cancelled || activeRef.current) { priming = false; return }
    const now = performance.now()
    if (v.error) { recoverReload(); return }
    if (v.ended || (!v.seeking && (v.currentTime < primeS - 0.12 || v.currentTime > primeS + 0.35))) {
      // WebKitGTK/GStreamer sometimes resets a paused-seeked element to 0 or jumps
      // to EOF on play(); reloading cleanly recovers before the cut arrives.
      recoverReload()
      return
    }
    if (v.paused) {
      if (now - primeStartedAt >= PRIME_MAX_PLAY_MS * 2) { stopPrimeToPause(false); return }
      rafId = window.requestAnimationFrame(tickPrime)
      return
    }
    const elapsed = now - primeStartedAt
    const advanced = v.currentTime > primeOrigin + 0.002
    if (!v.seeking && v.readyState >= 2 && (advanced || elapsed >= PRIME_MAX_PLAY_MS)) {
      stopPrimeToPause(v.currentTime >= primeS - 0.05 && v.currentTime <= primeS + settleTolS)
      return
    }
    if (elapsed >= PRIME_MAX_PLAY_MS * 3) { stopPrimeToPause(false); return }
    rafId = window.requestAnimationFrame(tickPrime)
  }
  const startPrime = () => {
    if (cancelled || primed || priming || activeRef.current || v.readyState < 2 || v.seeking) return
    priming = true
    primeOrigin = v.currentTime
    primeStartedAt = performance.now()
    v.playbackRate = 1
    void v.play().catch(() => {
      if (!cancelled && !activeRef.current) priming = false
    })
    cancelRaf()
    rafId = window.requestAnimationFrame(tickPrime)
  }
  const ensurePark = () => {
    if (cancelled || activeRef.current || priming) return
    if (!v.paused) v.pause()
    if (v.readyState < 1 || v.seeking) return
    if (!primed && Math.abs(v.currentTime - primeS) > seekTolS) {
      try { v.currentTime = primeS } catch { /* not seekable yet */ }
      return
    }
    if (!primed && v.readyState >= 2 && !v.seeking) startPrime()
  }
  const begin = () => {
    if (cancelled || activeRef.current) return
    unsettledSince = performance.now()
    ensurePark()
    watchdogId = window.setInterval(() => {
      if (cancelled || activeRef.current) return
      if (!priming && !primed) ensurePark()
      const settled = !v.error && !v.seeking && !v.ended && v.readyState >= 2 && primed && Math.abs(v.currentTime - wantS) <= settleTolS
      if (settled) { unsettledSince = performance.now(); return }
      if (priming || performance.now() - unsettledSince < PARK_STUCK_MS) return
      unsettledSince = performance.now()
      primed = false
      v.load()
    }, 250)
  }

  if (!v.paused) v.pause()
  v.addEventListener('loadedmetadata', ensurePark)
  v.addEventListener('seeked', ensurePark)
  v.addEventListener('canplay', ensurePark)

  if (delayFrames > 0) {
    let left = delayFrames
    const waitFrame = () => {
      rafId = 0
      if (cancelled || activeRef.current) return
      left -= 1
      if (left > 0) rafId = window.requestAnimationFrame(waitFrame)
      else begin()
    }
    rafId = window.requestAnimationFrame(waitFrame)
  } else {
    begin()
  }

  return () => {
    cancelled = true
    cancelRaf()
    if (watchdogId) window.clearInterval(watchdogId)
    v.removeEventListener('loadedmetadata', ensurePark)
    v.removeEventListener('seeked', ensurePark)
    v.removeEventListener('canplay', ensurePark)
    if (!activeRef.current && !v.paused) v.pause()
  }
}

/** The base-track video as two stacked elements (see baseDeckPlan.ts). The
 * front element carries the usual `data-cut-video` selector, higher `zIndex`,
 * and the grade filter; the standby is transparent and parked/primed on the
 * next clip's first frame. `frontRef` always names the on-screen element
 * (null in a gap) and is set before the playback clock's effects run. */
export function BaseVideoDeck({
  deck,
  upcomingKey,
  frontRef,
  frameMs,
  frontStyle,
  onSourceError,
  decode,
}: {
  deck: DeckState
  upcomingKey?: string | null
  frontRef: RefObject<HTMLVideoElement | null>
  frameMs: number
  frontStyle?: CSSProperties
  /** A source (not proxy) file the webview cannot decode (error code 3 or 4). */
  onSourceError: (assetId: string) => void
  /** Proxy decode health (a system without the H.264 decoder). */
  decode: DecodeReporter
}) {
  const elements = useRef<[HTMLVideoElement | null, HTMLVideoElement | null]>([null, null])
  useLayoutEffect(() => {
    frontRef.current = deck.front === null ? null : elements.current[deck.front]
  })
  // The live stage went away (paused in a gap, poster mode): no element is on screen.
  useLayoutEffect(() => () => { frontRef.current = null }, [frontRef])
  const setElement0 = useCallback((element: HTMLVideoElement | null) => { elements.current[0] = element }, [])
  const setElement1 = useCallback((element: HTMLVideoElement | null) => { elements.current[1] = element }, [])
  const activeStyle = frontStyle ? { ...frontStyle, ...FRONT_LAYER } : FRONT_LAYER
  return (
    <>
      {([0, 1] as const).map((index: SlotIndex) => (
        <DeckVideo
          key={index}
          slot={deck.slots[index]}
          front={deck.front === index}
          upcomingKey={upcomingKey}
          frameMs={frameMs}
          style={deck.front === index ? activeStyle : HIDDEN}
          onElement={index === 0 ? setElement0 : setElement1}
          onSourceError={onSourceError}
          decode={decode}
        />
      ))}
    </>
  )
}

function DeckVideo({
  slot, front, upcomingKey, frameMs, style, onElement, onSourceError, decode,
}: {
  slot: DeckSlot | null
  front: boolean
  upcomingKey?: string | null
  frameMs: number
  style?: CSSProperties
  onElement: (element: HTMLVideoElement | null) => void
  onSourceError: (assetId: string) => void
  decode: DecodeReporter
}) {
  const ref = useRef<HTMLVideoElement | null>(null)
  const frontRef = useRef(front)
  const wasFrontRef = useRef(front)
  frontRef.current = front
  const setRef = useCallback((element: HTMLVideoElement | null) => {
    ref.current = element
    onElement(element)
    if (!element) return undefined
    // Removed with the live stage: release the media pipeline at once.
    return () => {
      if (ref.current === element) ref.current = null
      onElement(null)
      releaseMediaElement(element)
    }
  }, [onElement])
  const shouldPark = !front && !!slot && (upcomingKey === undefined || slot.key === upcomingKey)
  const parkMs = shouldPark ? slot.parkMs : null
  const slotKey = shouldPark ? slot.key : null
  useEffect(() => {
    const wasFront = wasFrontRef.current
    wasFrontRef.current = front
    const v = ref.current
    if (!v || front) return undefined
    if (parkMs === null) {
      if (!v.paused) v.pause()
      return undefined
    }
    // Wait two frames when leaving the screen so the compositor hides this
    // element before its seek updates the decoder surface.
    return mountParkedMediaSync({
      video: v,
      wantS: parkMs / 1000,
      frameMs,
      delayFrames: wasFront ? 2 : 0,
      activeRef: frontRef,
    })
  }, [front, parkMs, slotKey, frameMs])
  const video = slot?.video
  return (
    <video
      ref={setRef}
      className="pv-base"
      src={video?.src}
      muted
      playsInline
      preload="auto"
      aria-hidden={front ? undefined : true}
      data-cut-video={front || undefined}
      data-cut-video-kind={front ? video?.kind : undefined}
      data-cut-video-standby={front || !video ? undefined : video.clipId}
      style={style}
      onLoadedData={(e) => { if (video?.kind === 'proxy') decode.onDecoded(e.currentTarget, video.assetId) }}
      // A source the webview cannot decode (raw / unsupported codec:
      // MediaError DECODE or SRC_NOT_SUPPORTED) is not offered again; the
      // composed poster stands in until its proxy lands. Transient ABORTED or
      // NETWORK errors are ignored so a blip does not disable the preview.
      // A PROXY that cannot be decoded means the system lacks the decoder:
      // the monitor says so (usePreviewDecodeHealth).
      onError={(e) => {
        if (!video) return
        if (video.kind === 'proxy') { decode.onDecodeError(e.currentTarget, video.assetId); return }
        const code = e.currentTarget.error?.code
        if (code === 3 || code === 4) onSourceError(video.assetId)
      }}
    />
  )
}
