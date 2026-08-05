import { useEffect, useRef, type CSSProperties } from 'react'
import type { ClipTransform } from '../../lib/client'
import { sourceMsAtTimelinePosition } from '../../lib/mediaTime'
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
}: {
  layer: OverlayLayerData
  playheadMs: number
  rate: number
  override?: Required<ClipTransform> | null
}) {
  const ref = useRef<HTMLVideoElement>(null)
  useEffect(() => {
    const v = ref.current
    if (!v || layer.isImage) return
    const targetS = Math.max(0, sourceMsAtTimelinePosition(layer, playheadMs) / 1000)
    if (rate === 1 && !layer.reverse) {
      v.playbackRate = layer.speed
      if (Math.abs(v.currentTime - targetS) > 0.06) {
        try { v.currentTime = targetS } catch { /* not seekable yet */ }
      }
      if (v.paused) void v.play().catch(() => { /* autoplay race; retried next sync */ })
    } else {
      if (!v.paused) v.pause()
      if (Math.abs(v.currentTime - targetS) > 0.02) {
        try { v.currentTime = targetS } catch { /* not seekable yet */ }
      }
    }
  }, [playheadMs, rate, layer])

  const filter = gradeFilter(layer.grade)
  const style = { ...overlayBoxStyle(override ?? layer.transform), ...(filter ? { filter } : null) }
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
      ref={ref}
      className="pv-overlay"
      src={layer.src}
      muted
      playsInline
      style={style}
      data-cut-overlay={layer.trackId}
      data-cut-overlay-clip={layer.clipId}
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
