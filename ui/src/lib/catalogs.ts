// lib/catalogs — drift-proof discovery catalogs (effects.list / transitions.list).
//
// The engine OWNS the canonical list of effects (edit.effect) and crossfade
// transition styles (edit.crossfade {transition}). Hardcoding those lists in the
// UI meant a new engine effect/transition stayed INVISIBLE to a human until the UI
// was hand-edited too. These helpers fetch the engine's `effects.list` /
// `transitions.list` catalogs ONCE (cached for the session) so the pickers always
// reflect exactly what the engine accepts — no source-level drift.
//
// Both verbs are pure READs (no project needed). On any failure the helpers resolve
// to an empty catalog so a caller can fall back to its curated inline set rather
// than render nothing. Callers: panels/Inspector (effects), panels/Timeline
// (transitions). Deps: lib/client (callVerb).

import { callVerb } from './client'

/** One effect from effects.list — the engine's effects-as-data catalog row. */
export interface EffectCatalogEntry {
  key: string
  track: 'video' | 'audio'
  description: string
  overlay_only: boolean
  params: { name: string; kind: 'number' | 'color'; min?: number; max?: number; default?: number; required: boolean }[]
}

/** One transition from transitions.list — an edit.crossfade {transition} style. */
export interface TransitionCatalogEntry {
  name: string
  category: string
  direction?: string | null
  description: string
}

// Session caches — the catalogs are static for a running engine, so fetch once and
// reuse the in-flight/resolved promise across every popover open.
let effectsPromise: Promise<EffectCatalogEntry[]> | null = null
let transitionsPromise: Promise<{ categories: string[]; transitions: TransitionCatalogEntry[] }> | null = null

/** Fetch (and cache) the full effects catalog. Empty array on failure. */
export function getEffectsCatalog(): Promise<EffectCatalogEntry[]> {
  if (!effectsPromise) {
    effectsPromise = callVerb('effects.list', {})
      .then((r) => {
        if (!r.ok) {
          effectsPromise = null
          return []
        }
        return (r.result as { effects?: EffectCatalogEntry[] })?.effects ?? []
      })
      .catch(() => {
        effectsPromise = null
        return []
      })
  }
  return effectsPromise
}

/** Fetch (and cache) the full transitions catalog + its category list. */
export function getTransitionsCatalog(): Promise<{ categories: string[]; transitions: TransitionCatalogEntry[] }> {
  if (!transitionsPromise) {
    transitionsPromise = callVerb('transitions.list', {})
      .then((r) => {
        if (!r.ok) {
          transitionsPromise = null
          return { categories: [], transitions: [] }
        }
        const res = r.result as { categories?: string[]; transitions?: TransitionCatalogEntry[] }
        return { categories: res?.categories ?? [], transitions: res?.transitions ?? [] }
      })
      .catch(() => {
        transitionsPromise = null
        return { categories: [], transitions: [] }
      })
  }
  return transitionsPromise
}
