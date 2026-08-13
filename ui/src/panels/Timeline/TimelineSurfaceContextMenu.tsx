import ContextMenuFrame from '../../components/ContextMenuFrame'
import type { ReactNode } from 'react'
import { Icon } from '../../icons'
import type { Project, Track } from '../../lib/client'
import { runUserVerb } from '../../lib/userActionFeedback'
import type { LaidItem } from './layout'
import { gapFillState, type TimelineSurfaceMenuState } from './TimelineSurfaceMenuModel'

interface TimelineSurfaceContextMenuProps {
  menu: Extract<TimelineSurfaceMenuState, { kind: 'empty' | 'gap' }>
  project: Project | null
  allItems: LaidItem[]
  clipboardClipId: string | null
  clipboardKind: 'video' | 'audio' | null
  durationMs: number
  onSeek: (atMs: number) => void
  onExportRange: (range: [number, number] | null) => void
  onAddTrack: (kind: 'video' | 'audio') => void | Promise<void>
  onPasteAt: (atMs: number, trackId: string) => void
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
  return <button className={`tl-ctx__item${danger ? ' tl-ctx__item--danger' : ''}`} data-cut-timeline-ctx={action} role="menuitem" disabled={disabled} title={title} aria-description={disabled ? title : undefined} onClick={onClick}>{children}</button>
}

function pasteState(track: Track | null, clipboardHasContent: boolean, clipboardKind: 'video' | 'audio' | null): { enabled: boolean; reason: string } {
  if (!clipboardHasContent || !clipboardKind) return { enabled: false, reason: 'Copy or cut a clip first' }
  if (!track) return { enabled: false, reason: 'Right-click a video or audio timeline lane to choose a paste target' }
  if (track.kind !== clipboardKind) return { enabled: false, reason: `The copied ${clipboardKind} clip needs ${clipboardKind === 'audio' ? 'an' : 'a'} ${clipboardKind} track` }
  if (track.locked) return { enabled: false, reason: `Unlock ${track.id} before pasting` }
  return { enabled: true, reason: `Paste the copied ${clipboardKind} clip at this exact position on ${track.id}` }
}

/** Empty-lane, gap, and locked-track menus. Media clip menus remain in their
 * own owner; this component only exposes actions valid for the resolved target. */
export default function TimelineSurfaceContextMenu({
  menu,
  project,
  allItems,
  clipboardClipId,
  clipboardKind,
  durationMs,
  onSeek,
  onExportRange,
  onAddTrack,
  onPasteAt,
  onClose,
}: TimelineSurfaceContextMenuProps) {
  if (menu.kind === 'empty') {
    const canEditProject = !!project
    const track = menu.trackId ? project?.tracks.find((candidate) => candidate.id === menu.trackId) ?? null : null
    const paste = pasteState(track, !!clipboardClipId, clipboardKind)
    const canSetIn = durationMs - menu.atMs >= 50
    const canSetOut = menu.atMs >= 50
    return <ContextMenuFrame x={menu.x} y={menu.y} menuId="data-cut-timeline-empty-menu" backdropId="data-cut-timeline-ctx-backdrop" onClose={onClose}>
      <span className="tl-ctx__label" aria-hidden="true">Timeline</span>
      <Item action="empty-seek" title="Move the playhead to this exact timeline position" onClick={() => { onSeek(menu.atMs); onClose() }}><Icon name="marker" size={14} /> Seek here</Item>
      <Item action="empty-paste" disabled={!paste.enabled} title={paste.reason} onClick={() => {
        if (!paste.enabled || !track) return
        onPasteAt(menu.atMs, track.id)
        onClose()
      }}><Icon name="paste" size={14} /> Paste copied clip here</Item>
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
    const paste = pasteState(track, !!clipboardClipId, clipboardKind)
    return <ContextMenuFrame x={menu.x} y={menu.y} menuId="data-cut-gap-menu" backdropId="data-cut-timeline-ctx-backdrop" onClose={onClose}>
      <span className="tl-ctx__label" aria-hidden="true">Gap · {Math.round(gap.durMs)}ms</span>
      <Item action="gap-seek" title="Move the playhead to the start of this exact gap" onClick={() => { onSeek(gap.startMs); onClose() }}><Icon name="marker" size={14} /> Seek to gap start</Item>
      <Item action="gap-paste" disabled={!paste.enabled} title={paste.reason} onClick={() => {
        if (!paste.enabled) return
        onPasteAt(gap.startMs, track.id)
        onClose()
      }}><Icon name="paste" size={14} /> Paste copied clip here</Item>
      <Item action="gap-select-range" title="Select this exact empty span as the export range" onClick={() => { onExportRange([Math.round(gap.startMs), Math.round(gap.startMs + gap.durMs)]); onClose() }}><Icon name="fitTimeline" size={14} /> Select gap range</Item>
      <Item action="gap-fit-clipboard" disabled={!fit.enabled} title={fit.reason} onClick={() => {
        if (!fit.enabled || !source) return
        void runUserVerb('edit.fit_to_fill', { track: gap.trackId, at_ms: Math.round(gap.editorialStartMs), source_clip: source.id, rationale: `fit copied ${source.id} into gap on ${gap.trackId}` }, 'Could not fit the copied clip into this gap.')
        onClose()
      }}><Icon name="fitTimeline" size={14} /> Fit copied clip to gap</Item>
    </ContextMenuFrame>
  }

  return null
}
