import { Icon } from '../../icons'
import type { LaidItem, TimelineContentClass } from './layout'

interface GeneratedOverlayContextMenuProps {
  item: LaidItem
  menu: { x: number; y: number; atMs: number }
  onClose: () => void
  onSelect: (clipIds: string[]) => void
  removeItemById: (itemId: string, ripple: boolean) => void | Promise<void>
  removeTrackById: (trackId: string) => void | Promise<void>
  splitItemAt: (itemId: string, atMs: number) => void
  isOverlayTrack: boolean
}

function clampMenu(el: HTMLDivElement, x: number, y: number): void {
  const margin = 8
  const rect = el.getBoundingClientRect()
  el.style.left = `${Math.max(margin, Math.min(x, window.innerWidth - rect.width - margin))}px`
  el.style.top = `${Math.max(margin, Math.min(y, window.innerHeight - rect.height - margin))}px`
}

/**
 * The intentionally short menu for generated title and shape renders.
 *
 * Generated overlays have editing identity in their Inspector data, not in the
 * rendered media file. Their menu therefore only exposes selection/Inspector,
 * transform, split, and removal operations — no generic footage or clipboard
 * operations can accidentally sever that identity.
 */
export default function GeneratedOverlayContextMenu({
  item,
  menu,
  onClose,
  onSelect,
  removeItemById,
  removeTrackById,
  splitItemAt,
  isOverlayTrack,
}: GeneratedOverlayContextMenuProps) {
  const kind = item.contentClass as Extract<TimelineContentClass, 'title' | 'shape'>
  const label = kind === 'shape' ? 'Shape' : 'Title'
  const editLabel = kind === 'shape' ? 'Edit shape…' : 'Edit title…'
  return (
    <>
      <div className="tl-ctx-backdrop" data-cut-ctx-backdrop onMouseDown={onClose} onContextMenu={(event) => { event.preventDefault(); onClose() }} />
      <div className="tl-ctx" role="menu" data-cut-clip-menu data-cut-clip-kind={kind} style={{ left: menu.x, top: menu.y }}
        ref={(el) => { if (el) clampMenu(el, menu.x, menu.y) }}
      >
        <span className="tl-ctx__label" aria-hidden="true">{label}</span>
        <button className="tl-ctx__item" data-cut-ctx="overlay-edit" role="menuitem"
          title={`Select this ${label.toLowerCase()} and open its Inspector controls`}
          onClick={() => {
            onSelect([item.id])
            document.dispatchEvent(new CustomEvent('cut:open-ui-surface', { detail: { id: 'properties' } }))
            onClose()
          }}>
          <Icon name="captions" size={14} /> {editLabel}
        </button>
        <button className="tl-ctx__item" data-cut-ctx="transform" role="menuitem"
          title="Open Transform / Layer for position, scale, and opacity"
          onClick={() => { onSelect([item.id]); document.dispatchEvent(new CustomEvent('cut:open-layer')); onClose() }}>
          <Icon name="transform" size={14} /> Transform…
        </button>
        <button className="tl-ctx__item" data-cut-ctx="split" role="menuitem"
          title="Split this generated overlay at the click point (S)"
          onClick={() => { splitItemAt(item.id, menu.atMs); onClose() }}>
          <Icon name="split" size={14} /> Split here <kbd className="tl-ctx__kbd">S</kbd>
        </button>
        <span className="tl-ctx__sep" aria-hidden="true" />
        <button className="tl-ctx__item tl-ctx__item--danger" data-cut-ctx="remove" role="menuitem"
          title={`Remove this ${label.toLowerCase()} and close the gap (Del)`}
          onClick={() => { void removeItemById(item.id, true); onClose() }}>
          <Icon name="rippleDelete" size={14} /> Remove <kbd className="tl-ctx__kbd">Del</kbd>
        </button>
        {isOverlayTrack && (
          <button className="tl-ctx__item tl-ctx__item--danger" data-cut-ctx="remove-track" role="menuitem"
            onClick={() => { void removeTrackById(item.trackId); onClose() }}>
            <Icon name="trash" size={14} /> Remove track “{item.trackId}”
          </button>
        )}
      </div>
    </>
  )
}
