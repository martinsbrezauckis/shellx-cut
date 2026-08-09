import type { Asset, Project } from '../../lib/client'
import { assetHasAudio } from '../../lib/placement'
import { isMediaContentClass, linkedSiblings, type LaidItem, type TimelineContentClass } from './layout'

// Direct-edit menu helpers (edit.replace / edit.fit_to_fill / edit.nest).
// The right-click menu's Replace/Fit source picker and Nest command share these
// pure helpers so menu curation and action paths use one source of truth.

/** Coarse media family of an asset from its probe.kind (media.probe stores it at
 * the probe root). Drives which assets are offered as a replace/fill source for a
 * given clip kind (a video slot takes video/image; an audio slot takes audio). */
export function assetMediaKind(a: Asset): 'video' | 'audio' | 'image' | 'other' {
  const k = (a.probe as { kind?: unknown } | undefined)?.kind
  return k === 'video' || k === 'audio' || k === 'image' ? k : 'other'
}

/** Filename tail of an asset path, for a readable picker label. */
export function assetBasename(a: Asset): string {
  return a.path.replace(/[\\/]+$/, '').split(/[\\/]/).filter(Boolean).pop() || a.path
}

/** The empty slot adjacent to a clip on its own track. Adjacency and the
 * returned at_ms are in EDITORIAL time — edit.fit_to_fill resolves its gap on
 * the engine's cumulative-track cursor (app/core/src/edit.rs fit_to_fill), and
 * laid positions rewind after an upstream crossfade. A gap's duration is the
 * same in both bases (gaps carry no crossfade). */
export function adjacentGapSlot(it: LaidItem, items: LaidItem[]): { track: string; at_ms: number; duration_ms: number } | null {
  const edEndMs = it.editorialStartMs + it.durMs
  const after = items.find((g) => g.kind === 'gap' && g.trackId === it.trackId && g.editorialStartMs === edEndMs)
  if (after) return { track: it.trackId, at_ms: after.editorialStartMs, duration_ms: after.durMs }
  const before = items.find((g) => g.kind === 'gap' && g.trackId === it.trackId && g.editorialStartMs + g.durMs === it.editorialStartMs)
  if (before) return { track: it.trackId, at_ms: before.editorialStartMs, duration_ms: before.durMs }
  return null
}

/** True when selected media clips occupy consecutive positions on one track. */
export function isContiguousRun(sel: LaidItem[], items: LaidItem[]): boolean {
  if (sel.length < 2) return false
  const track = sel[0].trackId
  if (!sel.every((i) => i.trackId === track)) return false
  const onTrack = items.filter((i) => i.trackId === track).sort((a, b) => a.startMs - b.startMs)
  const sorted = [...sel].sort((a, b) => a.startMs - b.startMs)
  const start = onTrack.findIndex((i) => i.id === sorted[0].id)
  if (start < 0 || start + sorted.length > onTrack.length) return false
  for (let k = 0; k < sorted.length; k++) if (onTrack[start + k]?.id !== sorted[k].id) return false
  return true
}

export type ContextMenuVisibility = 'visible' | 'hidden'

/**
 * One menu action's user-observable state. Class-invalid actions are hidden;
 * a valid action whose project/timeline precondition is absent stays visible
 * and disabled with this reason. Keeping that distinction in the model makes
 * the menu both honest and testable without duplicating selector conditions.
 */
export interface ContextMenuActionState {
  visibility: ContextMenuVisibility
  enabled: boolean
  reason: string
}

export interface ClipContextMenuContract {
  /** `none` means the timeline deliberately has no clip context menu (gap). */
  surface: 'media' | 'caption' | 'none'
  contentClass: TimelineContentClass
  isMedia: boolean
  isVisual: boolean
  isFootage: boolean
  /** Source replacement/fit/nest would sever a generated overlay's identity. */
  allowsSourceEdits: boolean
  /** Retiming/reverse are not valid for stills or generated overlays. */
  allowsSpeedEdits: boolean
  allowsFreeze: boolean
  allowsStabilize: boolean
  allowsPictureTools: boolean
  allowsColorAndCrop: boolean
  allowsPrivacyTools: boolean
  allowsSplitEdit: boolean
  hasTimelineAudio: boolean
  detachAudio: ContextMenuActionState
  addTransition: ContextMenuActionState & { atMs: number | null }
}

function hidden(reason: string): ContextMenuActionState {
  return { visibility: 'hidden', enabled: false, reason }
}

