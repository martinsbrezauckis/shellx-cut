// capture.ts — in-app screenshot capture for the ui.screenshot verb
// (UI contract: "ui.screenshot is a verification PRIMITIVE, not a nice-to-have").
//
// Role: when the server relays a screenshot_request over WS (events.ts), this
// module captures the app root with html-to-image (MIT, DOM→SVG→canvas) and
// returns a base64 PNG. DOM-capture libs cannot rasterize <video> elements
// (they serialize the DOM into an SVG foreignObject — the live video frame is
// not part of the DOM text), so every visible <video> is composited on top
// from its live pixels via canvas drawImage, honoring object-fit: contain
// letterboxing. Anything composited (or failed) is reported in `notes` —
// UI contract: "never silently omit content".
//
// getDisplayMedia is explicitly NOT used (UI contract — breaks headless-adjacent
// flows). Callers: events.ts (screenshot_request handler). Deps: html-to-image.

import { toSvg } from 'html-to-image'
import { OpaqueMediaCovers } from './captureOpaque'
import { TOTAL_BUDGET_MS, backgroundNotes, collectBackgrounds, repairBackgrounds } from './captureBackgrounds'

function visibleOpacity(el: Element, root: Element, view: Window): number {
  let alpha = 1
  for (let cur: Element | null = el; cur; cur = cur === root ? null : cur.parentElement) {
    const cs = view.getComputedStyle(cur)
    if (cs.display === 'none' || cs.visibility !== 'visible' || cs.contentVisibility === 'hidden') return 0
    const op = Number(cs.opacity)
    if (!Number.isFinite(op) || op <= 0) return 0
    alpha *= Math.min(1, op)
  }
  return alpha
}

/** What a capture returns — png as raw base64 (no data: prefix) + caveats. */
export interface CaptureResult {
  png_base64: string
  /** Human/agent-readable caveats: composited videos, skipped elements. */
  notes: string[]
}

/** Pipeline stage a capture failure is attributed to (structured error
 *  contract — ui.screenshot is a verification PRIMITIVE, so its failures must
 *  be diagnosable, never "[object Event]"). */
export type CaptureStage = 'root-missing' | 'dom-rasterize' | 'canvas-context' | 'png-encode'

/** A capture failure that names its pipeline stage + a human-readable detail.
 *  Thrown by captureApp(); events.ts relays {code, stage, message} to the
 *  server so the ui.screenshot verb can fail actionably. A raw html-to-image
 *  rejection can stringify to "[object Event]",
 *  hiding both the stage and the failing resource). */
export class CaptureError extends Error {
  readonly stage: CaptureStage
  constructor(stage: CaptureStage, detail: string) {
    super(`capture failed at ${stage}: ${detail}`)
    this.name = 'CaptureError'
    this.stage = stage
  }
}

/**
 * Normalize an unknown thrown/rejected value into a human-readable detail.
 * html-to-image rejects with the raw resource-load EVENT (its onerror
 * argument), whose default String() is the useless "[object Event]" — for
 * events, extract the event type and the failing element (tag + src/href) so
 * the error names the culprit resource. Exported for unit tests.
 */
export function captureFailureDetail(err: unknown): string {
  if (err instanceof Error) return err.message || String(err)
  if (typeof Event !== 'undefined' && err instanceof Event) {
    const parts = [`${err.type || 'unknown'} event`]
    const target = err.target as
      | (Partial<Pick<HTMLElement, 'tagName'>> & { src?: unknown; currentSrc?: unknown; href?: unknown })
      | null
    if (target && typeof target === 'object') {
      if (typeof target.tagName === 'string') parts.push(`on <${target.tagName.toLowerCase()}>`)
      const src = [target.currentSrc, target.src, target.href].find(
        (v): v is string => typeof v === 'string' && v.length > 0,
      )
      if (src) parts.push(`(${src})`)
    }
    return parts.join(' ')
  }
  return String(err)
}

