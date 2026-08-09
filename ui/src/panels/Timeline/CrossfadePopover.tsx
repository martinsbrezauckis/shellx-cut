import { useEffect, useMemo, useRef, useState } from 'react'
import { getTransitionsCatalog, type TransitionCatalogEntry } from '../../lib/catalogs'
import type { Seam } from './layout'

const XFADE_DEFAULT_MS = 500

/** Fallback edit.crossfade transition styles if transitions.list is unreachable -
 *  the popover is normally CATALOG-DRIVEN (getTransitionsCatalog), so a new engine
 *  transition shows up without a source edit. This keeps the picker non-empty when
 *  the engine read fails. 'dissolve' is the classic cross-dissolve default. */
const TRANSITION_STYLES_FALLBACK = [
  'dissolve', 'fade', 'fadeblack', 'fadewhite', 'fadegrays', 'fadefast', 'fadeslow',
  'wipeleft', 'wiperight', 'wipeup', 'wipedown', 'wipetl', 'wipetr', 'wipebl', 'wipebr',
  'slideleft', 'slideright', 'slideup', 'slidedown',
  'smoothleft', 'smoothright', 'smoothup', 'smoothdown',
  'coverleft', 'coverright', 'coverup', 'coverdown',
  'revealleft', 'revealright', 'revealup', 'revealdown',
  'circleopen', 'circleclose', 'circlecrop', 'rectcrop',
  'horzopen', 'horzclose', 'vertopen', 'vertclose',
  'diagtl', 'diagtr', 'diagbl', 'diagbr',
  'hlslice', 'hrslice', 'vuslice', 'vdslice',
  'radial', 'pixelize', 'hblur', 'distance', 'squeezeh', 'squeezev', 'zoomin',
  'hlwind', 'hrwind', 'vuwind', 'vdwind',
] as const

interface CrossfadePopoverProps {
  seam: Seam
  leftPx: number
  topPx: number
  onApply: (durationMs: number, transition: string) => void
  onClose: () => void
}

/** Crossfade duration editor for a selected seam. The parent owns the actual
 * edit.crossfade dispatch; this component owns the floating control surface. */
export default function CrossfadePopover({
  seam,
  leftPx,
  topPx,
  onApply,
  onClose,
}: CrossfadePopoverProps) {
  const [val, setVal] = useState<number>(seam.xfadeMs > 0 ? seam.xfadeMs : XFADE_DEFAULT_MS)
  const [style, setStyle] = useState<string>('dissolve')
  const [catalog, setCatalog] = useState<TransitionCatalogEntry[]>([])
  const inputRef = useRef<HTMLInputElement>(null)

  useEffect(() => {
    inputRef.current?.focus()
    inputRef.current?.select()
    let live = true
    void getTransitionsCatalog().then((c) => { if (live) setCatalog(c.transitions) })
    return () => { live = false }
  }, [])

  const grouped = useMemo(() => {
    const m = new Map<string, TransitionCatalogEntry[]>()
    for (const t of catalog) {
      const arr = m.get(t.category) ?? []
      arr.push(t)
      m.set(t.category, arr)
    }
    return m
  }, [catalog])

  return (
    <div
      className="tl-xfade-pop"
      data-cut-xfade-pop={`${seam.leftId}:${seam.rightId}`}
      style={{ left: leftPx, top: topPx }}
      onMouseDown={(e) => e.stopPropagation()}
    >
      <div className="tl-xfade-pop__title">
        crossfade <span className="tl-xfade-pop__clips">{seam.leftId} → {seam.rightId}</span>
      </div>
      <div className="tl-xfade-pop__row">
        <input
          ref={inputRef}
          className="tl-xfade-pop__input"
          data-cut-xfade-input
          type="number"
          min={0}
          step={50}
          value={val}
          onChange={(e) => setVal(Math.max(0, Math.round(Number(e.target.value) || 0)))}
          onKeyDown={(e) => {
            if (e.key === 'Enter') onApply(val, style)
            else if (e.key === 'Escape') onClose()
          }}
        />
        <span className="tl-xfade-pop__unit">ms</span>
      </div>
      <div className="tl-xfade-pop__row">
        <select
          className="tl-xfade-pop__style"
          data-cut-xfade-style
          data-cut-xfade-style-count={catalog.length || undefined}
          value={style}
          onChange={(e) => setStyle(e.target.value)}
          title="transition style"
        >
          {catalog.length > 0
            ? [...grouped.entries()].map(([cat, items]) => (
                <optgroup key={cat} label={cat}>
                  {items.map((t) => (
                    <option key={t.name} value={t.name} title={t.description}>{t.name}</option>
                  ))}
                </optgroup>
              ))
            : TRANSITION_STYLES_FALLBACK.map((s) => (
                <option key={s} value={s}>{s}</option>
              ))}
        </select>
      </div>
      <div className="tl-xfade-pop__note">timeline shortens by the overlap</div>
      <div className="tl-xfade-pop__actions">
        <button className="tl-xfade-pop__btn tl-xfade-pop__btn--primary" data-cut-action="apply-xfade" onClick={() => onApply(val, style)}>
          Apply
        </button>
        <button
          className="tl-xfade-pop__btn"
          data-cut-action="clear-xfade"
          disabled={seam.xfadeMs === 0}
          title={seam.xfadeMs === 0 ? 'no crossfade to clear' : 'remove the crossfade (hard cut)'}
          onClick={() => onApply(0, style)}
        >
          Clear
        </button>
      </div>
    </div>
  )
}
