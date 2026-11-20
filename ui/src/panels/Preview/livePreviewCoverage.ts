// What the LIVE preview (the playing proxy <video>s, CSS grade, caption text)
// cannot draw at a timeline position. The live stage plays the base clip and
// overlay clips with their position, size and opacity, approximates a clip's
// contrast / brightness / saturation / temperature with a CSS filter, and draws
// caption text. Everything else exists only in the engine's composed frame:
// titles and shapes unless their transparent overlay movie (qtrle .mov) really
// plays in this webview (measured: WebKitGTK plays it, with alpha; Chromium /
// WebView2 has no qtrle decoder), so a title counts as hidden until its element
// has shown a picture,
// fades, dissolves, effects, masks, mattes, crop, stabilization, Ken Burns and
// keyframed opacity, gamma / LUT / grade stacks / power windows / input color
// spaces, adjustment layers and track blend modes. While such an element is at
// the playhead the monitor says so instead of silently showing less. Pure.
import type { Project } from '../../lib/client'
import { layoutTrack } from '../Timeline/layout'

export type HiddenElement = 'titles' | 'fades' | 'transitions' | 'effects' | 'motion' | 'color' | 'adjustments'

/** Plain words for each kind, in the order they are listed. */
export const HIDDEN_ELEMENT_WORDS: Record<HiddenElement, string> = {
  titles: 'titles',
  fades: 'fades',
  transitions: 'transitions',
  effects: 'effects',
  motion: 'motion',
  color: 'color details',
  adjustments: 'adjustment layers',
}
const ORDER: HiddenElement[] = ['titles', 'fades', 'transitions', 'effects', 'motion', 'color', 'adjustments']

type RawClip = Record<string, unknown>
const nonEmpty = (value: unknown) => Array.isArray(value) && value.length > 0
const present = (value: unknown) => value !== undefined && value !== null && value !== false

function isTitleClip(project: Project, trackId: string, clip: RawClip): boolean {
  if (trackId.startsWith('title') || present(clip.title_text) || present(clip.shape_kind)) return true
  const assetId = typeof clip.asset === 'string' ? clip.asset : ''
  const path = project.assets[assetId]?.path ?? ''
  return /[\\/]titles[\\/][^\\/]+\.mov$/i.test(path)
}

/** Elements at `atMs` that only the composed frame shows, in display order. */
export function hiddenInLivePreview(
  project: Project | null, atMs: number, imageAssets?: Set<string>, liveDrawn?: ReadonlySet<string>,
): HiddenElement[] {
  if (!project) return []
  const found = new Set<HiddenElement>()
  for (const track of project.tracks) {
    if (track.kind !== 'video' || track.visible === false) continue
    const blend = (track as { blend_mode?: unknown }).blend_mode
    const clipsById = new Map<string, RawClip>()
    for (const clip of track.clips) {
      if ('id' in clip && 'asset' in clip) clipsById.set(clip.id, clip as unknown as RawClip)
    }
    for (const item of layoutTrack(track, imageAssets)) {
      if (item.kind !== 'video' || atMs < item.startMs || atMs >= item.startMs + item.durMs) continue
      const clip = clipsById.get(item.id)
      if (!clip) continue
      if (isTitleClip(project, track.id, clip) && !liveDrawn?.has(item.id)) found.add('titles')
      const fade = clip.fade as { in_ms?: number; out_ms?: number } | undefined
      if (fade && ((fade.in_ms ?? 0) > atMs - item.startMs || (fade.out_ms ?? 0) > item.startMs + item.durMs - atMs)) found.add('fades')
      const xfade = typeof clip.xfade_in_ms === 'number' ? clip.xfade_in_ms : 0
      if (xfade > atMs - item.startMs) found.add('transitions')
      if (nonEmpty(clip.effects) || present(clip.mask) || present(clip.matte) || present(clip.stabilize)
        || present(clip.crop) || (typeof blend === 'string' && blend !== 'normal')) found.add('effects')
      if (present(clip.animation) || nonEmpty(clip.keyframes)) found.add('motion')
      const grade = clip.grade as { gamma?: number; lut?: string | null } | null | undefined
      if (nonEmpty(clip.grade_stack) || nonEmpty(clip.grade_windows) || present(clip.input_color_space)
        || (grade && ((grade.gamma ?? 1) !== 1 || !!grade.lut))) found.add('color')
      // no break: inside a dissolve two clips of the track cover the position
    }
  }
  const adjustments = (project as { adjustments?: unknown[] }).adjustments ?? []
  for (const adjustment of adjustments) {
    const range = (adjustment as { range_ms?: unknown }).range_ms
    if (Array.isArray(range) && typeof range[0] === 'number' && typeof range[1] === 'number' && atMs >= range[0] && atMs < range[1]) {
      found.add('adjustments')
    }
  }
  return ORDER.filter((kind) => found.has(kind))
}

function listWords(kinds: HiddenElement[]): string {
  const words = kinds.map((kind) => HIDDEN_ELEMENT_WORDS[kind])
  return words.length <= 1 ? (words[0] ?? '') : `${words.slice(0, -1).join(', ')} and ${words[words.length - 1]}`
}

/** The notice on the picture while the live preview leaves elements out, or
 * null when nothing is left out. `composed`: the viewer shows the exact frame
 * when paused (the default); otherwise the person turned that off. */
export function livePreviewNotice(kinds: HiddenElement[], composed: boolean): string | null {
  if (kinds.length === 0) return null
  const list = listWords(kinds)
  return composed
    ? `Playback preview: ${list} appear when paused`
    : `${list.charAt(0).toUpperCase()}${list.slice(1)} are not shown. Turn COMPOSED on to see them`
}
