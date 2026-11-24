// mediaScope.ts — which project a media URL or a cached media result belongs to.
//
// Asset ids (`a1`), op ids and render ids repeat between projects, so a URL such
// as /api/frame?at_ms=0&v=op_0004 or /proxies/a1.mp4 names a DIFFERENT picture
// once another project is open. The browser (and the in-memory waveform and
// filmstrip caches) must never serve one project's answer for another's. Every
// URL and cache key built from an asset or op id therefore carries this scope.
//
// The scope is the project's identity hash plus the UI's project-session
// counter, so reopening the same project (or replacing it behind the same path)
// also starts with a clean slate. It is set once, by the component that owns the
// open project, before its children render.

import type { Project } from './clientModel'
import { safeMediaPath } from './mediaUrl'

let activeScope = ''
const scopeChangeListeners = new Set<(next: string, prev: string) => void>()

/** Subscribe to active project media-scope changes (e.g. cache eviction on close/switch). */
export function onMediaScopeChange(listener: (next: string, prev: string) => void): () => void {
  scopeChangeListeners.add(listener)
  return () => {
    scopeChangeListeners.delete(listener)
  }
}

/** Scope token for the open project; '' when none is open (no parameter is added). */
export function mediaScopeOf(project: Pick<Project, 'project_identity'> | null | undefined, session: number): string {
  const identity = project?.project_identity?.origin_path_sha256
  if (!identity) return ''
  // The engine's identity reads `sha256:<hex>`: keep 16 hex digits of the hash, not
  // the prefix plus a few.
  return `${session}-${identity.replace(/^sha256:/, '').slice(0, 16)}`
}

/** Called by the owner of the open project on every render (idempotent). */
export function setActiveMediaScope(scope: string): void {
  if (scope === activeScope) return
  const prev = activeScope
  activeScope = scope
  for (const listener of scopeChangeListeners) listener(scope, prev)
}

export function activeMediaScope(): string {
  return activeScope
}

/** Add the project scope to a media URL as `p=<scope>`; unchanged when no
 *  project is open. Works for URLs with and without a query string. A value that
 *  is not a path on one of the engine's media routes (see mediaUrl.ts) gives ''
 *  (no source) instead of a URL: the project data it came from is not trusted. */
export function scopeMediaUrl(url: string, scope: string = activeScope): string {
  const safe = safeMediaPath(url)
  if (safe === null) return ''
  if (!scope) return safe
  const hash = safe.indexOf('#')
  const base = hash >= 0 ? safe.slice(0, hash) : safe
  const tail = hash >= 0 ? safe.slice(hash) : ''
  return `${base}${base.includes('?') ? '&' : '?'}p=${encodeURIComponent(scope)}${tail}`
}

/** Prefix a cache key with the project scope. */
export function scopedCacheKey(key: string, scope: string = activeScope): string {
  return scope ? `${scope}|${key}` : key
}