/** Structured {stage, message} for any capture rejection — CaptureError keeps
 *  its stage; anything else (a bug before staging) is reported honestly as
 *  stage "unknown". Exported for events.ts (the WS error frame) and tests. */
export function describeCaptureError(err: unknown): { stage: string; message: string } {
  if (err instanceof CaptureError) return { stage: err.stage, message: err.message }
  return { stage: 'unknown', message: captureFailureDetail(err) }
}

/**
 * Compute the painted content box of a video letterboxed by
 * `object-fit: contain` inside `rect` (the element's border box).
 * Returns the on-screen x/y/w/h of the actual video pixels.
 */
function containBox(
  rect: DOMRect,
  videoW: number,
  videoH: number,
): { x: number; y: number; w: number; h: number } {
  if (videoW <= 0 || videoH <= 0) return { x: rect.x, y: rect.y, w: rect.width, h: rect.height }
  const scale = Math.min(rect.width / videoW, rect.height / videoH)
  const w = videoW * scale
  const h = videoH * scale
  return { x: rect.x + (rect.width - w) / 2, y: rect.y + (rect.height - h) / 2, w, h }
}

/**
 * Wait (bounded) for a video to have a decodable current frame — a seek may
 * be in flight exactly when the screenshot lands. Resolves true when
 * readyState reaches HAVE_CURRENT_DATA, false on timeout.
 */
function awaitFrame(video: HTMLVideoElement, timeoutMs = 600): Promise<boolean> {
  if (video.readyState >= 2) return Promise.resolve(true)
  return new Promise((resolve) => {
    const done = (ok: boolean) => {
      video.removeEventListener('loadeddata', onReady)
      video.removeEventListener('seeked', onReady)
      clearTimeout(timer)
      resolve(ok)
    }
    const onReady = () => done(video.readyState >= 2)
    const timer = setTimeout(() => done(video.readyState >= 2), timeoutMs)
    video.addEventListener('loadeddata', onReady)
    video.addEventListener('seeked', onReady)
  })
}

/**
 * Capture the app root (#root) to a PNG.
 *
 * Pipeline: html-to-image toCanvas (pixelRatio 1 — agent eyes need truth,
 * not retina weight) → composite each visible, ready <video> from its live
 * frame → toDataURL → strip prefix. Throws only when the root capture itself
 * fails; per-video failures degrade to a note (content visibly missing is
 * better than no screenshot, and the note says exactly what is missing).
 */
/** 1×1 transparent PNG — the fallback an inlined <img> rasterizes to when its
 * real source can't be fetched. Without it, html-to-image REJECTS the whole
 * capture on a single failed image load ([object Event]) — one broken poster
 * would blind the agent entirely. The poster region is reported in notes. */
const TRANSPARENT_PX =
  'data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg=='

function loadTempImage(url: string): Promise<HTMLImageElement> {
  return new Promise((resolve, reject) => {
    const img = new Image()
    img.crossOrigin = 'anonymous'
    img.decoding = 'async'
    img.onload = () => {
      void img.decode().catch(() => {}).then(() => resolve(img))
    }
    img.onerror = (ev) => reject(ev)
    img.src = url
  })
}

