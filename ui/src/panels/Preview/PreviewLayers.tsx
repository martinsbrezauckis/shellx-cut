import { useEffect, useRef, type CSSProperties } from 'react'
import type { ClipTransform } from '../../lib/client'
import { sourceMsAtTimelinePosition } from '../../lib/mediaTime'
import { mountParkedMediaSync } from './BaseVideoDeck'
import { canPlayPreviewVideo } from './model'
import { MEDIA_RESYNC_INTERVAL_MS, MEDIA_STALL_MS, overlayResyncDriftS, startsInPlace } from './playbackClock'
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
  override?: (ClipTransform & { opacity: number }) | null
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
  const shouldPlay = !preroll && canPlayPreviewVideo(rate, layer.reverse, layer.freezeAtMs)
  const activePlayRef = useRef(shouldPlay)
  activePlayRef.current = shouldPlay
  const targetRef = useRef(0)
  const syncStateRef = useRef({ lastSeekAt: -Infinity, seekingSince: Number.NaN, lastTime: Number.NaN, stalledSince: Number.NaN, srcKey: '' })

  useEffect(() => {
    const v = ref.current
    if (!v || layer.isImage || !preroll) return undefined
    const firstS = Math.max(0, sourceMsAtTimelinePosition(layer, layer.startMs) / 1000)
    return mountParkedMediaSync({
      video: v,
      wantS: firstS,
      frameMs: 1000 / 30,
      delayFrames: 0,
      activeRef: activePlayRef,
    })
  }, [preroll, layer.isImage, layer.clipId, layer.src, layer.startMs, layer.srcInMs, layer.speed, layer.reverse, layer.freezeAtMs])

  useEffect(() => {
    const v = ref.current
    if (!v || layer.isImage || preroll) return undefined
    const targetS = Math.max(0, sourceMsAtTimelinePosition(layer, playheadMs) / 1000)
    targetRef.current = targetS
    const st = syncStateRef.current
    const srcKey = `${layer.clipId}:${layer.src}`
    if (st.srcKey !== srcKey) {
      st.srcKey = srcKey
      st.lastSeekAt = -Infinity
      st.seekingSince = Number.NaN
      st.lastTime = Number.NaN
      st.stalledSince = Number.NaN
    }
    const sync = () => {
      const wantS = targetRef.current
      const now = performance.now()
      if (canPlayPreviewVideo(rate, layer.reverse, layer.freezeAtMs)) {
        v.playbackRate = layer.speed
        if (v.error) {
          if (now - st.lastSeekAt >= MEDIA_RESYNC_INTERVAL_MS) {
            st.lastSeekAt = now
            st.seekingSince = Number.NaN
            st.stalledSince = now
            v.load()
          }
          return
        }
        if (v.seeking) {
          if (!Number.isFinite(st.seekingSince)) st.seekingSince = now
          if (now - st.seekingSince >= MEDIA_STALL_MS && now - st.lastSeekAt >= MEDIA_RESYNC_INTERVAL_MS) {
            st.lastSeekAt = now
            st.seekingSince = Number.NaN
            st.stalledSince = now
            v.load()
          }
          return
        }
        st.seekingSince = Number.NaN
        if (v.readyState < 1) return
        const behindMs = (wantS - v.currentTime) * 1000
        const pausedInPlace = v.paused && v.readyState >= 2
          && ((behindMs >= -90 && behindMs <= 250 * Math.max(1, layer.speed))
            || startsInPlace(true, false, v.readyState, behindMs, layer.speed))
        const driftLimitMs = overlayResyncDriftS() * 1000 * Math.max(1, layer.speed)
        const needsInitialSeek = v.paused && !pausedInPlace && Math.abs(behindMs) > 60
        const needsDriftResync = !v.paused && Math.abs(behindMs) > driftLimitMs && now - st.lastSeekAt >= MEDIA_RESYNC_INTERVAL_MS
        if (needsInitialSeek || needsDriftResync) {
          st.lastSeekAt = now
          st.seekingSince = now
          st.stalledSince = now
          try { v.currentTime = wantS } catch { /* not seekable yet */ }
        } else if (!v.paused) {
          if (!Number.isFinite(st.lastTime) || Math.abs(v.currentTime - st.lastTime) > 0.001) {
            st.lastTime = v.currentTime
            st.stalledSince = now
          } else if (Math.abs(behindMs) > MEDIA_STALL_MS && now - st.stalledSince >= MEDIA_STALL_MS + 300 && now - st.lastSeekAt >= MEDIA_RESYNC_INTERVAL_MS) {
            st.lastSeekAt = now
            st.seekingSince = Number.NaN
            st.stalledSince = now
            v.load()
            return
          }
        }
        if (v.paused) void v.play().catch(() => { /* autoplay race; retried next sync */ })
      } else {
        if (!v.paused) v.pause()
        if (v.seeking) {
          if (!Number.isFinite(st.seekingSince)) st.seekingSince = now
        } else {
          st.seekingSince = Number.NaN
          if (v.readyState >= 1 && Math.abs(v.currentTime - wantS) > 0.02) {
            st.seekingSince = now
            try { v.currentTime = wantS } catch { /* not seekable yet */ }
          }
        }
      }
    }
    sync()
    v.addEventListener('loadedmetadata', sync)
    v.addEventListener('canplay', sync)
    v.addEventListener('seeked', sync)
    const pausedWatchdog = !canPlayPreviewVideo(rate, layer.reverse, layer.freezeAtMs)
      ? window.setInterval(() => {
        if (v.error || (v.seeking && Number.isFinite(st.seekingSince) && performance.now() - st.seekingSince >= 900)) {
          st.seekingSince = Number.NaN
          v.load()
          return
        }
        sync()
      }, 250)
      : 0
    return () => {
      if (pausedWatchdog) window.clearInterval(pausedWatchdog)
      v.removeEventListener('loadedmetadata', sync)
      v.removeEventListener('canplay', sync)
      v.removeEventListener('seeked', sync)
    }
  }, [playheadMs, rate, layer, preroll])

  const filter = gradeFilter(layer.grade)
  const style = { ...overlayBoxStyle(override ?? layer.transform, layer.sourceAspect), ...(filter ? { filter } : null), ...(preroll ? { opacity: 0 } : null) }
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
