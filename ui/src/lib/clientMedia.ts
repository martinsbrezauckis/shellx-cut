// clientMedia.ts — display-only media fetch helpers for timeline preview UI.
//
// Role: cache and serve waveform/filmstrip helper calls without bloating the
// typed verb contract in client.ts. The public exports are still re-exported
// from client.ts for compatibility with existing panel imports.

import { callVerb } from './client'
import type { Waveform } from './clientModel'

// ---------------------------------------------------------------------------
// Waveform cache (timeline audio-clip overlay, zero-local-mutation contract display-only)
// ---------------------------------------------------------------------------

/** In-flight + resolved waveform fetches, keyed by `asset@buckets`. The value
 * is the PROMISE so concurrent clips of the same asset share ONE request, and
 * a settled promise is reused across re-renders / zoom changes (peaks are a
 * function of the source media, not of the view). A resolved `null` is cached
 * too — a successful empty result is never refetched. Verb-level failures are
 * dropped from the cache so transient cutd/import/probe issues can recover.
 * Module-level so it outlives panel remounts; the audio content of an asset
 * (addressed by id) does not change within a session, so no invalidation. */
const waveformCache = new Map<string, Promise<Waveform | null>>()

/**
 * Fetch (or reuse) the waveform peaks for an asset. Returns null when the
 * asset has no audio stream or the verb otherwise fails — the timeline simply
 * draws no waveform in that case (no crash, no console noise). `buckets` is
 * part of the cache key so a deeper request doesn't return a coarser cached
 * result; the timeline asks for one fixed resolution, so in practice one
 * entry per asset. Display-only: this reads peaks, it never mutates state.
 */
export function getWaveform(asset: string, buckets?: number): Promise<Waveform | null> {
  const key = `${asset}@${buckets ?? 'def'}`
  const hit = waveformCache.get(key)
  if (hit) return hit
  const p = callVerb('media.waveform', buckets != null ? { asset, buckets } : { asset })
    .then((r) => {
      if (!r.ok) {
        waveformCache.delete(key)
        return null
      }
      return r.result ?? null
    })
    // Transport failure (cutd down) is NOT a permanent "no audio" verdict —
    // drop the entry so a later draw retries instead of caching the disconnect.
    .catch(() => {
      waveformCache.delete(key)
      return null
    })
  waveformCache.set(key, p)
  return p
}

/* WINDOWED THUMBNAILS (per-zoom filmstrip) -------------------------------------
 * The whole-asset base strip is fixed-density, so zooming in just stretches it.
 * For a clip under the magnifier we instead fetch exactly the frames VISIBLE at
 * the current zoom — `count` frames sampled across a SOURCE sub-window. As you
 * zoom in the window shrinks, so density rises (→ per-frame at sub-second zoom),
 * yet cost stays bounded because only the visible window is ever sampled.
 *
 * The cache is keyed by the full request (asset + window + count + height); the
 * Timeline QUANTIZES windows/counts so pan/zoom jitter reuses entries. Module-
 * level so it survives panel remounts (an asset's pixels don't change in-session).
 * Returns the served tile URL, or null on failure (timeline falls back to the
 * base strip — no crash, no console noise). */
const windowThumbCache = new Map<string, Promise<string | null>>()

export interface WindowThumbs {
  url: string
  /** The SOURCE window the tile actually covers (server-clamped to [0,dur)). */
  startMs: number
  endMs: number
}

/**
 * Fetch (or reuse) a windowed thumbnail tile for an asset's source sub-range.
 * `t0`/`t1` are SOURCE ms; `count` frames are tiled across them at height `h`.
 * Returns {url,startMs,endMs} (the server may clamp the window) or null on
 * failure. Display-only — reads frames, never mutates project state.
 */
export function getWindowThumbs(
  asset: string,
  t0: number,
  t1: number,
  count: number,
  h: number,
): Promise<WindowThumbs | null> {
  const key = `${asset}@${t0}-${t1}#${count}x${h}`
  const hit = windowThumbCache.get(key)
  const wrap = (url: string | null): WindowThumbs | null => (url ? { url: `/${url}`, startMs: t0, endMs: t1 } : null)
  if (hit) return hit.then(wrap)
  const p = callVerb('media.filmstrip', { asset, range_ms: [t0, t1], count, h })
    .then((r) => (r.ok && r.result ? ((r.result as { thumbs?: string }).thumbs ?? null) : null))
    // Transport failure ≠ permanent "no thumbs": drop so a later request retries.
    .catch(() => {
      windowThumbCache.delete(key)
      return null
    })
  windowThumbCache.set(key, p)
  return p.then(wrap)
}
