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

/**
 * localStorage key holding the user's chosen default export folder ("Choose
 * default export folder…" → `project.set_output_dir`). It lives here, not in
 * lib/exportDestination, because URL construction is now its first consumer:
 * a file the engine wrote into that folder is OUTSIDE the project and needs a
 * different URL shape (see exportUrl). exportDestination re-exports it, so its
 * own callers are unaffected.
 */
export const EXPORT_OUTPUT_DIR_STORAGE_KEY = 'cut.outputDir'

/** The stored default export folder, or null. Guarded: the pure-function tests
 *  run under node (no localStorage) and restricted webviews can throw on read. */
function storedOutputDir(): string | null {
  try {
    return typeof localStorage === 'undefined' ? null : localStorage.getItem(EXPORT_OUTPUT_DIR_STORAGE_KEY)
  } catch {
    return null
  }
}

/** Normalize a NATIVE host path for comparison: the engine returns Windows
 *  paths backslash-separated and sometimes with the extended-length prefix
 *  (`\\?\C:\…`). Only ever used for matching — the raw path is what we send. */
function normalizeHostPath(path: string): string {
  return path.replace(/\\/g, '/').replace(/^\/\/[?.]\//, '')
}

/** Is `norm` (already normalized) inside directory `dirNorm`? Compares whole
 *  path segments — `/Out-evil/x` is not inside `/Out`. Windows-shaped paths
 *  compare case-insensitively (the filesystem is), POSIX ones do not. */
function isInsideDir(norm: string, dirNorm: string): boolean {
  const dir = dirNorm.replace(/\/+$/, '')
  if (!dir) return false
  const windowsish = /^[A-Za-z]:\//.test(norm) || norm.startsWith('//')
  const a = windowsish ? norm.toLowerCase() : norm
  const b = windowsish ? dir.toLowerCase() : dir
  return a.length > b.length && a.startsWith(b) && a[b.length] === '/'
}

/** Map an export.* / render.* result `path` to a URL cutd can actually serve.
 *
 * TWO SHAPES, because exports do not all live in one place:
 *  - `/api/export/<rel>` — resolved against `<project>/exports`. Used for
 *    exports inside the project. Portable: it keeps working after the project
 *    folder is moved or copied, so old receipts still resolve.
 *  - `/api/export-file?path=<absolute>` — names ONE exact file. Used when the
 *    file is outside the project, which is where every export goes once the
 *    user picks a default export folder. The server fences this to the
 *    authorized export roots (project exports subtree + the chosen folder).
 *
 * WHY the shape matters (0.6.105/0.6.106 P1, fixed here): this helper used to
 * force everything into the relative shape. An export in the user's chosen
 * folder therefore became either a path that could not resolve inside the
 * project → 404 and dead in-app playback, or — when the chosen folder's own
 * path contained an `exports/` segment — a BARE NAME that resolved to a stale
 * same-named file inside the project, so the app played the WRONG export with
 * no error at all. Deciding the shape from the chosen output folder removes the
 * lossy conversion at the source; the server refuses the ambiguous leftovers.
 *
 * WINDOWS: the marker search runs on the normalized copy (separators are `\`
 * natively, so a literal `exports/` search would miss and leak the whole drive
 * path into the URL — the silent-preview-audio regression). The absolute shape
 * sends the RAW engine path, percent-encoded: the server resolves it with the
 * platform's own path rules, so `\\?\C:\…` must survive untouched.
 *
 * `outputDir` is a parameter (defaulted from localStorage) so the pure tests can
 * drive every branch without a DOM. */
export function exportUrl(path: string, outputDir: string | null = storedOutputDir()): string {
  const norm = normalizeHostPath(path)
  const exact = () => `${API_BASE}/api/export-file?path=${encodeURIComponent(path)}`
  // 1. In the user's chosen export folder → only the exact shape can name it.
  if (outputDir && isInsideDir(norm, normalizeHostPath(outputDir))) return exact()
  // 2. Under an `exports/` segment → the portable project-relative shape.
  const marker = 'exports/'
  const i = norm.lastIndexOf(marker)
  if (i >= 0) return `${API_BASE}/api/export/${encodeRel(norm.slice(i + marker.length))}`
  // 3. Absolute, but in neither → still name it exactly; the server decides
  //    whether it is inside an authorized root (a Save As target is not).
  if (norm.startsWith('/') || /^[A-Za-z]:\//.test(norm)) return exact()
  // 4. A relative path with no `exports/` segment: project-relative by
  //    definition — unchanged from the original behavior.
  return `${API_BASE}/api/export/${encodeRel(norm.replace(/^\/+/, ''))}`
}

/** Percent-encode a relative path per SEGMENT (the `/` separators stay). */
function encodeRel(rel: string): string {
  return rel.split('/').map(encodeURIComponent).join('/')
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
