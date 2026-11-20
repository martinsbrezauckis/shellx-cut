import { useEffect, useRef, type CSSProperties } from 'react'
import type { ClipTransform } from '../../lib/client'
import { sourceMsAtTimelinePosition } from '../../lib/mediaTime'
import { canPlayPreviewVideo } from './model'
import { overlayResyncDriftS, startsInPlace } from './playbackClock'
import { useReleasedMediaRef } from './mediaElementRelease'
import type { DecodeReporter } from './usePreviewDecodeHealth'
import {
  gradeFilter,
  overlayBoxStyle,
  type CaptionLayer,
  type OverlayLayer as OverlayLayerData,
} from './composite'

export function OverlayVideo({
  layer,
  playheadMs,
  rate,
  override,
  preroll = false,
  decode,
  onLiveDrawn,
}: {
  layer: OverlayLayerData
  playheadMs: number
  rate: number
  override?: Required<ClipTransform> | null
  /** The clip starts shortly: load it, park it paused on its first frame and
   * keep it invisible, so it only has to play when it appears. */
  preroll?: boolean
  /** Proxy decode health (a system without the H.264 decoder). */
  decode?: DecodeReporter
  /** Whether this webview really draws the clip (a picture loaded) or not
   * (error); a title movie in a format the webview cannot play is hidden. */
  onLiveDrawn?: (clipId: string, drawn: boolean) => void
}) {
  const ref = useRef<HTMLVideoElement>(null)
  const releasedRef = useReleasedMediaRef(ref)
  useEffect(() => {
    const v = ref.current
    if (!v || layer.isImage) return
    if (preroll) {
      const firstS = Math.max(0, sourceMsAtTimelinePosition(layer, layer.startMs) / 1000)
      const park = () => {
        if (!v.paused) v.pause()
        if (v.readyState >= 1 && !v.seeking && Math.abs(v.currentTime - firstS) > 0.02) {
          try { v.currentTime = firstS } catch { /* not seekable yet */ }
        }
      }
      park()
      v.addEventListener('loadedmetadata', park)
      return () => v.removeEventListener('loadedmetadata', park)
    }
    const targetS = Math.max(0, sourceMsAtTimelinePosition(layer, playheadMs) / 1000)
    if (canPlayPreviewVideo(rate, layer.reverse, layer.freezeAtMs)) {
      v.playbackRate = layer.speed
      // Same rule as the base clock: an element that stands at the target
      // starts or keeps playing without a seek; otherwise it is moved when the
      // drift exceeds what this webview's media timing warrants.
      const behindMs = (targetS - v.currentTime) * 1000
      const inPlace = startsInPlace(v.paused, v.seeking, v.readyState, behindMs, layer.speed)
      if ((!inPlace && Math.abs(behindMs) > 60) || Math.abs(behindMs) > overlayResyncDriftS() * 1000 * Math.max(1, layer.speed)) {
        try { v.currentTime = targetS } catch { /* not seekable yet */ }
      }
      if (v.paused) void v.play().catch(() => { /* autoplay race; retried next sync */ })
    } else {
      if (!v.paused) v.pause()
      if (Math.abs(v.currentTime - targetS) > 0.02) {
        try { v.currentTime = targetS } catch { /* not seekable yet */ }
      }
    }
    return undefined
  }, [playheadMs, rate, layer, preroll])

  const filter = gradeFilter(layer.grade)
  const style = { ...overlayBoxStyle(override ?? layer.transform), ...(filter ? { filter } : null), ...(preroll ? { opacity: 0 } : null) }
  if (layer.isImage) {
    return (
      <img
        className="pv-overlay"
        src={layer.src}
        alt=""
        style={style}
        data-cut-overlay={layer.trackId}
        data-cut-overlay-clip={layer.clipId}
      />
    )
  }
  return (
    <video
      ref={releasedRef}
      className="pv-overlay"
      src={layer.src}
      muted
      playsInline
      preload="auto"
      style={style}
      aria-hidden={preroll || undefined}
      data-cut-overlay={preroll ? undefined : layer.trackId}
      data-cut-overlay-clip={preroll ? undefined : layer.clipId}
      data-cut-overlay-preroll={preroll ? layer.clipId : undefined}
      onLoadedData={(e) => {
        if (layer.kind === 'proxy') decode?.onDecoded(e.currentTarget, layer.assetId)
        onLiveDrawn?.(layer.clipId, e.currentTarget.videoWidth > 0)
      }}
      onError={(e) => {
        if (layer.kind === 'proxy') decode?.onDecodeError(e.currentTarget, layer.assetId)
        onLiveDrawn?.(layer.clipId, false)
      }}
    />
  )
}

export function CaptionText({ cap, projectHeight }: { cap: CaptionLayer; projectHeight: number }) {
  const s = cap.style
  const pos = (s.pos as 'bottom' | 'top' | 'center' | undefined) ?? 'bottom'
  const sizeCqh = projectHeight > 0 ? (s.size / projectHeight) * 100 : 5
  const style: CSSProperties = {
    fontFamily: typeof s.font === 'string' ? s.font : undefined,
    fontSize: `${sizeCqh.toFixed(3)}cqh`,
    color: typeof s.color === 'string' ? s.color : '#fff',
    background: typeof s.bg === 'string' ? s.bg : undefined,
  }
  return (
    <div className={`pv-caption pv-caption--${pos}`} data-cut-caption={cap.id}>
      <span className="pv-caption-text" style={style}>{cap.text}</span>
    </div>
  )
}
