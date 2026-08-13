import type { ReactNode } from 'react'
import ContextMenuFrame from '../../components/ContextMenuFrame'
import { Icon } from '../../icons'
import { confirmAction } from '../../lib/tauri'
import type { Project } from '../../lib/client'
import { runUserVerb } from '../../lib/userActionFeedback'
import type { LaidItem } from './layout'
import { removeTrackState, type TimelineSurfaceMenuState } from './TimelineSurfaceMenuModel'

interface TimelineTrackContextMenuProps {
  menu: Extract<TimelineSurfaceMenuState, { kind: 'track' | 'locked' }>
  project: Project | null
  allItems: LaidItem[]
  onSelect: (clipIds: string[]) => void
  onRemoveTrack: (trackId: string) => void | Promise<void>
  onClose: () => void
}

function Item({ action, title, disabled = false, danger = false, children, onClick }: {
  action: string
  title: string
  disabled?: boolean
  danger?: boolean
  children: ReactNode
  onClick: () => void
}) {
  return <button
    className={`tl-ctx__item${danger ? ' tl-ctx__item--danger' : ''}`}
    data-cut-track-ctx={action}
    role="menuitem"
    disabled={disabled}
    title={title}
    aria-description={disabled ? title : undefined}
    onClick={onClick}
  >{children}</button>
}

/** Track-header and locked-track context owner. Dense mixer/reorder controls
 * remain in the header; this menu only exposes discrete track operations. */
export default function TimelineTrackContextMenu({
  menu,
  project,
  allItems,
  onSelect,
  onRemoveTrack,
  onClose,
}: TimelineTrackContextMenuProps) {
  const track = project?.tracks.find((candidate) => candidate.id === menu.trackId) ?? null
  if (!track) return null
  const remove = removeTrackState(track, project?.tracks ?? [])
  const toggleLock = () => {
    void runUserVerb('edit.track_lock', {
      track: track.id,
      on: !track.locked,
      rationale: `${track.locked ? 'unlock' : 'lock'} ${track.id} from track context menu`,
    }, `Could not ${track.locked ? 'unlock' : 'lock'} track ${track.id}.`)
    onClose()
  }
  const removeTrack = () => {
    void (async () => {
      if (!await confirmAction(`Remove track “${track.id}” and its clips?\n\nThis is destructive, but undo can restore the timeline change.`, { title: 'Remove track?', okLabel: 'Remove track', cancelLabel: 'Keep track' })) return
      await onRemoveTrack(track.id)
      onClose()
    })()
  }
  const item = menu.kind === 'locked' && menu.itemId
    ? allItems.find((candidate) => candidate.id === menu.itemId) ?? null
    : null
  const label = menu.kind === 'locked' ? `Locked track · ${track.id}` : `Track · ${track.id}`
  return <ContextMenuFrame
    x={menu.x}
    y={menu.y}
    menuId={menu.kind === 'locked' ? 'data-cut-locked-track-menu' : 'data-cut-track-menu'}
    backdropId="data-cut-timeline-ctx-backdrop"
    onClose={onClose}
    ariaLabel={label}
  >
    <span className="tl-ctx__label" aria-hidden="true">{label}</span>
    {item && <Item action="inspect" title="Select this locked clip for inspection; no edit is dispatched" onClick={() => {
      onSelect([item.id])
      document.dispatchEvent(new CustomEvent('cut:open-ui-surface', { detail: { id: 'properties' } }))
      onClose()
    }}><Icon name="info" size={14} /> Inspect clip</Item>}
    <Item action="lock" title={`${track.locked ? 'Unlock' : 'Lock'} ${track.id}`} onClick={toggleLock}>
      <Icon name="lock" size={14} /> {track.locked ? 'Unlock track' : 'Lock track'}
    </Item>
    {menu.kind === 'track' && (track.kind === 'video' || track.kind === 'caption') && <Item
      action="visibility"
      title={track.visible === false ? `Show ${track.id} in preview and export` : `Hide ${track.id} from preview and export`}
      onClick={() => {
        void runUserVerb('edit.track_visible', {
          track: track.id,
          on: track.visible === false,
          rationale: `${track.visible === false ? 'show' : 'hide'} ${track.id} from track context menu`,
        }, `Could not change visibility for ${track.id}.`)
        onClose()
      }}
    ><Icon name={track.visible === false ? 'redact' : 'eye'} size={14} /> {track.visible === false ? 'Show track' : 'Hide track'}</Item>}
    {menu.kind === 'track' && track.kind === 'audio' && <>
      <Item action="mute" title={track.muted ? `Unmute ${track.id}` : `Mute ${track.id}`} onClick={() => {
        void runUserVerb('edit.mute', { track: track.id, on: !track.muted, rationale: `${track.muted ? 'unmute' : 'mute'} ${track.id} from track context menu` }, `Could not change mute for ${track.id}.`)
        onClose()
      }}><Icon name="mute" size={14} /> {track.muted ? 'Unmute track' : 'Mute track'}</Item>
      <Item action="solo" title={track.solo ? `Clear solo on ${track.id}` : `Solo ${track.id}`} onClick={() => {
        void runUserVerb('edit.solo', { track: track.id, on: !track.solo, rationale: `${track.solo ? 'clear solo on' : 'solo'} ${track.id} from track context menu` }, `Could not change solo for ${track.id}.`)
        onClose()
      }}><Icon name="audioClip" size={14} /> {track.solo ? 'Clear solo' : 'Solo track'}</Item>
    </>}
    {remove.enabled && <>
      <span className="tl-ctx__sep" aria-hidden="true" />
      <Item action="remove" danger title={remove.reason} onClick={removeTrack}><Icon name="trash" size={14} /> Remove track…</Item>
    </>}
  </ContextMenuFrame>
}
