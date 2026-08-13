import { memo, type KeyboardEvent, type MouseEvent } from 'react'
import { Icon } from '../../icons'
import { TRACK_HEIGHT, msToPx, shortDur, type LaidItem } from './layout'
import WaveformCanvas from './WaveformCanvas'

/** Height (px) of the slim waveform strip drawn at the bottom of a VIDEO clip. */
const WAVE_VIDEO_STRIP_H = 18

type ClipGestureMode = 'move' | 'trim-l' | 'trim-r'

interface ClipViewProps {
  item: LaidItem
  zoom: number
  selected: boolean
  dragging: boolean
  locked?: boolean
  displayName?: string
  offline?: boolean
  relinking?: boolean
  onClipDown: (e: MouseEvent, item: LaidItem, mode: ClipGestureMode) => void
  onRelinkAsset: (assetId: string) => Promise<boolean>
  /** Asset thumbnail strip (url + the asset's full duration) → "frames in the
   *  time bar". Present only for video clips whose asset has a built filmstrip. */
  filmstrip?: { url: string; assetDurMs: number }
  /** WINDOWED (zoom) thumbnails: a denser tile covering just the SOURCE window
   *  [startMs,endMs] of this clip that is currently visible — overlaid on top of
   *  the base strip when zoomed in, so each frame stays sharp instead of being
   *  stretched. Computed at the Timeline level (bounded by the viewport).
   *  Absent at overview zoom (the base strip suffices). */
  windowed?: { url: string; startMs: number; endMs: number }
}

