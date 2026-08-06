import type { MouseEvent } from 'react'
import { msToPx, shortDur, type Seam } from './layout'

interface TimelineSeamHandlesProps {
  seams: Seam[]
  activeSeam: Seam | null
  zoom: number
  onSeamDown: (e: MouseEvent<HTMLDivElement>, seam: Seam) => void
}

export default function TimelineSeamHandles({
  seams,
  activeSeam,
  zoom,
  onSeamDown,
}: TimelineSeamHandlesProps) {
  return (
    <>
      {seams.map((seam) => {
        const active = activeSeam?.leftId === seam.leftId && activeSeam?.rightId === seam.rightId
        return (
          <div
            key={`seam:${seam.leftId}:${seam.rightId}`}
            className={`tl-seam${seam.xfadeMs > 0 ? ' tl-seam--xfade' : ''}${active ? ' tl-seam--active' : ''}`}
            // laidMs = the visible boundary in render space; atMs is EDITORIAL
            // (dispatch-only) and diverges from the drawn position after an
            // upstream crossfade — never position by it.
            style={{ left: msToPx(seam.laidMs, zoom) }}
            data-cut-action="seam"
            data-cut-seam={`${seam.leftId}:${seam.rightId}`}
            data-cut-seam-xfade={seam.xfadeMs || undefined}
            title={
              seam.xfadeMs > 0
                ? `crossfade ${shortDur(seam.xfadeMs)} — click to edit`
                : 'hard cut — click to add a crossfade'
            }
            onMouseDown={(e) => onSeamDown(e, seam)}
          />
        )
      })}
    </>
  )
}
