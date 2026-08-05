// clientUrls.ts — URL helpers shared by the typed verb client and UI panels.
//
// Role: keep path/URL construction out of the already-large verb contract in
// client.ts. These helpers are pure and intentionally small because path bugs
// usually show up only on specific host/platform combinations.

/** Base URL — relative in prod (cutd serves us); vite proxies /api in dev. */
export const API_BASE = ''

/**
 * URL that streams a registered asset's original source for the
 * preview `<video>` when no proxy exists yet (edit instantly while the proxy
 * builds, or when proxy generation is toggled off). The server fences this to the
 * open project's asset registry and serves it seek-capable. A source whose codec
 * the browser can't decode (e.g. raw / ProRes) just fires `<video>` onError, and
 * the Preview falls back to the composed-frame poster.
 */
export function sourceUrl(assetId: string): string {
  return `${API_BASE}/api/source/${encodeURIComponent(assetId)}`
}

/** Map an export.* result `path` (absolute or project-relative, always under the
 * project's `exports/` subtree) to the served `/api/export/<rel>` URL — used to
 * play a rendered section (export.range) or the live audio monitor mix back in
 * the monitor. The server fences the route to the exports dir; we only translate
 * the path → URL here.
 *
 * WINDOWS: the engine returns NATIVE paths — on Windows that is backslash-
 * separated and may carry the extended-length prefix, e.g.
 * `\\?\C:\Users\…\screen.cutproj\exports\audio.mp3`. Searching for the literal
 * `exports/` then misses (separator is `\`), the whole drive path leaks into the
 * URL, the server 404s, and the preview `<audio>` stays SILENT. Normalize
 * separators and strip the `\\?\` / `\\.\` prefix first so the `exports/`
 * anchor matches on every platform. */
export function exportUrl(path: string): string {
  const norm = path.replace(/\\/g, '/').replace(/^\/\/[?.]\//, '')
  const marker = 'exports/'
  const i = norm.lastIndexOf(marker)
  const rel = i >= 0 ? norm.slice(i + marker.length) : norm.replace(/^\/+/, '')
  return `${API_BASE}/api/export/${rel.split('/').map(encodeURIComponent).join('/')}`
}

/**
 * Composed-frame URL for the <img> poster fallback (UI contract Preview panel).
 * `version` is an optional cache-bust token tied to the latest applied op id:
 * the composed frame at a fixed at_ms changes when the timeline changes, but
 * the URL would otherwise be identical and the browser would serve stale pixels.
 */
export function frameUrl(atMs: number, version?: string, compose?: boolean): string {
  let base = `${API_BASE}/api/frame?at_ms=${Math.max(0, Math.round(atMs))}`
  if (compose) base += '&compose=1'
  return version ? `${base}&v=${encodeURIComponent(version)}` : base
}