async function rasterizeRootToCanvas(
  root: HTMLElement,
  skipImgs: Set<HTMLImageElement>,
): Promise<HTMLCanvasElement> {
  // Keep data:image/svg+xml URI so <foreignObject> stays same-origin (blob: SVG
  // URLs taint the destination canvas in Chromium), and release the temporary
  // Image element immediately after drawing onto the capture canvas.
  const svgDataUrl = await toSvg(root, {
    pixelRatio: 1,
    backgroundColor: '#0a0a0a', // --bg; SVG rasterization defaults transparent
    // html-to-image keeps every fetched resource in a module-level cache keyed
    // by URL WITHOUT its query string. Project-scoped media URLs differ only
    // by `?p=<project scope>` (asset ids such as a1 repeat between projects),
    // so without this the first project's thumbnails would be pasted into
    // every later capture, and the agent's screenshot would show the wrong
    // project's pictures while the live window is correct.
    includeQueryParams: true,
    // Belt: a failed image fetch falls back to a transparent pixel.
    imagePlaceholder: TRANSPARENT_PX,
    // Braces: <video>, <audio>, <source>, <track>, and EVERY <img> are
    // excluded from the SVG path (video/img live pixels are composited below;
    // cloning active audio/source nodes causes html-to-image/Blink to retain
    // detached MediaPlayer pipelines and full cloned DOM subtrees).
    filter: (el) =>
      !(el instanceof HTMLVideoElement)
      && !(el instanceof HTMLAudioElement)
      && !(el instanceof HTMLSourceElement)
      && !(el instanceof HTMLTrackElement)
      && !(el instanceof HTMLImageElement && skipImgs.has(el)),
  })
  let img: HTMLImageElement | null = null
  try {
    img = await loadTempImage(svgDataUrl)
    const rect = root.getBoundingClientRect()
    const w = Math.max(1, Math.round(img.naturalWidth || rect.width))
    const h = Math.max(1, Math.round(img.naturalHeight || rect.height))
    const canvas = document.createElement('canvas')
    canvas.width = w
    canvas.height = h
    const ctx = canvas.getContext('2d')
    if (ctx) {
      ctx.fillStyle = '#0a0a0a'
      ctx.fillRect(0, 0, w, h)
      ctx.drawImage(img, 0, 0, w, h)
    }
    return canvas
  } finally {
    if (img) {
      img.onload = null
      img.onerror = null
      img.src = ''
    }
  }
}

