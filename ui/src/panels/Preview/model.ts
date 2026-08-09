import { sourceUrl, type Project } from '../../lib/client'
import { baseVideoTrackId } from '../../lib/layerStack'
import { sourceMsAtTimelinePosition } from '../../lib/mediaTime'
import { layoutTrack } from '../Timeline/layout'

/**
 * A playable video under the playhead: the proxy if built, else the source.
 * startMs/srcInMs/durMs carry the active clip's timeline placement so the
 * free-run clock can map the <video> element's presented source time back to a
 * timeline position and detect the clip's out point to hand off to the next clip.
 */
export type ActiveVideo = {
  clipId: string
  src: string
  srcMs: number
  kind: 'proxy' | 'source'
  assetId: string
  startMs: number
  srcInMs: number
  srcOutMs: number
  durMs: number
  speed: number
  reverse: boolean
  trackId: string
}

type VideoFrameCallback = (now: number, meta: { mediaTime: number }) => void

export const RVFC_SUPPORTED =
  typeof HTMLVideoElement !== 'undefined' && 'requestVideoFrameCallback' in HTMLVideoElement.prototype

export function videoFrameCallbacks(el: HTMLVideoElement): {
  request: (cb: VideoFrameCallback) => number
  cancel: (handle: number) => void
} | null {
  const request = Reflect.get(el, 'requestVideoFrameCallback')
  const cancel = Reflect.get(el, 'cancelVideoFrameCallback')
  if (typeof request !== 'function' || typeof cancel !== 'function') return null
  return {
    request: request.bind(el),
    cancel: cancel.bind(el),
  }
}

/**
 * A timeline is half-open: its exact content extent has no frame. Playback
 * stops at that extent, though, and the paused monitor should keep showing the
 * final presented frame instead of replacing it with an all-black end frame.
 * Seeking beyond the extent remains untouched so real timeline gaps stay black.
 */
export function previewFrameMs(atMs: number, durationMs: number, frameMs: number): number {
  const at = Math.max(0, Math.round(atMs))
  const duration = Math.max(0, Math.round(durationMs))
  if (duration > 0 && at === duration) {
    return Math.max(0, Math.floor(duration - Math.max(1, frameMs)))
  }
  return at
}

/** The base visual asset occupying a timeline position, independent of whether
 * its source can currently be opened. */
export function activeBaseAssetId(project: Project | null, atMs: number): string | null {
  if (!project) return null
  const trackId = baseVideoTrackId(project.tracks)
  const track = project.tracks.find((candidate) => candidate.id === trackId)
  if (!track || track.visible === false) return null
  const item = layoutTrack(track).find((candidate) => (
    candidate.kind === 'video'
    && !!candidate.asset
    && atMs >= candidate.startMs
    && atMs < candidate.startMs + candidate.durMs
  ))
  return item?.asset ?? null
}

export function activeVideo(project: Project | null, atMs: number, failed: Set<string>): ActiveVideo | null {
  if (!project) return null
  const trackId = baseVideoTrackId(project.tracks)
  const track = project.tracks.find((candidate) => candidate.id === trackId)
  // A hidden or gapped base must stay a black canvas. Promoting a higher PiP
  // track to the <video> base would discard its transform and change z-order.
  if (!track || track.visible === false) return null
  for (const it of layoutTrack(track)) {
    if (it.kind !== 'video' || !it.asset) continue
    if (atMs >= it.startMs && atMs < it.startMs + it.durMs) {
      const asset = project.assets[it.asset]
      const srcInMs = it.srcInMs ?? 0
      const srcOutMs = it.srcOutMs ?? srcInMs + it.durMs
      const speed = it.speed && it.speed > 0 ? it.speed : 1
      const reverse = !!it.reverse
      const srcMs = sourceMsAtTimelinePosition({ startMs: it.startMs, srcInMs, srcOutMs, speed, reverse }, atMs)
      const placement = { clipId: it.id, startMs: it.startMs, srcInMs, srcOutMs, durMs: it.durMs, speed, reverse, trackId: track.id }
      if (asset?.proxy) return { src: asset.proxy, srcMs, kind: 'proxy', assetId: it.asset, ...placement }
      if (asset && !failed.has(it.asset)) return { src: sourceUrl(it.asset), srcMs, kind: 'source', assetId: it.asset, ...placement }
      return null
    }
  }
  return null
}
