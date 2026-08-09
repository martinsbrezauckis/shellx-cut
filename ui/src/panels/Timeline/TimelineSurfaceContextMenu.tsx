import ContextMenuFrame from '../../components/ContextMenuFrame'
import type { ReactNode } from 'react'
import { Icon } from '../../icons'
import { confirmAction } from '../../lib/tauri'
import { runUserVerb } from '../../lib/userActionFeedback'
import type { Project } from '../../lib/client'
import type { LaidItem } from './layout'
import { gapFillState, removeTrackState, type TimelineSurfaceMenuState } from './TimelineSurfaceMenuModel'

interface TimelineSurfaceContextMenuProps {
  menu: TimelineSurfaceMenuState
  project: Project | null
  allItems: LaidItem[]
  clipboardClipId: string | null
  durationMs: number
  onSeek: (atMs: number) => void
  onExportRange: (range: [number, number] | null) => void
  onAddTrack: (kind: 'video' | 'audio') => void | Promise<void>
  onSelect: (clipIds: string[]) => void
  onRemoveTrack: (trackId: string) => void | Promise<void>
  onClose: () => void
}

function Item({ action, title, disabled, danger = false, children, onClick }: {
  action: string
  title: string
  disabled?: boolean
  danger?: boolean
  children: ReactNode
  onClick: () => void
}) {
  return <button className={`tl-ctx__item${danger ? ' tl-ctx__item--danger' : ''}`} data-cut-timeline-ctx={action} role="menuitem" disabled={disabled} title={title} onClick={onClick}>{children}</button>
}

/** Empty-lane, gap, and locked-track menus. Media clip menus remain in their
 * own owner; this component only exposes actions valid for the resolved target. */