export async function captureApp(): Promise<CaptureResult> {
  const root = document.getElementById('root')
  if (!root) throw new CaptureError('root-missing', 'app root #root not found')
  const notes: string[] = []

  // An <img> that isn't fully loaded-and-valid makes html-to-image REJECT the
  // whole capture on its onerror ([object Event]) — the Preview poster requests
  // /api/frame, which returns 422 when no frame can be composed at the current
  // playhead (over a gap / past the end). ONE such poster would blind the agent
  // entirely. So EXCLUDE every <img> that is not complete-with-pixels from the
  // SVG serialization (filter) and instead composite the VALID ones from their
  // own pixels onto the canvas below — a valid frame still shows, a broken/
  // pending one goes blank + a note (UI contract: never silently omit, never blind).
  const skipImgs = new Set<HTMLImageElement>()
  const validImgs: Array<{ img: HTMLImageElement; opacity: number }> = []
  const view = root.ownerDocument.defaultView
  for (const img of Array.from(root.querySelectorAll('img'))) {
    if (!img.getAttribute('src')) continue
    skipImgs.add(img) // always skip in the SVG path (we composite or note it)
    const opacity = view ? visibleOpacity(img, root, view) : 1
    if (opacity <= 0) continue
    if (img.complete && img.naturalWidth > 0) validImgs.push({ img, opacity })
    else notes.push(`poster image not ready/failed (${img.getAttribute('src')}) — region left blank`)
  }

  const backgroundDeadlineAt = performance.now() + TOTAL_BUDGET_MS
  const backgrounds = collectBackgrounds(root, backgroundDeadlineAt)

  // Stage 'dom-rasterize': the html-to-image DOM→SVG→canvas pass. It rejects
  // with the raw resource-load Event on any inlining failure the belt/braces
  // above didn't cover (e.g. a CSS background-image or @font-face fetch) —
  // rethrow structured so the verb error names the stage + failing resource.
  let canvas: HTMLCanvasElement
  try {
    canvas = await rasterizeRootToCanvas(root, skipImgs)
  } catch (err) {
    throw new CaptureError('dom-rasterize', captureFailureDetail(err))
  }

  const ctx = canvas.getContext('2d')
  if (!ctx) throw new CaptureError('canvas-context', '2d context unavailable on capture canvas')
  const rootRect = root.getBoundingClientRect()
  const origin = { x: rootRect.x, y: rootRect.y }
  const covers = new OpaqueMediaCovers(backgroundDeadlineAt)

  // CSS background images (the media-bin thumbnail, the timeline filmstrip) go
  // through the same SVG pass, which some web views cannot rasterize: check each
  // one against the capture, redraw a blank one, and report what was redrawn or
  // missing (never a silent black box).
  try {
    const report = await repairBackgrounds(ctx, backgrounds.elements, { x: rootRect.x, y: rootRect.y }, canvas.width / Math.max(1, rootRect.width), {
      deadlineAt: backgroundDeadlineAt,
      collectionPartial: backgrounds.partial,
      covers,
    })
    notes.push(...backgroundNotes(report))
  } catch (e) {
    notes.push(`CSS background images could not be checked: ${String(e)}`)
  }

  for (const video of Array.from(root.querySelectorAll('video'))) {
    const rect = video.getBoundingClientRect()
    const opacity = view ? visibleOpacity(video, root, view) : 1
    // Invisible, hidden standby/preroll, or zero-size videos contribute
    // nothing — skip before awaiting frames so standby slots don't warn or overwrite.
    if (rect.width <= 0 || rect.height <= 0 || opacity <= 0) continue
    if (!(await awaitFrame(video))) {
      notes.push(`video ${video.currentSrc || '(no src)'} had no decoded frame — region left blank`)
      continue
    }
    const box = containBox(rect, video.videoWidth, video.videoHeight)
    ctx.save()
    try {
      covers.clip(ctx, video, box, origin)
      if (opacity < 1) ctx.globalAlpha *= opacity
      // Same-origin proxy media served by cutd — not tainted, drawable.
      ctx.drawImage(video, box.x - rootRect.x, box.y - rootRect.y, box.w, box.h)
      notes.push(`composited live <video> frame at ${Math.round(box.w)}x${Math.round(box.h)} (DOM-capture cannot rasterize video)`)
    } catch (e) {
      notes.push(`video frame compositing failed: ${String(e)} — region left blank`)
    } finally {
      ctx.restore()
    }
  }

  // Composite the visible VALID <img>s (e.g. a loaded Preview poster) from their own
  // pixels — they were excluded from the SVG path so a sibling broken poster
  // couldn't reject the capture; a valid one still appears. object-fit:contain
  // is honored the same way as video. Same-origin (cutd) → not tainted.
  for (const { img, opacity } of validImgs) {
    const rect = img.getBoundingClientRect()
    if (rect.width <= 0 || rect.height <= 0) continue
    const box = containBox(rect, img.naturalWidth, img.naturalHeight)
    ctx.save()
    try {
      covers.clip(ctx, img, box, origin)
      if (opacity < 1) ctx.globalAlpha *= opacity
      ctx.drawImage(img, box.x - rootRect.x, box.y - rootRect.y, box.w, box.h)
    } catch (e) {
      notes.push(`poster compositing failed: ${String(e)} — region left blank`)
    } finally {
      ctx.restore()
    }
  }

  notes.push(...covers.notes())

  // Stage 'png-encode': toDataURL throws SecurityError when the canvas was
  // tainted by a cross-origin draw (should never happen — media is same-origin
  // via cutd — but a structured error beats a mystery if it ever does).
  let dataUrl: string
  try {
    dataUrl = canvas.toDataURL('image/png')
  } catch (err) {
    throw new CaptureError('png-encode', captureFailureDetail(err))
  } finally {
    canvas.width = 0
    canvas.height = 0
  }
  return { png_base64: dataUrl.slice('data:image/png;base64,'.length), notes }
}
