import { msToPx, RAIL_W, RULER_H, shortDur } from './layout'

export interface AssetDropState {
  atMs: number
  trackId: string | null
}

interface TimelineOverlaysProps {
  assetDnd: AssetDropState | null
  dragRange: [number, number] | null
  exportRange: [number, number] | null
  zoom: number
  fullH: number
}

/** Non-interactive timeline overlays: asset drop line and export range band. */
export default function TimelineOverlays({
  assetDnd,
  dragRange,
  exportRange,
  zoom,
  fullH,
}: TimelineOverlaysProps) {
  const range = dragRange ?? exportRange
  return (
    <>
      {assetDnd && (
        <div
          className="tl-asset-drop"
          style={{ left: RAIL_W + msToPx(assetDnd.atMs, zoom), top: RULER_H }}
          data-cut-asset-drop={Math.round(assetDnd.atMs)}
        >
          <span className="tl-asset-drop__chip">{shortDur(assetDnd.atMs)}</span>
        </div>
      )}
      {range && (() => {
        const lo = msToPx(range[0], zoom)
        const hi = msToPx(range[1], zoom)
        return (
          <div
            className={`tl-range ${dragRange ? 'tl-range--live' : ''}`}
            style={{ left: RAIL_W + lo, width: Math.max(2, hi - lo), top: 0, height: Math.max(fullH, 1) }}
            data-cut-range={`${Math.round(range[0])},${Math.round(range[1])}`}
          >
            <span className="tl-range__flag tl-range__flag--in">{shortDur(range[0])}</span>
            <span className="tl-range__flag tl-range__flag--out">{shortDur(range[1])}</span>
            <span className="tl-range__dur">{shortDur(range[1] - range[0])}</span>
          </div>
        )
      })()}
    </>
  )
}
