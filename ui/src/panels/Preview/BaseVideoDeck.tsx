import { useCallback, useEffect, useLayoutEffect, useRef, type CSSProperties, type RefObject } from 'react'
import type { DeckSlot, DeckState, SlotIndex } from './baseDeckPlan'
import { releaseMediaElement } from './mediaElementRelease'
import type { DecodeReporter } from './usePreviewDecodeHealth'

/** A parked element whose seek or load has not finished after this long is
 * reloaded while it is still hidden (the WebKitGTK stuck-seek case), so it is
 * ready, or at least recovering, before the cut reaches it. */
const PARK_STUCK_MS = 1200

const HIDDEN: CSSProperties = { opacity: 0 }

/** The base-track video as two stacked elements (see baseDeckPlan.ts). The
 * front element carries the usual `data-cut-video` selector and the grade
 * filter; the standby is transparent and parked on the next clip's first
 * frame. `frontRef` always names the on-screen element (null in a gap) and is
 * set before the playback clock's effects run. */
export function BaseVideoDeck({
  deck,
  frontRef,
  frameMs,
  frontStyle,
  onSourceError,
  decode,
}: {
  deck: DeckState
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
  return (
    <>
      {([0, 1] as const).map((index: SlotIndex) => (
        <DeckVideo
          key={index}
          slot={deck.slots[index]}
          front={deck.front === index}
          frameMs={frameMs}
          style={deck.front === index ? frontStyle : HIDDEN}
          onElement={index === 0 ? setElement0 : setElement1}
          onSourceError={onSourceError}
          decode={decode}
        />
      ))}
    </>
  )
}

function DeckVideo({
  slot, front, frameMs, style, onElement, onSourceError, decode,
}: {
  slot: DeckSlot | null
  front: boolean
  frameMs: number
  style?: CSSProperties
  onElement: (element: HTMLVideoElement | null) => void
  onSourceError: (assetId: string) => void
  decode: DecodeReporter
}) {
  const ref = useRef<HTMLVideoElement | null>(null)
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
  const parkMs = !front && slot ? slot.parkMs : null
  const slotKey = slot?.key ?? null
  // Standby: stay paused on the first frame of the clip it waits for. A park
  // that does not settle (seek never completes, load fails) is retried with a
  // reload while the element is still hidden.
  useEffect(() => {
    const v = ref.current
    if (!v || parkMs === null) return
    const want = parkMs / 1000
    let unsettledSince = performance.now()
    const park = () => {
      if (!v.paused) v.pause()
      if (v.readyState >= 1 && !v.seeking && Math.abs(v.currentTime - want) > frameMs / 2000) v.currentTime = want
    }
    const watchdog = window.setInterval(() => {
      const settled = !v.error && !v.seeking && v.readyState >= 2 && Math.abs(v.currentTime - want) <= frameMs / 1000
      if (settled) { unsettledSince = performance.now(); return }
      if (performance.now() - unsettledSince < PARK_STUCK_MS) return
      unsettledSince = performance.now()
      v.load()
    }, 250)
    park()
    v.addEventListener('loadedmetadata', park)
    return () => {
      window.clearInterval(watchdog)
      v.removeEventListener('loadedmetadata', park)
    }
  }, [parkMs, slotKey, frameMs])
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
