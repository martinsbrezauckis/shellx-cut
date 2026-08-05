import type { Asset } from '../../lib/client'
import type { LaidItem } from './layout'

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

/** The empty slot adjacent to a clip on its own track. */
export function adjacentGapSlot(it: LaidItem, items: LaidItem[]): { track: string; at_ms: number; duration_ms: number } | null {
  const endMs = it.startMs + it.durMs
  const after = items.find((g) => g.kind === 'gap' && g.trackId === it.trackId && g.startMs === endMs)
  if (after) return { track: it.trackId, at_ms: after.startMs, duration_ms: after.durMs }
  const before = items.find((g) => g.kind === 'gap' && g.trackId === it.trackId && g.startMs + g.durMs === it.startMs)
  if (before) return { track: it.trackId, at_ms: before.startMs, duration_ms: before.durMs }
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