const ClipView = memo(function ClipView({ item, zoom, selected, dragging, locked = false, displayName, offline = false, relinking = false, onClipDown, onRelinkAsset, filmstrip, windowed }: ClipViewProps) {
  const left = msToPx(item.startMs, zoom)
  const width = Math.max(2, msToPx(item.durMs, zoom))
  const label = displayName ?? item.label
  if (item.kind === 'gap') {
    // Gaps are content, not absence: hatched, hover shows duration.
    return (
      <div
        className="tl-gap"
        style={{ left, width }}
        title={`gap ${shortDur(item.durMs)}`}
        data-cut-gap={item.id}
      />
    )
  }
  const wide = width >= 60
  // PiP tooltip: normalized geometry as the agent would have set it.
  const pipTitle = item.transform
    ? `PiP ${Math.round(item.transform.scale * 100)}% @ (${item.transform.x.toFixed(2)}, ${item.transform.y.toFixed(2)})`
    : undefined
  const clipTitle = `${label} · ${shortDur(item.durMs)}${pipTitle ? ` · ${pipTitle}` : ''}`
  const openKeyboardMenu = (event: KeyboardEvent<HTMLDivElement>) => {
    if (event.key !== 'ContextMenu' && !(event.shiftKey && event.key === 'F10')) return
    event.preventDefault()
    const rect = event.currentTarget.getBoundingClientRect()
    event.currentTarget.dispatchEvent(new globalThis.MouseEvent('contextmenu', {
      bubbles: true,
      cancelable: true,
      clientX: rect.left + Math.min(24, rect.width / 2),
      clientY: rect.top + Math.min(18, rect.height / 2),
    }))
  }
  return (
    <div
      className={[
        'tl-clip',
        `tl-clip--${item.kind}`,
        item.isImage ? 'tl-clip--image' : '',
        item.motionLink ? 'tl-clip--motion' : '',
        selected ? 'tl-clip--selected' : '',
        dragging ? 'tl-clip--dragging' : '',
        locked ? 'tl-clip--locked' : '',
        offline ? 'tl-clip--offline' : '',
      ].join(' ')}
      style={{ left, width }}
      title={clipTitle}
      data-cut-action="clip"
      data-cut-clip={item.id}
      data-cut-locked={locked || undefined}
      data-cut-offline-asset={offline ? item.asset : undefined}
      tabIndex={0}
      role="group"
      aria-label={`${label} clip context menu`}
      aria-keyshortcuts="Shift+F10 ContextMenu"
      onMouseDown={(e) => onClipDown(e, item, 'move')}
      onKeyDown={openKeyboardMenu}
    >
      {/* Thumbnail filmstrip ("frames in the time bar"): the asset's strip,
          sliced to THIS clip's [src_in, src_out] and stretched to the clip width.
          The strip covers [0, assetDur] linearly, so source ms → strip x is the
          fraction t/assetDur. A dark gradient layers over it for label legibility.
          Video clips only; pointer-events none so clip gestures are unaffected. */}
      {!offline && item.kind === 'video' && filmstrip && item.srcInMs !== undefined && item.srcOutMs !== undefined && (() => {
        if (item.isImage) {
          // Still image: tile the single thumbnail across the clip width (no
          // time-slicing) so the picture is visible end-to-end.
          return (
            <div
              className="tl-clip-film"
              data-cut-clip-film={item.id}
              style={{
                backgroundImage: `linear-gradient(rgba(0,0,0,0.28), rgba(0,0,0,0.46)), url(${filmstrip.url})`,
                backgroundSize: `100% 100%, auto 100%`,
                backgroundPosition: `0 0, left center`,
                backgroundRepeat: `no-repeat, repeat-x`,
              }}
            />
          )
        }
        const span = Math.max(1, item.srcOutMs - item.srcInMs)
        const scaledW = width * (filmstrip.assetDurMs / span)
        const posX = -width * (item.srcInMs / span)
        return (
          <div
            className="tl-clip-film"
            data-cut-clip-film={item.id}
            style={{
              backgroundImage: `linear-gradient(rgba(0,0,0,0.28), rgba(0,0,0,0.46)), url(${filmstrip.url})`,
              backgroundSize: `100% 100%, ${scaledW}px 100%`,
              backgroundPosition: `0 0, ${posX}px 0`,
              backgroundRepeat: 'no-repeat, no-repeat',
            }}
          />
        )
      })()}
      {/* WINDOWED (zoom) overlay: a denser tile for the visible SOURCE sub-range,
          laid OVER the base strip so zoomed-in frames stay sharp instead of
          stretching. The tile already IS exactly [windowed.startMs, endMs], so we
          just position it over the matching sub-rect of the clip (source span →
          full width) and stretch it 100%×100%. Off when at overview zoom. */}
      {!offline && item.kind === 'video' && !item.isImage && windowed && item.srcInMs !== undefined && item.srcOutMs !== undefined && (() => {
        const span = Math.max(1, item.srcOutMs - item.srcInMs)
        const leftPx = width * ((windowed.startMs - item.srcInMs) / span)
        const wPx = width * ((windowed.endMs - windowed.startMs) / span)
        if (wPx < 1) return null
        return (
          <div
            className="tl-clip-film tl-clip-film--zoom"
            data-cut-clip-zoom={item.id}
            style={{
              left: leftPx,
              width: wPx,
              backgroundImage: `linear-gradient(rgba(0,0,0,0.28), rgba(0,0,0,0.46)), url(${windowed.url})`,
              backgroundSize: `100% 100%, 100% 100%`,
              backgroundPosition: `0 0, 0 0`,
              backgroundRepeat: 'no-repeat, no-repeat',
            }}
          />
        )
      })()}
      {/* Audio waveform overlay (workspace contract v1): a centered peak trace behind the
          clip label, drawn from media.waveform peaks sliced to this clip's
          source range. Audio clips only (video/caption clips have no audio
          stream to draw). Loads async + cached per asset; renders nothing until
          peaks arrive (the flat clip body is the loading state) and nothing for
          an asset with no audio (getWaveform resolves null). Display-only,
          pointer-events none — the clip's own gestures are unaffected. */}
      {!offline && item.kind === 'audio' && item.asset && item.srcInMs !== undefined && item.srcOutMs !== undefined && (
        <WaveformCanvas
          asset={item.asset}
          srcInMs={item.srcInMs}
          srcOutMs={item.srcOutMs}
          width={width}
          height={TRACK_HEIGHT.audio - 6}
          selected={selected}
        />
      )}
      {/* VIDEO-clip audio waveform: talking-head / screen-rec audio
          is muxed into the VIDEO clip, so there's no separate audio clip to draw
          on — the user saw "no waveform". Draw a slim strip pinned to the clip
          BOTTOM (over the filmstrip). getWaveform returns null for a SILENT clip
          (b-roll), so the strip self-hides via data-wave. Not images. */}
      {!offline && item.kind === 'video' && !item.isImage && item.asset && item.srcInMs !== undefined && item.srcOutMs !== undefined && (
        <WaveformCanvas
          asset={item.asset}
          srcInMs={item.srcInMs}
          srcOutMs={item.srcOutMs}
          width={width}
          height={WAVE_VIDEO_STRIP_H}
          selected={selected}
          bottom
        />
      )}
      {offline && item.asset && (
        <span className="tl-clip-offline" data-cut-timeline-offline={item.id}>
          <Icon name="warning" size={14} />
          {wide && <span className="tl-clip-offline__label">Source missing</span>}
          <button
            type="button"
            className="tl-clip-offline__relink"
            data-cut-action="timeline-relink-offline"
            data-cut-timeline-relink={item.asset}
            disabled={relinking}
            aria-label={`Relink missing source ${label}`}
            title={`Relink missing source ${label}`}
            onMouseDown={(event) => event.stopPropagation()}
            onClick={(event) => {
              event.stopPropagation()
              void onRelinkAsset(item.asset!)
            }}
          >
            {relinking ? '…' : wide ? 'Relink' : '↗'}
          </button>
        </span>
      )}
      {/* fade corner triangles (edit.fade): width = ramp length in time-px,
          clamped to the clip (renderer clamps long fades the same way) */}
      {item.fade && item.fade.in_ms > 0 && (
        <span
          className="tl-fade tl-fade--in"
          data-cut-fade-in={item.id}
          style={{ width: Math.min(width, msToPx(item.fade.in_ms, zoom)) }}
          title={`fade in ${shortDur(item.fade.in_ms)} (${item.fade.kind})`}
        />
      )}
      {item.fade && item.fade.out_ms > 0 && (
        <span
          className="tl-fade tl-fade--out"
          data-cut-fade-out={item.id}
          style={{ width: Math.min(width, msToPx(item.fade.out_ms, zoom)) }}
          title={`fade out ${shortDur(item.fade.out_ms)} (${item.fade.kind})`}
        />
      )}
      {/* still-image clip: photo glyph — distinct from motion video at a glance */}
      {wide && !offline && item.isImage && (
        <Icon name="image" size={14} className="tl-photo-icon" />
      )}
      {wide && !offline && <span className="tl-clip-name">{label}</span>}
      {/* overlay geometry badge (edit.transform) — small, never a status color */}
      {item.transform && (
        <span className="tl-pip-badge" data-cut-pip={item.id} title={pipTitle}>PiP</span>
      )}
      {item.motionLink && (
        <span
          className={`tl-motion-badge tl-motion-badge--${item.motionLink.state}`}
          data-cut-motion-link={item.id}
          data-cut-motion-state={item.motionLink.state}
          title={`Linked ShellX Motion · ${item.motionLink.state} · ${item.motionLink.packageId}`}
        >M</span>
      )}
      {/* speed badge (edit.speed) — shows the retime factor on sped clips; the
          clip is ALREADY drawn at its real (post-speed) timeline width. */}
      {item.speed && (
        <span
          className="tl-speed-badge"
          data-cut-speed-badge={item.id}
          title={`${item.speed}× speed${item.speed > 1 ? ' (faster)' : ' (slow motion)'}`}
        >
          {item.speed}×
        </span>
      )}
      {/* grade badge (edit.grade) — marks clips carrying a non-identity color
          grade so users can see at a glance which clips are graded. */}
      {item.graded && (
        <span className="tl-grade-badge" data-cut-grade-badge={item.id} title="Colour grade applied">
          ◐
        </span>
      )}
      {wide && !offline && <span className="tl-clip-dur">{shortDur(item.durMs)}</span>}
      {/* Crossfade overlap wedge (edit.crossfade, xfade_in_ms > 0): a bracket
          at the clip's START showing the dissolve overlap with the LEFT
          neighbour. Width = overlap in time-px; the timeline is SHORTER by
          this amount (the overlap is shared). Quiet blue (position/edit accent
          — the crossfade is an edit, not a status). pointer-events none. */}
      {item.xfadeInMs && item.xfadeInMs > 0 && (
        <span
          className="tl-xfade"
          data-cut-xfade={item.id}
          data-cut-xfade-ms={item.xfadeInMs}
          style={{ width: Math.min(width, msToPx(item.xfadeInMs, zoom)) }}
          title={`crossfade ${shortDur(item.xfadeInMs)} (overlap with the previous clip)`}
        />
      )}
      {/* Trim zones: 9px hit area overhanging 3px outward, per-side cursors
          (timeline behavior contract). AV clips → edit.trim; caption clips → captions
          .set_range; captions are draggable and trimmable. */}
      <>
        <div className="tl-trim tl-trim--l" data-cut-trim={`${item.id}:l`}
          onMouseDown={(e) => onClipDown(e, item, 'trim-l')} />
        <div className="tl-trim tl-trim--r" data-cut-trim={`${item.id}:r`}
          onMouseDown={(e) => onClipDown(e, item, 'trim-r')} />
      </>
    </div>
  )
})

export default ClipView
