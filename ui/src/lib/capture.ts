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

import { toCanvas } from 'html-to-image'

/** What a capture returns — png as raw base64 (no data: prefix) + caveats. */
export interface CaptureResult {
  png_base64: string
  /** Human/agent-readable caveats: composited videos, skipped elements. */
  notes: string[]
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

export async function captureApp(): Promise<CaptureResult> {
  const root = document.getElementById('root')
  if (!root) throw new Error('app root #root not found')
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
  const validImgs: HTMLImageElement[] = []
  for (const img of Array.from(root.querySelectorAll('img'))) {
    if (!img.getAttribute('src')) continue
    const valid = img.complete && img.naturalWidth > 0
    skipImgs.add(img) // always skip in the SVG path (we composite or note it)
    if (valid) validImgs.push(img)
    else notes.push(`poster image not ready/failed (${img.getAttribute('src')}) — region left blank`)
  }

  const canvas = await toCanvas(root, {
    pixelRatio: 1,
    backgroundColor: '#0a0a0a', // --bg; SVG rasterization defaults transparent
    // Belt: a failed image fetch falls back to a transparent pixel.
    imagePlaceholder: TRANSPARENT_PX,
    // Braces: <video> and EVERY <img> are excluded from the SVG path (their
    // live pixels are composited below); a broken/pending poster therefore
    // can't reject the whole capture.
    filter: (el) => !(el instanceof HTMLVideoElement) && !(el instanceof HTMLImageElement && skipImgs.has(el)),
  })

  const ctx = canvas.getContext('2d')
  if (!ctx) throw new Error('2d context unavailable on capture canvas')
  const rootRect = root.getBoundingClientRect()

  for (const video of Array.from(root.querySelectorAll('video'))) {
    const rect = video.getBoundingClientRect()
    // Invisible or zero-size videos contribute nothing — skip silently
    // (nothing was on screen to omit).
    if (rect.width <= 0 || rect.height <= 0) continue
    if (!(await awaitFrame(video))) {
      notes.push(`video ${video.currentSrc || '(no src)'} had no decoded frame — region left blank`)
      continue
    }
    const box = containBox(rect, video.videoWidth, video.videoHeight)
    try {
      // Same-origin proxy media served by cutd — not tainted, drawable.
      ctx.drawImage(video, box.x - rootRect.x, box.y - rootRect.y, box.w, box.h)
      notes.push(`composited live <video> frame at ${Math.round(box.w)}x${Math.round(box.h)} (DOM-capture cannot rasterize video)`)
    } catch (e) {
      notes.push(`video frame compositing failed: ${String(e)} — region left blank`)
    }
  }

  // Composite the VALID <img>s (e.g. a loaded Preview poster) from their own
  // pixels — they were excluded from the SVG path so a sibling broken poster
  // couldn't reject the capture; a valid one still appears. object-fit:contain
  // is honored the same way as video. Same-origin (cutd) → not tainted.
  for (const img of validImgs) {
    const rect = img.getBoundingClientRect()
    if (rect.width <= 0 || rect.height <= 0) continue
    const box = containBox(rect, img.naturalWidth, img.naturalHeight)
    try {
      ctx.drawImage(img, box.x - rootRect.x, box.y - rootRect.y, box.w, box.h)
    } catch (e) {
      notes.push(`poster compositing failed: ${String(e)} — region left blank`)
    }
  }

  const dataUrl = canvas.toDataURL('image/png')
  return { png_base64: dataUrl.slice('data:image/png;base64,'.length), notes }
}
