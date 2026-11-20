// panels/Preview/posterChip.ts — the status chip on the monitor while it shows an
// engine-rendered still frame instead of live video. Pure.
// The old chip read "no preview" whenever the base track had no playable video
// under the playhead, including the end of the timeline, a still image and a clip
// that sits on an overlay track, where a picture IS shown and nothing is wrong.
// Now the chip says what the frame is, or says nothing.
// Callers: Preview/index.tsx -> PreviewMonitorBadges. Deps: ./model, Timeline/layout.

import type { Project } from '../../lib/client'
import { baseVideoTrackId } from '../../lib/layerStack'
import { layoutTrack } from '../Timeline/layout'

export interface PosterChip {
  label: string
  /** Hover text: what it means and what the person can do. */
  title: string
  /** Quiet note (not work in progress): drawn without the pulsing accent. */
  note: boolean
  /** Which state, for the debug selector. */
  kind: 'proxy' | 'composed' | 'live-failed' | 'end' | 'gap'
}

export interface PosterChipInput {
  project: Project | null
  playheadMs: number
  durationMs: number
  proxyBuilding: boolean
  composed: boolean
  /** Base-track asset ids that cannot play live here. */
  failedSources: ReadonlySet<string>
  imageAssets: ReadonlySet<string>
  /** Overlay video layers drawn above the base at the playhead. */
  overlayCount: number
}

/** The base-track clip under the playhead, whether or not it can play. */
function baseClipAsset(project: Project, atMs: number): string | null {
  const track = project.tracks.find((t) => t.id === baseVideoTrackId(project.tracks))
  if (!track || track.visible === false) return null
  for (const item of layoutTrack(track)) {
    if (item.kind === 'video' && item.asset && atMs >= item.startMs && atMs < item.startMs + item.durMs) {
      return item.asset
    }
  }
  return null
}

/** The chip for a poster frame, or null when the frame needs no explanation. */
export function posterChip(input: PosterChipInput): PosterChip | null {
  if (input.proxyBuilding) {
    return { label: 'PROXY RENDERING…', title: 'Building a smooth preview of this clip.', note: false, kind: 'proxy' }
  }
  if (input.composed) {
    return { label: 'COMPOSED FRAME', title: 'The exact composed frame, with edits applied.', note: false, kind: 'composed' }
  }
  const { project } = input
  if (!project) return null
  const asset = baseClipAsset(project, input.playheadMs)
  if (asset) {
    if (input.imageAssets.has(asset)) return null // a still image: the picture is the clip
    if (input.failedSources.has(asset)) {
      return {
        label: 'STILL FRAME',
        title: 'This clip could not play live, so a still frame is shown. Pause on a frame to check the picture.',
        note: true,
        kind: 'live-failed',
      }
    }
    return null
  }
  // Nothing on the base track here. A clip on an overlay track is a picture.
  if (input.overlayCount > 0) return null
  if (input.durationMs > 0 && input.playheadMs >= input.durationMs) {
    return { label: 'END OF TIMELINE', title: 'The playhead is at the end; the last frame is shown.', note: true, kind: 'end' }
  }
  return {
    label: 'NO CLIP HERE',
    title: 'Nothing is on the video track at the playhead. Move the playhead onto a clip, or add one.',
    note: true,
    kind: 'gap',
  }
}
