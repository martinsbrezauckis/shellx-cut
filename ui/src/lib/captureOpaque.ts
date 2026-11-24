// Post-DOM media keeps its pixels unless paint order AND an opaque background
// are known. Hit areas, translucent overlays and uncertainty never hide media.
export interface PaintBox { x: number; y: number; w: number; h: number }
const MAX_CHECKS = 4096

function overlaps(a: PaintBox, b: DOMRect): boolean {
  return a.x < b.right && a.x + a.w > b.left && a.y < b.bottom && a.y + a.h > b.top
}

function opaqueColor(color: string): boolean {
  // Computed CSS colors use rgb/rgba or color functions. An omitted alpha is
  // opaque; explicit alpha must be exactly 1, never rounded from translucency.
  if (!/^(rgb|rgba|color|lab|lch|oklab|oklch)\(/.test(color)) return false
  const body = color.slice(color.indexOf('(') + 1, -1)
  const alpha = body.includes('/') ? body.split('/')[1].trim()
    : color.startsWith('rgba(') ? body.split(',')[3]?.trim() : undefined
  return alpha === undefined || alpha === '1' || alpha === '100%'
}

function trapsFixedPosition(cs: CSSStyleDeclaration): boolean {
  return cs.transform !== 'none'
    || cs.filter !== 'none'
    || cs.perspective !== 'none'
    || (cs as CSSStyleDeclaration & { backdropFilter?: string }).backdropFilter !== 'none' && Boolean((cs as CSSStyleDeclaration & { backdropFilter?: string }).backdropFilter)
    || /paint|layout|strict|content/.test(cs.contain)
    || /transform|filter|perspective|backdrop-filter/.test(cs.willChange)
}

/** Only axis-aligned, unmasked opaque backgrounds are certain. Ancestor
 * opacity/filter/blending affects the entire subtree, including solid children.
 * Overflow clipping applies only when the ancestor is in the element's
 * containing-block chain (not escaped via an intervening position:fixed) and
 * contains the complete candidate box. */
function opaqueBox(el: Element, rect: DOMRect, view: Window, deadlineAt: number): CSSStyleDeclaration | null {
  const style = view.getComputedStyle(el)
  if (!opaqueColor(style.backgroundColor) || style.backgroundClip !== 'border-box') return null
  let escapedToFixed = style.position === 'fixed'
  for (let ancestor: Element | null = el; ancestor; ancestor = ancestor.parentElement) {
    if (performance.now() >= deadlineAt) return null
    const cs = view.getComputedStyle(ancestor)
    if (cs.contentVisibility === 'hidden' || cs.visibility !== 'visible' || Number(cs.opacity) !== 1 || cs.filter !== 'none'
      || cs.clip !== 'auto' || cs.mixBlendMode !== 'normal' || cs.clipPath !== 'none' || cs.maskImage !== 'none') return null
    if (cs.transform !== 'none') {
      const m = new DOMMatrixReadOnly(cs.transform)
      if (!m.is2D || m.b !== 0 || m.c !== 0 || m.a !== 1 || m.d !== 1) return null
    }
    const trapsFixed = trapsFixedPosition(cs)
    if (ancestor !== el && (!escapedToFixed || trapsFixed) && (/(hidden|clip|scroll|auto)/.test(`${cs.overflowX} ${cs.overflowY}`) || /paint|strict|content/.test(cs.contain))) {
      const r = ancestor.getBoundingClientRect()
      const left = r.left + parseFloat(cs.borderLeftWidth), right = r.right - parseFloat(cs.borderRightWidth)
      const top = r.top + parseFloat(cs.borderTopWidth), bottom = r.bottom - parseFloat(cs.borderBottomWidth)
      if (rect.left < left || rect.right > right || rect.top < top || rect.bottom > bottom) return null
      // An overflow clip can also remove a child's rounded corner pixels.
      const corners = [cs.borderTopLeftRadius, cs.borderTopRightRadius, cs.borderBottomRightRadius, cs.borderBottomLeftRadius].map(radius)
      if (corners.some(corner => corner === null)) return null
      const [tl, tr, br, bl] = corners
      if ((rect.left < r.left + tl!.x && rect.top < r.top + tl!.y)
        || (rect.right > r.right - tr!.x && rect.top < r.top + tr!.y)
        || (rect.right > r.right - br!.x && rect.bottom > r.bottom - br!.y)
        || (rect.left < r.left + bl!.x && rect.bottom > r.bottom - bl!.y)) return null
    }
    if (ancestor !== el) {
      if (trapsFixed) escapedToFixed = false
      if (cs.position === 'fixed') escapedToFixed = true
    }
  }
  return style
}

/** Temporarily hit-test pointer-events:none or inert background media/panels
 * without changing paint order. Restore inline/inert state synchronously. */
function above(target: Element, cover: Element, x: number, y: number): boolean {
  const styles = [target, cover].map(el => (el as HTMLElement).style)
  if (styles.some(style => !style)) return false
  const saved = styles.map(style => [style.getPropertyValue('pointer-events'), style.getPropertyPriority('pointer-events')])
  const inertAncestors: HTMLElement[] = []
  for (const start of [target, cover]) {
    for (let cur: Element | null = start; cur; cur = cur.parentElement) {
      if (cur instanceof HTMLElement && cur.inert && !inertAncestors.includes(cur)) {
        inertAncestors.push(cur)
      }
    }
  }
  try {
    inertAncestors.forEach(el => { el.inert = false })
    styles.forEach(style => style.setProperty('pointer-events', 'auto', 'important'))
    const stack = target.ownerDocument.elementsFromPoint(x, y)
    const t = stack.indexOf(target), c = stack.indexOf(cover)
    return t >= 0 && c >= 0 && c < t
  } finally {
    styles.forEach((style, i) => {
      if (saved[i][0]) style.setProperty('pointer-events', saved[i][0], saved[i][1])
      else style.removeProperty('pointer-events')
    })
    inertAncestors.forEach(el => { el.inert = true })
  }
}

function radius(value: string): { x: number; y: number } | null {
  const parts = value.split(/\s+/)
  if (parts.some(part => !/^\d+(?:\.\d+)?px$/.test(part))) return null
  return { x: parseFloat(parts[0]), y: parseFloat(parts[1] ?? parts[0]) }
}

export class OpaqueMediaCovers {
  private checks = 0
  private covered = 0
  private limited = false
  constructor(private deadlineAt: number) {}

  /** Caller owns save/restore. The same viewport-to-canvas transform as the
   * media draw applies to these holes. No extra images, draws or time budget. */
  clip(ctx: CanvasRenderingContext2D, target: Element, box: PaintBox, origin: { x: number; y: number }, scale = 1): void {
    const doc = target.ownerDocument, view = doc.defaultView
    if (!view || !target.isConnected) return
    try {
      const walker = doc.createTreeWalker(doc.documentElement, NodeFilter.SHOW_ELEMENT)
      let cover: Element | null
      while ((cover = walker.nextNode() as Element | null)) {
        if (performance.now() >= this.deadlineAt || this.checks >= MAX_CHECKS) { this.limited = true; return }
        if (!(cover instanceof HTMLElement) || cover.contains(target) || cover.getClientRects().length !== 1) continue
        const r = cover.getBoundingClientRect()
        if (r.width <= 0 || r.height <= 0 || !overlaps(box, r)) continue
        const style = opaqueBox(cover, r, view, this.deadlineAt)
        if (!style) continue
        const radii = [style.borderTopLeftRadius, style.borderTopRightRadius, style.borderBottomRightRadius, style.borderBottomLeftRadius].map(radius)
        if (radii.some(r => r === null) || typeof ctx.roundRect !== 'function') continue
        const x1 = Math.max(0, box.x, r.left), x2 = Math.min(view.innerWidth, box.x + box.w, r.right)
        const y1 = Math.max(0, box.y, r.top), y2 = Math.min(view.innerHeight, box.y + box.h, r.bottom)
        if (x1 >= x2 || y1 >= y2) continue
        const x = (x1 + x2) / 2
        const y = (y1 + y2) / 2
        this.checks++
        if (performance.now() >= this.deadlineAt) { this.limited = true; return }
        if (!above(target, cover, x, y)) continue
        // Intersect with the inverse of each opaque rounded box separately:
        // overlapping covers remain excluded, never toggled back by even-odd.
        ctx.beginPath()
        ctx.rect(0, 0, ctx.canvas.width, ctx.canvas.height)
        ctx.roundRect((r.x - origin.x) * scale, (r.y - origin.y) * scale, r.width * scale, r.height * scale,
          radii.map(r => ({ x: r!.x * scale, y: r!.y * scale })))
        ctx.clip('evenodd')
        this.covered++
      }
    } catch {
      // Unavailable hit tests/CSS/geometry keep the unclassified media visible.
    }
  }

  notes(): string[] {
    const notes: string[] = []
    if (this.covered) notes.push('Media compositing preserved confirmed opaque backgrounds in front of media; translucent and uncertain layers keep media visible.')
    if (this.limited) notes.push('Opaque-cover checks reached the shared capture deadline or work cap; unchecked media stays visible.')
    return notes
  }
}
