import type { MouseEvent, RefObject } from 'react'
import type { Marker } from '../../lib/client'
import { msToPx, RAIL_W, RULER_H } from './layout'

interface TimelineGuidesProps {
  markers: Marker[]
  zoom: number
  tracksH: number
  fullH: number
  snapLineMs: number | null
  playheadRef: RefObject<HTMLDivElement | null>
  onPlayheadMouseDown: (e: MouseEvent<HTMLDivElement>) => void
}

export default function TimelineGuides({
  markers,
  zoom,
  tracksH,
  fullH,
  snapLineMs,
  playheadRef,
  onPlayheadMouseDown,
}: TimelineGuidesProps) {
  return (
    <>
      {markers.map((m) => (
        <div
          key={m.id}
          className="tl-marker-line"
          style={{ left: RAIL_W + msToPx(m.at_ms, zoom), top: RULER_H, height: tracksH }}
        />
      ))}
      {snapLineMs !== null && (
        <div className="tl-snapline" style={{ left: RAIL_W + msToPx(snapLineMs, zoom), height: fullH }} />
      )}
      <div className="tl-playhead" ref={playheadRef} style={{ height: Math.max(fullH, 1) }} data-cut-playhead>
        <div
          className="tl-playhead-handle"
          onMouseDown={(e) => {
            e.stopPropagation()
            onPlayheadMouseDown(e)
          }}
        />
      </div>
    </>
  )
}
