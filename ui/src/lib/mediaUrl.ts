// mediaUrl.ts — the one gate for every media URL the UI builds from engine or
// project data (thumbnails, filmstrips, proxies, posters, waveforms, preview
// sources, review sheets, CSS backgrounds).
//
// A project, capture folder or cache made by someone else is untrusted input.
// A stored field such as `/attacker.example/x.png` becomes the protocol-relative
// URL `//attacker.example/x.png` once the UI puts a `/` in front of it, and the
// web view would then request another host. So a media URL is accepted only when
// it names one of the engine's own media routes on the engine's own origin;
// anything else yields nothing and the caller shows its normal "no picture" state.
//
// The accepted shape is an origin-relative path (`/proxies/a1.mp4?x=1`) or a
// project-relative field (`filmstrip/a1.jpg`, which gets its leading `/`). Absolute
// URLs of any kind are not accepted here; repairable CSS URLs are checked by
// `sameOriginMediaPath`, which first requires the page's own origin.

/** Routes the engine serves project and review media from. */
const MEDIA_ROUTES = [
  '/api/frame',
  '/api/source/',
  '/api/export/',
  '/api/export-file',
  '/api/library-blob/',
  '/api/library-poster',
  '/api/recording-rehearsal/',
  '/proxies/',
  '/frames/',
  '/filmstrip/',
]

const PLACEHOLDER_ORIGIN = 'http://cut.invalid'
const MAX_LENGTH = 4096

function isMediaRoute(pathname: string): boolean {
  return MEDIA_ROUTES.some((r) => (r.endsWith('/') ? pathname.startsWith(r) : pathname === r || pathname.startsWith(`${r}?`)))
}

/**
 * The origin-relative media path (`/filmstrip/a1.jpg?p=1`) for `raw`, or null when
 * `raw` is not a plain path on one of the engine's media routes. Rejects
 * protocol-relative (`//host`), absolute (`https:`, `javascript:`, `data:`, `blob:`),
 * backslash, control-character and percent-encoded slash, backslash or dot forms.
 */
export function safeMediaPath(raw: unknown): string | null {
  if (typeof raw !== 'string' || raw === '' || raw.length > MAX_LENGTH) return null
  // Control characters, spaces and backslashes are never part of an engine media URL
  // (browsers rewrite `\` to `/` and drop tabs and newlines, which hides a host).
  if (/[\u0000- \u007f-\u009f\\]/.test(raw)) return null
  // Project-relative fields carry no leading slash; a scheme before the first `/` is
  // an absolute URL, not a relative path.
  if (!raw.startsWith('/') && /^[^/?#]*:/.test(raw)) return null
  const path = raw.startsWith('/') ? raw : `/${raw}`
  if (path.startsWith('//')) return null
  let url: URL
  try {
    url = new URL(path, PLACEHOLDER_ORIGIN)
  } catch {
    return null
  }
  if (url.origin !== PLACEHOLDER_ORIGIN) return null
  // Decode the path once and look again: `%2f%2f`, `%5c` and `%2e%2e` must not hide
  // what the checks above look for.
  let decoded: string
  try {
    decoded = decodeURIComponent(url.pathname)
  } catch {
    return null
  }
  if (/[\u0000-\u001f\u007f\\]/.test(decoded) || decoded.startsWith('//')) return null
  if (decoded.split('/').some((s) => s === '..' || s === '.')) return null
  if (!isMediaRoute(url.pathname)) return null
  return `${url.pathname}${url.search}${url.hash}`
}

/** `safeMediaPath` for a value that goes straight into `src` or `url(...)`:
 *  the safe path, or undefined so the element simply has no source. */
export function engineMediaUrl(raw: unknown): string | undefined {
  return safeMediaPath(raw) ?? undefined
}

/**
 * Same as `safeMediaPath` for an absolute URL a computed style resolved: accepted
 * only when it points at `pageOrigin` (the engine's own origin). Returns the
 * origin-relative path to load, or null.
 */
export function sameOriginMediaPath(href: string, pageOrigin: string): string | null {
  let url: URL
  try {
    url = new URL(href)
  } catch {
    return null
  }
  if (url.origin !== pageOrigin || url.username || url.password) return null
  return safeMediaPath(`${url.pathname}${url.search}`)
}
