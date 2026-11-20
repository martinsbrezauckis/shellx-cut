// captureBackgrounds.ts — make `ui.screenshot` truthful about CSS background images.
//
// The capture rasterizes the DOM through html-to-image (DOM → SVG foreignObject →
// canvas). A web view may refuse to rasterize an image inside that SVG: WKWebView
// paints a black box for a very wide image (the 20480×72 px filmstrip strip of a
// two-minute clip), and any failed fetch leaves a transparent hole. The media-bin
// thumbnail and the timeline filmstrip are CSS backgrounds, so the agent got a
// picture without them and no word about it.
//
// After the DOM pass, every element with a `background-image: url(...)` is checked:
// its region of the capture is compared with the same layers drawn straight from
// the already-decoded image. A region that came out blank is drawn again from the
// image (size, position and repeat honoured; a two-colour vertical gradient on top
// of it too), and the result says how many were redrawn. An image that cannot be
// loaded at all is named in the notes. Nothing is silently dropped.
//
// The work is bounded, because the layers' URLs come from project data: only the
// engine's own origin and media routes are loaded, at most MAX_IMAGES distinct
// images, all within one TOTAL_BUDGET_MS for collection, loading and drawing
// (shorter than the engine's own wait for a screenshot). When the budget ends,
// loads still running are cancelled and a late answer is ignored. Unfinished
// repair is named in the notes.

import { sameOriginMediaPath } from './mediaUrl'

/** Most distinct images one capture will load for the repair. */
export const MAX_IMAGES = 24
/** Most elements with a background the repair looks at. */
export const MAX_ELEMENTS = 400
/** Collection, loading and drawing together; the engine waits 10 s for a screenshot. */
export const TOTAL_BUDGET_MS = 4000
/** Maximum canvas draws across sampling and repair, including repeated tiles. */
export const MAX_DRAW_OPERATIONS = 4096
/** One image, within what is left of the budget. */
export const IMAGE_TIMEOUT_MS = 2500
/** No new load starts with less than this left of the budget. */
const MIN_LOAD_MS = 100

export interface BackgroundLayer {
  /** Absolute URL of a url(...) layer, or null for a gradient/colour layer. */
  url: string | null
  /** The raw layer text (a gradient layer is replayed from it). */
  raw: string
  size: string
  position: string
  repeat: string
}

export interface BackgroundElement {
  x: number
  y: number
  w: number
  h: number
  layers: BackgroundLayer[]
}

/** Split a CSS comma list at the top level (not inside parentheses or quotes). */
export function splitLayers(value: string): string[] {
  const out: string[] = []
  let depth = 0
  let quote = ''
  let start = 0
  for (let i = 0; i < value.length; i++) {
    const c = value[i]
    if (quote) {
      if (c === quote) quote = ''
    } else if (c === '"' || c === "'") quote = c
    else if (c === '(') depth++
    else if (c === ')') depth--
    else if (c === ',' && depth === 0) {
      out.push(value.slice(start, i).trim())
      start = i + 1
    }
  }
  const last = value.slice(start).trim()
  if (last) out.push(last)
  return out
}