function disabled(reason: string): ContextMenuActionState {
  return { visibility: 'visible', enabled: false, reason }
}

function enabled(reason: string): ContextMenuActionState {
  return { visibility: 'visible', enabled: true, reason }
}

/**
 * Return the engine-valid editorial seam for a contextual crossfade.
 *
 * `startMs` is laid/render time and moves left when an upstream or existing
 * crossfade overlaps clips. `edit.crossfade` keys the cumulative track at its
 * editorial boundary instead. This is shared by the menu enable state and the
 * dispatch hook so a button cannot be disabled (or enabled) on different
 * semantics than the action it triggers.
 */
export function editorialTransitionAtMs(item: LaidItem, items: LaidItem[]): number | null {
  if (!isMediaContentClass(item.contentClass)) return null
  const onTrack = items.filter((candidate) =>
    candidate.id !== item.id &&
    candidate.trackId === item.trackId &&
    isMediaContentClass(candidate.contentClass),
  )
  const editorialEndMs = item.editorialStartMs + item.durMs
  if (onTrack.some((candidate) => candidate.editorialStartMs === editorialEndMs)) return editorialEndMs
  if (onTrack.some((candidate) => candidate.editorialStartMs + candidate.durMs === item.editorialStartMs)) {
    return item.editorialStartMs
  }
  return null
}

/**
 * Resolve the one audio clip a timeline action may safely address.
 *
 * A video item's asset/start pair is deliberately insufficient: independently
 * trimmed clips can retain both values while no longer describing the same
 * source window or laid slot. `linkedSiblings` owns that stricter identity
 * (asset, source window, and laid span), so both the menu and the dispatcher
 * consume it rather than maintaining their own near-match rules.
 */
export function exactTimelineAudioTarget(item: LaidItem, items: LaidItem[]): string | null {
  if (item.contentClass === 'audio') return item.id
  if (item.contentClass !== 'video') return null
  const audioSiblings = linkedSiblings(item, items).filter((candidate) => candidate.contentClass === 'audio')
  return audioSiblings.length === 1 ? audioSiblings[0].id : null
}

/**
 * Authoritative class/capability contract for the timeline clip context menu.
 *
 * It consumes `LaidItem.contentClass`, which is created from project clip
 * provenance in layout.ts. Keeping class and timing guards here prevents
 * local `trackId.startsWith(...)`, DOM, or laid-time shortcuts from leaking
 * back into individual menu rows.
 */
export function clipContextMenuContract(
  item: LaidItem,
  project: Project | null,
  allItems: LaidItem[],
): ClipContextMenuContract {
  const { contentClass } = item
  const isMedia = isMediaContentClass(contentClass)
  const isVisual = contentClass === 'video' || contentClass === 'still' ||
    contentClass === 'title' || contentClass === 'shape'
  const isFootage = contentClass === 'video'
  const generatedOverlay = contentClass === 'title' || contentClass === 'shape'
  const hasTimelineAudio = exactTimelineAudioTarget(item, allItems) !== null
  const transitionAtMs = editorialTransitionAtMs(item, allItems)
  const transition = !isMedia
    ? hidden('Transitions apply only to timeline media clips')
    : transitionAtMs === null
      ? disabled('Needs an adjacent media clip on the same track')
      : enabled('Adjacent media clips share an editorial cut')
  const detachAudio = contentClass !== 'video'
    ? hidden('Detach audio applies only to moving video footage')
    : !item.asset || !assetHasAudio(project, item.asset)
      ? hidden('This video asset has no audio stream to detach')
      : enabled('This video asset has an audio stream that can be extracted')

  return {
    surface: contentClass === 'gap' ? 'none' : contentClass === 'caption' ? 'caption' : 'media',
    contentClass,
    isMedia,
    isVisual,
    isFootage,
    allowsSourceEdits: isMedia && !generatedOverlay,
    allowsSpeedEdits: contentClass === 'video' || contentClass === 'audio',
    allowsFreeze: contentClass === 'video',
    allowsStabilize: contentClass === 'video',
    allowsPictureTools: isVisual,
    allowsColorAndCrop: contentClass === 'video' || contentClass === 'still',
    allowsPrivacyTools: contentClass === 'video' || contentClass === 'still',
    allowsSplitEdit: contentClass === 'video',
    hasTimelineAudio,
    detachAudio,
    addTransition: { ...transition, atMs: transitionAtMs },
  }
}
