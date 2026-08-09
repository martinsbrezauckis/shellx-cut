import type { MouseEvent } from 'react'
import type { Track, WindowThumbs } from '../../lib/client'
import { useOfflineMedia } from '../../app/OfflineMediaContext'
import { msToPx, TRACK_HEIGHT, type LaidItem, type Seam } from './layout'
import ClipView from './ClipView'
import DuckStrip from './DuckStrip'
import TimelineSeamHandles from './TimelineSeamHandles'
import {
  GainControl,
  KindIcon,
  ListenButton,
  MuteButton,
  PanControl,
  SoloButton,
  TrackLockButton,
  TrackOrderControls,
  TrackVisibilityButton,
} from './TrackControls'

type ClipGestureMode = 'move' | 'trim-l' | 'trim-r'

interface TimelineTrackGhost {
  trackId: string
  srcTrackId: string
  startMs: number
  durMs: number
}

interface TimelineTrackRowProps {
  track: Track
  tracks: Track[]
  items: LaidItem[]
  groupStart: boolean
  isDrop: boolean
  dropInvalid: boolean
  baseVideoId: string | undefined
  contentW: number
  zoom: number
  selectedClipIds: string[]
  draggingClipId: string | null
  filmstrips: Map<string, { url: string; assetDurMs: number }>
  windowedTiles: Map<string, WindowThumbs>
  assetLabels: ReadonlyMap<string, string>
  seams: Seam[]
  activeSeam: Seam | null
  ghost: TimelineTrackGhost | null
  auditionRevisionKey: string
  onLaneDown: (e: MouseEvent<HTMLDivElement>) => void
  onClipDown: (e: MouseEvent, item: LaidItem, mode: ClipGestureMode) => void
  onSeamDown: (e: MouseEvent<HTMLDivElement>, seam: Seam) => void
}

export default function TimelineTrackRow({
  track,
  tracks,
  items,
  groupStart,
  isDrop,
  dropInvalid,
  baseVideoId,
  contentW,
  zoom,
  selectedClipIds,
  draggingClipId,
  filmstrips,
  windowedTiles,
  assetLabels,
  seams,
  activeSeam,
  ghost,
  auditionRevisionKey,
  onLaneDown,
  onClipDown,
  onSeamDown,
}: TimelineTrackRowProps) {
  const { offlineAssetIds, relinkAsset, relinkingAssetId } = useOfflineMedia()
  const dropCls = isDrop ? (dropInvalid ? ' tl-track--drop-bad' : ' tl-track--drop-ok') : ''
  const visible = track.visible !== false
  const locked = !!track.locked
  return (
    <div
      className={`tl-track tl-track--${track.kind}${groupStart ? ' tl-track--group-start' : ''}${visible ? '' : ' tl-track--hidden'}${locked ? ' tl-track--locked' : ''}${dropCls}`}
      style={{ height: TRACK_HEIGHT[track.kind] ?? 40 }}
      data-cut-track={track.id}
      data-cut-track-visible={visible ? 'true' : 'false'}
      data-cut-track-locked={locked ? 'true' : 'false'}
      data-cut-locked={locked || undefined}
      data-cut-drop={isDrop ? (dropInvalid ? 'bad' : 'ok') : undefined}
    >
      <div className="tl-track-head" data-cut-track-kind={track.kind} onMouseDown={(e) => e.stopPropagation()}>
        <span className="tl-track-meta">
          <span className="tl-kind-icon"><KindIcon kind={track.kind} /></span>
          <span className="tl-track-name" title={track.id}>{track.id}</span>
          {track.kind === 'video' && track.id !== baseVideoId && (
            <span className="tl-overlay-tag" data-cut-overlay-track={track.id}>overlay</span>
          )}
          {track.kind === 'audio' && <TrackLockButton trackId={track.id} locked={locked} />}
        </span>
        <span className="tl-track-actions">
          {(track.kind === 'video' || track.kind === 'caption') && (
            <TrackVisibilityButton trackId={track.id} visible={visible} />
          )}
          {track.kind === 'video' && <TrackOrderControls tracks={tracks} trackId={track.id} />}
          {track.kind === 'audio' && (
            <>
            <MuteButton trackId={track.id} muted={!!track.muted} />
            <SoloButton trackId={track.id} solo={!!track.solo} />
            </>
          )}
          {track.kind === 'audio' && (
            <>
            <ListenButton trackId={track.id} revisionKey={auditionRevisionKey} />
            <PanControl trackId={track.id} pan={track.pan ?? 0} />
            <GainControl trackId={track.id} db={track.gain_db ?? 0} />
            </>
          )}
          {track.kind !== 'audio' && <TrackLockButton trackId={track.id} locked={locked} />}
        </span>
      </div>
      <div className="tl-lane" style={{ width: contentW }} onMouseDown={onLaneDown} data-cut-locked={locked || undefined}>
        {items.map((it) => (
          <ClipView
            key={it.id}
            item={it}
            zoom={zoom}
            selected={selectedClipIds.includes(it.id)}
            dragging={draggingClipId === it.id}
            locked={locked}
            displayName={it.asset ? assetLabels.get(it.asset) : undefined}
            offline={!!it.asset && offlineAssetIds.has(it.asset)}
            relinking={!!it.asset && relinkingAssetId === it.asset}
            filmstrip={it.asset ? filmstrips.get(it.asset) : undefined}
            windowed={windowedTiles.get(it.id)}
            onClipDown={onClipDown}
            onRelinkAsset={relinkAsset}
          />
        ))}
        {track.kind === 'audio' &&
          (track.gain_windows ?? []).map((w, i) => (
            <DuckStrip key={`${w.range_ms[0]}:${i}`} w={w} zoom={zoom} />
          ))}
        <TimelineSeamHandles
          seams={seams}
          activeSeam={activeSeam}
          zoom={zoom}
          onSeamDown={onSeamDown}
        />
        {ghost && ghost.trackId === track.id && (
          <div
            className="tl-ghost"
            data-cut-ghost
            data-cut-ghost-moved={ghost.trackId !== ghost.srcTrackId || undefined}
            style={{ left: msToPx(ghost.startMs, zoom), width: Math.max(2, msToPx(ghost.durMs, zoom)) }}
          />
        )}
      </div>
    </div>
  )
}