const URL_LAYER = /^url\(\s*(['"]?)(.*?)\1\s*\)$/

/** The url(...) target of one layer, resolved against the page, or null. */
export function layerUrl(layer: string): string | null {
  const m = URL_LAYER.exec(layer.trim())
  if (!m) return null
  try {
    return new URL(m[2], document.baseURI).href
  } catch {
    return null
  }
}

export type PartialReason = 'deadline' | 'elements' | 'draws'

export interface BackgroundCollection {
  elements: BackgroundElement[]
  partial?: PartialReason
}

/** Visible background elements under `root`, without materializing its descendants. */
export function collectBackgrounds(root: HTMLElement, deadlineAt = performance.now() + TOTAL_BUDGET_MS): BackgroundCollection {
  const found: BackgroundElement[] = []
  const walker = document.createTreeWalker(root, NodeFilter.SHOW_ELEMENT)
  while (true) {
    if (performance.now() >= deadlineAt) return { elements: found, partial: 'deadline' }
    if (found.length >= MAX_ELEMENTS) return { elements: found, partial: 'elements' }
    const el = walker.nextNode() as HTMLElement | null
    if (!el) break
    const cs = getComputedStyle(el)
    if (!cs.backgroundImage || !cs.backgroundImage.includes('url(')) continue
    const rect = el.getBoundingClientRect()
    if (rect.width <= 0 || rect.height <= 0) continue
    const images = splitLayers(cs.backgroundImage)
    const sizes = splitLayers(cs.backgroundSize)
    const positions = splitLayers(cs.backgroundPosition)
    const repeats = splitLayers(cs.backgroundRepeat)
    if (performance.now() >= deadlineAt) return { elements: found, partial: 'deadline' }
    const at = (list: string[], i: number, fallback: string) => list.length ? list[i % list.length] : fallback
    found.push({
      x: rect.x,
      y: rect.y,
      w: rect.width,
      h: rect.height,
      layers: images.map((raw, i) => ({
        url: layerUrl(raw),
        raw,
        size: at(sizes, i, 'auto'),
        position: at(positions, i, '0% 0%'),
        repeat: at(repeats, i, 'repeat'),
      })),
    })
  }
  return { elements: found, ...(performance.now() >= deadlineAt ? { partial: 'deadline' as const } : {}) }
}

/** One image load that can be cancelled: clearing `src` stops the request and
 *  the handlers are removed so a late result is never seen. */
interface PendingLoad {
  cancel(): void
  done: Promise<HTMLImageElement | null>
}

function startLoad(url: string, timeoutMs: number, makeImage: () => HTMLImageElement): PendingLoad {
  const img = makeImage()
  let settle!: (v: HTMLImageElement | null) => void
  const done = new Promise<HTMLImageElement | null>((resolve) => { settle = resolve })
  let finished = false
  const finish = (v: HTMLImageElement | null) => {
    if (finished) return
    finished = true
    clearTimeout(timer)
    img.onload = null
    img.onerror = null
    if (!v) img.src = ''
    settle(v)
  }
  const timer = setTimeout(() => finish(null), timeoutMs)
  img.onload = () => finish(img)
  img.onerror = () => finish(null)
  img.src = url
  return { cancel: () => finish(null), done }
}

interface RepairBudget {
  deadlineAt: number
  draws: number
  partial?: PartialReason
}

function timeUp(budget: RepairBudget): boolean {
  if (performance.now() < budget.deadlineAt) return false
  budget.partial = 'deadline'
  return true
}

function allowDraw(budget: RepairBudget): boolean {
  if (timeUp(budget)) return false
  if (budget.draws >= MAX_DRAW_OPERATIONS) {
    budget.partial = 'draws'
    return false
  }
  budget.draws++
  return true
}

type Len = { kind: 'auto' } | { kind: 'px'; v: number } | { kind: 'pct'; v: number }

function parseLen(token: string): Len {
  const t = token.trim().toLowerCase()
  if (t === 'auto' || t === '') return { kind: 'auto' }
  if (t.endsWith('%')) return { kind: 'pct', v: parseFloat(t) / 100 }
  return { kind: 'px', v: parseFloat(t) || 0 }
}

/** Painted size of a background image of natural size nw×nh in a box of w×h. */
export function layerSize(size: string, nw: number, nh: number, w: number, h: number): [number, number] {
  const s = size.trim().toLowerCase()
  if (s === 'cover' || s === 'contain') {
    const k = s === 'cover' ? Math.max(w / nw, h / nh) : Math.min(w / nw, h / nh)
    return [nw * k, nh * k]
  }
  const [a, b = 'auto'] = s.split(/\s+/)
  const lw = parseLen(a)
  const lh = parseLen(b)
  const px = (l: Len, whole: number): number | null => (l.kind === 'px' ? l.v : l.kind === 'pct' ? l.v * whole : null)
  let iw = px(lw, w)
  let ih = px(lh, h)
  if (iw === null && ih === null) return [nw, nh]
  if (iw === null) iw = (ih as number) * (nw / nh)
  if (ih === null) ih = iw * (nh / nw)
  return [iw, ih]
}

/** Offset of the painted image inside the box on one axis. */
export function layerOffset(token: string, boxLen: number, imageLen: number, axis: 'x' | 'y'): number {
  const t = token.trim().toLowerCase()
  const keyword: Record<string, number> = axis === 'x' ? { left: 0, center: 0.5, right: 1 } : { top: 0, center: 0.5, bottom: 1 }
  if (t in keyword) return (boxLen - imageLen) * keyword[t]
  const len = parseLen(t)
  if (len.kind === 'px') return len.v
  if (len.kind === 'pct') return (boxLen - imageLen) * len.v
  return 0
}

/** Which axes a background-repeat value tiles: [x, y]. */
export function repeatAxes(value: string): [boolean, boolean] {
  const t = value.trim().toLowerCase().split(/\s+/)
  if (t.length === 1) {
    if (t[0] === 'repeat') return [true, true]
    if (t[0] === 'repeat-x') return [true, false]
    if (t[0] === 'repeat-y') return [false, true]
    return [false, false]
  }
  return [t[0] === 'repeat', t[1] === 'repeat']
}

/** Draw one url layer of `el` into `ctx` (canvas coordinates already translated
 *  so the element's box starts at 0,0), clipped to the box. */
function drawImageLayer(ctx: CanvasRenderingContext2D, img: HTMLImageElement, layer: BackgroundLayer, w: number, h: number, budget: RepairBudget): boolean {
  const nw = img.naturalWidth
  const nh = img.naturalHeight
  if (!nw || !nh) return true
  const [iw, ih] = layerSize(layer.size, nw, nh, w, h)
  if (iw <= 0 || ih <= 0) return true
  const [px = '0%', py = '0%'] = layer.position.trim().split(/\s+/)
  const ox = layerOffset(px, w, iw, 'x')
  const oy = layerOffset(py, h, ih, 'y')
  const [repeatX, repeatY] = repeatAxes(layer.repeat)
  const xs: number[] = []
  const ys: number[] = []
  const first = (o: number, len: number, repeat: boolean) => (repeat ? o - Math.ceil(o / len) * len : o)
  const x0 = first(ox, iw, repeatX)
  const y0 = first(oy, ih, repeatY)
  for (let x = x0; x < w && xs.length < 400; x += iw) { if (timeUp(budget)) return false; xs.push(x); if (!repeatX) break }
  for (let y = y0; y < h && ys.length < 400; y += ih) { if (timeUp(budget)) return false; ys.push(y); if (!repeatY) break }
  for (const ty of ys) {
    for (const tx of xs) {
      if (timeUp(budget)) return false
      // Destination ∩ box, and the matching source rectangle (keeps a very wide
      // image's drawImage to the visible slice).
      const dx0 = Math.max(0, tx)
      const dy0 = Math.max(0, ty)
      const dx1 = Math.min(w, tx + iw)
      const dy1 = Math.min(h, ty + ih)
      if (dx1 <= dx0 || dy1 <= dy0) continue
      const sx = ((dx0 - tx) / iw) * nw
      const sy = ((dy0 - ty) / ih) * nh
      const sw = ((dx1 - dx0) / iw) * nw
      const sh = ((dy1 - dy0) / ih) * nh
      if (!allowDraw(budget)) return false
      ctx.drawImage(img, sx, sy, sw, sh, dx0, dy0, dx1 - dx0, dy1 - dy0)
    }
  }
  return true
}

const TWO_COLOUR_GRADIENT = /^linear-gradient\(\s*(rgba?\([^)]*\)|#[0-9a-f]{3,8})\s*,\s*(rgba?\([^)]*\)|#[0-9a-f]{3,8})\s*\)$/i

/** Replay a plain top-to-bottom two-colour gradient layer; false if it is anything else. */
function drawGradientLayer(ctx: CanvasRenderingContext2D, raw: string, w: number, h: number, budget: RepairBudget): boolean {
  const m = TWO_COLOUR_GRADIENT.exec(raw.trim())
  if (!m) return false
  if (!allowDraw(budget)) return false
  const g = ctx.createLinearGradient(0, 0, 0, h)
  g.addColorStop(0, m[1])
  g.addColorStop(1, m[2])
  ctx.fillStyle = g
  ctx.fillRect(0, 0, w, h)
  return true
}

/** Mean luminance (0..255) of a canvas region, sampled on a coarse grid. */
function meanLuma(ctx: CanvasRenderingContext2D, x: number, y: number, w: number, h: number, budget: RepairBudget): number | null {
  if (timeUp(budget)) return null
  const cw = Math.max(1, Math.min(32, Math.floor(w)))
  const ch = Math.max(1, Math.min(32, Math.floor(h)))
  const probe = document.createElement('canvas')
  probe.width = cw
  probe.height = ch
  const pctx = probe.getContext('2d')
  if (!pctx) return 0
  if (!allowDraw(budget)) return null
  pctx.drawImage(ctx.canvas, x, y, w, h, 0, 0, cw, ch)
  const data = pctx.getImageData(0, 0, cw, ch).data
  let sum = 0
  for (let i = 0; i < data.length; i += 4) {
    if (i % 256 === 0 && timeUp(budget)) return null
    sum += 0.2126 * data[i] + 0.7152 * data[i + 1] + 0.0722 * data[i + 2]
  }
  return sum / (data.length / 4)
}

export type SkipReason = 'foreign' | 'limit' | 'deadline'

export interface BackgroundReport {
  redrawn: string[]
  /** Could not be loaded (a failed or timed-out request). */
  missing: string[]
  /** Not even tried: outside the engine's media routes, over the count limit, or after the time budget. */
  skipped: { url: string; reason: SkipReason }[]
  /** Some visible backgrounds could not be checked within the repair limits. */
  partial?: PartialReason
}

export interface RepairOptions {
  budgetMs?: number
  maxImages?: number
  imageTimeoutMs?: number
  /** The page's own origin (the engine's); default `location.origin`. */
  pageOrigin?: string
  /** Image factory; default `new Image()`. */
  makeImage?: () => HTMLImageElement
  /** Absolute monotonic deadline shared with collection. */
  deadlineAt?: number
  /** Collection may have stopped before all descendants were visited. */
  collectionPartial?: PartialReason
}

/**
 * Check every collected background against the DOM capture in `ctx` and redraw the
 * ones that came out blank. `origin` is the root's top-left in the page, `scale`
 * the canvas pixels per CSS pixel. Returns what was redrawn, what could not be
 * loaded and what was not tried, for the capture's notes.
 */
export async function repairBackgrounds(
  ctx: CanvasRenderingContext2D,
  elements: BackgroundElement[],
  origin: { x: number; y: number },
  scale: number,
  options: RepairOptions = {},
): Promise<BackgroundReport> {
  const budgetMs = options.budgetMs ?? TOTAL_BUDGET_MS
  const maxImages = options.maxImages ?? MAX_IMAGES
  const imageTimeoutMs = options.imageTimeoutMs ?? IMAGE_TIMEOUT_MS
  const pageOrigin = options.pageOrigin ?? location.origin
  const makeImage = options.makeImage ?? (() => new Image())
  const report: BackgroundReport = { redrawn: [], missing: [], skipped: [] }
  if (options.collectionPartial) report.partial = options.collectionPartial
  const budget: RepairBudget = { deadlineAt: options.deadlineAt ?? performance.now() + budgetMs, draws: 0 }
  const images = new Map<string, HTMLImageElement | null>()
  const skip = (url: string, reason: SkipReason) => {
    if (!report.skipped.some((s) => s.url === url)) report.skipped.push({ url, reason })
  }
  let pending: PendingLoad | null = null
  let expired = false
  // At the end of the budget the running load is cancelled and nothing more starts.
  const deadline = setTimeout(() => {
    expired = true
    budget.partial = 'deadline'
    pending?.cancel()
  }, Math.max(0, budget.deadlineAt - performance.now()))
  try {
    for (const el of elements) {
      if (budget.partial === 'draws') break
      for (const layer of el.layers) {
        const url = layer.url
        if (!url || images.has(url) || report.skipped.some((s) => s.url === url)) continue
        const path = sameOriginMediaPath(url, pageOrigin)
        if (path === null) { skip(url, 'foreign'); continue }
        if (expired || timeUp(budget)) { skip(url, 'deadline'); continue }
        if (images.size >= maxImages) { skip(url, 'limit'); continue }
        const left = budget.deadlineAt - performance.now()
        // Too little left for a load to finish (and this check can run just before the deadline timer does).
        if (left < MIN_LOAD_MS) { expired = true; budget.partial = 'deadline'; skip(url, 'deadline'); continue }
        pending = startLoad(`${pageOrigin}${path}`, Math.max(1, Math.min(imageTimeoutMs, left)), makeImage)
        const img = await pending.done
        pending = null
        images.set(url, img)
        if (!img && (expired || timeUp(budget))) { images.delete(url); skip(url, 'deadline') }
      }
      if (expired || timeUp(budget)) continue
      const loaded = el.layers.filter((l) => l.url && images.get(l.url))
      for (const l of el.layers) {
        if (l.url && images.has(l.url) && !images.get(l.url) && !report.missing.includes(l.url)) report.missing.push(l.url)
      }
      if (loaded.length === 0) continue
      const dx = (el.x - origin.x) * scale
      const dy = (el.y - origin.y) * scale
      const dw = el.w * scale
      const dh = el.h * scale
      if (dw < 1 || dh < 1) continue
      // Expected picture: the image layers alone, drawn at the same size.
      const expected = document.createElement('canvas')
      expected.width = Math.max(1, Math.min(32, Math.round(dw)))
      expected.height = Math.max(1, Math.min(32, Math.round(dh)))
      const ectx = expected.getContext('2d')
      if (!ectx) continue
      ectx.scale(expected.width / el.w, expected.height / el.h)
      for (const l of [...loaded].reverse()) {
        if (!drawImageLayer(ectx, images.get(l.url!)!, l, el.w, el.h, budget)) break
      }
      if (budget.partial) break
      const wantLuma = meanLuma(ectx, 0, 0, expected.width, expected.height, budget)
      if (wantLuma === null) break
      // Too dark to tell a black box from a dark picture: leave it.
      if (wantLuma < 12) continue
      const gotLuma = meanLuma(ctx, dx, dy, dw, dh, budget)
      if (gotLuma === null) break
      // A dark overlay lowers the luminance to 54 % at most; far below that is blank.
      if (gotLuma >= wantLuma * 0.45) continue
      ctx.save()
      ctx.beginPath()
      ctx.rect(dx, dy, dw, dh)
      ctx.clip()
      ctx.translate(dx, dy)
      ctx.scale(scale, scale)
      // Layers are listed top first: paint bottom up.
      try {
        for (const l of [...el.layers].reverse()) {
          if (budget.partial || timeUp(budget)) break
          if (l.url) {
            const img = images.get(l.url)
            if (img && !drawImageLayer(ctx, img, l, el.w, el.h, budget)) break
          } else drawGradientLayer(ctx, l.raw, el.w, el.h, budget)
        }
      } finally {
        ctx.restore()
      }
      if (budget.partial) break
      for (const l of loaded) if (!report.redrawn.includes(l.url!)) report.redrawn.push(l.url!)
    }
  } finally {
    clearTimeout(deadline)
    pending?.cancel()
  }
  timeUp(budget)
  report.partial = budget.partial ?? report.partial
  return report
}

function shortUrl(url: string): string {
  try {
    const u = new URL(url)
    return `${u.pathname}${u.search ? '?…' : ''}`
  } catch {
    return url
  }
}

/** A refused URL for the notes: host and path, cut short, never loaded. */
function describeForeign(url: string): string {
  try {
    const u = new URL(url)
    return `${u.protocol}//${u.host}${u.pathname}`.slice(0, 80)
  } catch {
    return url.slice(0, 80)
  }
}

/** Plain sentences for the result's notes (empty when everything was embedded). */
export function backgroundNotes(report: BackgroundReport): string[] {
  const notes: string[] = []
  if (report.redrawn.length > 0) {
    notes.push(
      `${report.redrawn.length} CSS background image${report.redrawn.length === 1 ? '' : 's'} could not be rasterized by this web view's DOM capture and ${report.redrawn.length === 1 ? 'was' : 'were'} drawn from the loaded image instead: ${report.redrawn.slice(0, 3).map(shortUrl).join(', ')}`,
    )
  }
  if (report.missing.length > 0) {
    notes.push(
      `${report.missing.length} CSS background image${report.missing.length === 1 ? '' : 's'} could not be loaded and ${report.missing.length === 1 ? 'is' : 'are'} missing from this capture: ${report.missing.slice(0, 3).map(shortUrl).join(', ')}`,
    )
  }
  if (report.skipped.length > 0) {
    const count = (r: SkipReason) => report.skipped.filter((k) => k.reason === r).length
    const parts: string[] = []
    if (count('foreign') > 0) parts.push(`${count('foreign')} outside this app's own media routes (not loaded: ${report.skipped.filter((k) => k.reason === 'foreign').slice(0, 3).map((k) => describeForeign(k.url)).join(', ')})`)
    if (count('limit') > 0) parts.push(`${count('limit')} over the limit of ${MAX_IMAGES} images`)
    if (count('deadline') > 0) parts.push(`${count('deadline')} not reached within ${Math.round(TOTAL_BUDGET_MS / 1000)} s`)
    notes.push(`${report.skipped.length} CSS background image${report.skipped.length === 1 ? ' was' : 's were'} not checked or redrawn in this capture: ${parts.join('; ')}`)
  }
  if (report.partial) {
    const reason = report.partial === 'deadline'
      ? `its ${Math.round(TOTAL_BUDGET_MS / 1000)} s time budget`
      : report.partial === 'elements'
        ? `the ${MAX_ELEMENTS} background-element limit`
        : `the ${MAX_DRAW_OPERATIONS} canvas-draw limit`
    notes.push(`CSS background repair stopped at ${reason}; some backgrounds may be missing from this screenshot. Retry ui.screenshot with fewer media backgrounds visible.`)
  }
  return notes
}