export default function TimelineSurfaceContextMenu({
  menu,
  project,
  allItems,
  clipboardClipId,
  durationMs,
  onSeek,
  onExportRange,
  onAddTrack,
  onSelect,
  onRemoveTrack,
  onClose,
}: TimelineSurfaceContextMenuProps) {
  if (menu.kind === 'empty') {
    const canEditProject = !!project
    const canSetIn = durationMs - menu.atMs >= 50
    const canSetOut = menu.atMs >= 50
    return <ContextMenuFrame x={menu.x} y={menu.y} menuId="data-cut-timeline-empty-menu" backdropId="data-cut-timeline-ctx-backdrop" onClose={onClose}>
      <span className="tl-ctx__label" aria-hidden="true">Timeline</span>
      <Item action="empty-seek" title="Move the playhead to this exact timeline position" onClick={() => { onSeek(menu.atMs); onClose() }}><Icon name="marker" size={14} /> Seek here</Item>
      <Item action="empty-marker" disabled={!canEditProject} title={canEditProject ? 'Add a marker at this exact timeline position' : 'Open a project before adding a marker'} onClick={() => { if (!canEditProject) return; void runUserVerb('edit.add_marker', { at_ms: Math.round(menu.atMs), label: `m @ ${Math.round(menu.atMs)}ms`, rationale: 'add marker from empty timeline menu' }, 'Could not add a marker.'); onClose() }}><Icon name="marker" size={14} /> Add marker here</Item>
      <Item action="empty-mark-in" disabled={!canEditProject || !canSetIn} title={!canEditProject ? 'Open a project before setting an export range' : canSetIn ? 'Set the export range in-point here' : 'Move earlier to leave an export range after this point'} onClick={() => { if (!canEditProject) return; onExportRange([Math.round(menu.atMs), Math.round(durationMs)]); onClose() }}><Icon name="marker" size={14} /> Set export in</Item>
      <Item action="empty-mark-out" disabled={!canEditProject || !canSetOut} title={!canEditProject ? 'Open a project before setting an export range' : canSetOut ? 'Set the export range out-point here' : 'Move later to leave an export range before this point'} onClick={() => { if (!canEditProject) return; onExportRange([0, Math.round(menu.atMs)]); onClose() }}><Icon name="marker" size={14} /> Set export out</Item>
      <span className="tl-ctx__sep" aria-hidden="true" />
      <Item action="empty-add-video-track" disabled={!canEditProject} title={canEditProject ? 'Add a new video or overlay track' : 'Open a project before adding a track'} onClick={() => { if (!canEditProject) return; void onAddTrack('video'); onClose() }}><Icon name="video" size={14} /> Add video track</Item>
      <Item action="empty-add-audio-track" disabled={!canEditProject} title={canEditProject ? 'Add a new audio track' : 'Open a project before adding a track'} onClick={() => { if (!canEditProject) return; void onAddTrack('audio'); onClose() }}><Icon name="audioClip" size={14} /> Add audio track</Item>
    </ContextMenuFrame>
  }

  if (menu.kind === 'gap') {
    const gap = allItems.find((candidate) => candidate.id === menu.itemId && candidate.kind === 'gap') ?? null
    const track = gap ? project?.tracks.find((candidate) => candidate.id === gap.trackId) ?? null : null
    if (!gap || !track) return null
    const source = clipboardClipId ? allItems.find((candidate) => candidate.id === clipboardClipId) ?? null : null
    const fit = gapFillState(gap, track, source)
    return <ContextMenuFrame x={menu.x} y={menu.y} menuId="data-cut-gap-menu" backdropId="data-cut-timeline-ctx-backdrop" onClose={onClose}>
      <span className="tl-ctx__label" aria-hidden="true">Gap · {Math.round(gap.durMs)}ms</span>
      <Item action="gap-seek" title="Move the playhead to the start of this exact gap" onClick={() => { onSeek(gap.startMs); onClose() }}><Icon name="marker" size={14} /> Seek to gap start</Item>
      <Item action="gap-select-range" title="Select this exact empty span as the export range" onClick={() => { onExportRange([Math.round(gap.startMs), Math.round(gap.startMs + gap.durMs)]); onClose() }}><Icon name="fitTimeline" size={14} /> Select gap range</Item>
      <Item action="gap-fit-clipboard" disabled={!fit.enabled} title={fit.reason} onClick={() => {
        if (!fit.enabled || !source) return
        void runUserVerb('edit.fit_to_fill', { track: gap.trackId, at_ms: Math.round(gap.editorialStartMs), source_clip: source.id, rationale: `fit copied ${source.id} into gap on ${gap.trackId}` }, 'Could not fit the copied clip into this gap.')
        onClose()
      }}><Icon name="fitTimeline" size={14} /> Fit copied clip to gap</Item>
    </ContextMenuFrame>
  }

  const track = project?.tracks.find((candidate) => candidate.id === menu.trackId) ?? null
  if (!track) return null
  const remove = removeTrackState(track, project?.tracks ?? [])
  const item = menu.itemId ? allItems.find((candidate) => candidate.id === menu.itemId) ?? null : null
  return <ContextMenuFrame x={menu.x} y={menu.y} menuId="data-cut-locked-track-menu" backdropId="data-cut-timeline-ctx-backdrop" onClose={onClose}>
    <span className="tl-ctx__label" aria-hidden="true">Locked track · {track.id}</span>
    {item && <Item action="locked-inspect" title="Select this locked clip for inspection; no edit is dispatched" onClick={() => { onSelect([item.id]); document.dispatchEvent(new CustomEvent('cut:open-ui-surface', { detail: { id: 'properties' } })); onClose() }}><Icon name="info" size={14} /> Inspect clip</Item>}
    <Item action="locked-unlock" title={`Unlock ${track.id}; edits remain blocked until you choose this`} onClick={() => { void runUserVerb('edit.track_lock', { track: track.id, on: false, rationale: `unlock ${track.id} from locked-track menu` }, `Could not unlock track ${track.id}.`); onClose() }}><Icon name="lock" size={14} /> Unlock track</Item>
    <Item action="locked-remove-track" danger disabled={!remove.enabled} title={remove.reason} onClick={() => {
      if (!remove.enabled) return
      void (async () => {
        if (!await confirmAction(`Remove track “${track.id}” and its clips?\n\nThis is destructive, but undo can restore the timeline change.`, { title: 'Remove track?', okLabel: 'Remove track', cancelLabel: 'Keep track' })) return
        await onRemoveTrack(track.id)
        onClose()
      })()
    }}><Icon name="trash" size={14} /> Remove track…</Item>
  </ContextMenuFrame>
}
