import type { KeyboardEvent, MouseEvent } from 'react'
import type { Comment, Marker, Project } from '../../lib/client'
import { MARKER_COLOR_SWATCH } from '../../lib/clientModel'
import { resolveCommentTime } from '../../lib/commentAnchors'
import { markerClass, msToPx, RAIL_W, timecode, type RulerTick } from './layout'

interface TimelineRulerProps {
  innerW: number
  contentW: number
  zoom: number
  ticks: RulerTick[]
  markers: Marker[]
  comments: Comment[]
  project: Project | null
  markerGhost: { id: string; atMs: number } | null
  onRulerRangeDown: (e: MouseEvent<HTMLDivElement>) => void
  onMarkerDown: (e: MouseEvent<HTMLDivElement>, marker: Marker) => void
  onMarkerContextMenu: (e: MouseEvent<HTMLDivElement>, marker: Marker) => void
  onSeek: (atMs: number) => void
  onOpenComment: (id: string) => void
}

/** Sticky time ruler: ticks, marker triangles, review pins, and marker ghost. */
export default function TimelineRuler({
  innerW,
  contentW,
  zoom,
  ticks,
  markers,
  comments,
  project,
  markerGhost,
  onRulerRangeDown,
  onMarkerDown,
  onMarkerContextMenu,
  onSeek,
  onOpenComment,
}: TimelineRulerProps) {
  return (
    <div className="tl-ruler" style={{ width: innerW }} data-cut-action="ruler" data-cut-ruler title="Click to seek · drag to select a range for export" onMouseDown={onRulerRangeDown}>
      <div className="tl-ruler-corner" onMouseDown={(e) => e.stopPropagation()} />
      <div className="tl-ruler-content" style={{ left: RAIL_W, width: contentW }}>
        {ticks.map((t) => (
          <div key={t.ms} className={`tl-tick ${t.major ? 'tl-tick--major' : 'tl-tick--minor'}`} style={{ left: msToPx(t.ms, zoom) }}>
            {t.label && <span className="tl-tick-label">{t.label}</span>}
          </div>
        ))}
        {markers.map((m) => {
          const cls = markerClass(m)
          const dragging = markerGhost?.id === m.id
          const draggable = cls === 'plain'
          const openKeyboardMenu = (event: KeyboardEvent<HTMLDivElement>) => {
            if (event.key !== 'ContextMenu' && !(event.shiftKey && event.key === 'F10')) return
            event.preventDefault()
            const rect = event.currentTarget.getBoundingClientRect()
            event.currentTarget.dispatchEvent(new globalThis.MouseEvent('contextmenu', {
              bubbles: true,
              cancelable: true,
              clientX: rect.left + Math.min(8, rect.width / 2),
              clientY: rect.top + Math.min(8, rect.height / 2),
            }))
          }
          return (
            <div
              key={m.id}
              className={`tl-marker-tri tl-marker-tri--${cls}${dragging ? ' tl-marker-tri--dragging' : ''}${draggable ? '' : ' tl-marker-tri--fixed'}`}
              style={{
                left: msToPx(m.at_ms, zoom),
                // Marker color: the triangle is a CSS border trick, so the
                // fill IS border-top-color. Only user ('plain') markers carry
                // a color (core refuses coloring system markers).
                ...(m.color && cls === 'plain' ? { borderTopColor: MARKER_COLOR_SWATCH[m.color] } : {}),
              }}
              title={`${m.label}${m.note ? ` · ${m.note}` : ''}${draggable ? ' — drag to move' : ''}`}
              data-cut-marker={m.id}
              data-cut-marker-class={cls}
              data-cut-marker-color={m.color ?? 'default'}
              tabIndex={draggable ? 0 : undefined}
              role={draggable ? 'button' : undefined}
              aria-label={draggable ? `Marker ${m.label} menu` : undefined}
              onMouseDown={draggable ? (e) => onMarkerDown(e, m) : (e) => e.stopPropagation()}
              onContextMenu={draggable ? (e) => onMarkerContextMenu(e, m) : (e) => { e.preventDefault(); e.stopPropagation() }}
              onKeyDown={draggable ? openKeyboardMenu : undefined}
            />
          )
        })}
        {comments
          .filter((c) => c.status !== 'dismissed')
          .map((c) => ({ comment: c, time: resolveCommentTime(project, c) }))
          .map(({ comment: c, time }) => (
            <button
              key={c.id}
              className={`tl-comment-pin tl-comment-pin--${c.status} tl-comment-pin--anchor-${time.status}`}
              style={{ left: msToPx(time.atMs, zoom) }}
              title={`${timecode(time.atMs)} — ${c.text}${time.status === 'stale' ? ' · original clip missing' : ''}`}
              data-cut-comment-pin={c.id}
              data-cut-comment-anchor={time.status}
              aria-label={`review comment: ${c.text}`}
              onMouseDown={(e) => e.stopPropagation()}
              onClick={(e) => {
                e.stopPropagation()
                onSeek(time.atMs)
                onOpenComment(c.id)
              }}
            />
          ))}
        {markerGhost && (
          <div
            className="tl-marker-ghost"
            data-cut-marker-ghost={markerGhost.id}
            style={{ left: msToPx(markerGhost.atMs, zoom) }}
          />
        )}
      </div>
    </div>
  )
}
