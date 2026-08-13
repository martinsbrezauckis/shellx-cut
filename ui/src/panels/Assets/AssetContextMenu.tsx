import ContextMenuFrame from '../../components/ContextMenuFrame'
import type { ReactNode } from 'react'
import { Icon } from '../../icons'

export interface AssetMenuTarget {
  id: string
  name: string
  kind?: string
  offline: boolean
  used: number
}

export interface AssetContextMenuState {
  x: number
  y: number
  assetId: string
}

interface AssetContextMenuProps {
  menu: AssetContextMenuState
  asset: AssetMenuTarget | null
  busy: boolean
  onOpenSource: (assetId: string) => void
  onAddAtPlayhead: (assetId: string) => void
  onRelink: (assetId: string) => void
  onRemove: (assetId: string) => void
  onClose: () => void
}

function Item({ action, disabled = false, title, danger = false, children, onClick }: {
  action: string
  disabled?: boolean
  title: string
  danger?: boolean
  children: ReactNode
  onClick: () => void
}) {
  return <button className={`tl-ctx__item${danger ? ' tl-ctx__item--danger' : ''}`} data-cut-asset-ctx={action} role="menuitem" disabled={disabled} title={title} onClick={onClick}>{children}</button>
}

/** Exact-asset operations only. Non-media and offline source-monitor routes are
 * class/precondition invalid rather than being guessed from another card. */
export default function AssetContextMenu({ menu, asset, busy, onOpenSource, onAddAtPlayhead, onRelink, onRemove, onClose }: AssetContextMenuProps) {
  if (!asset) return null
  const supportsSource = asset.kind === 'video' || asset.kind === 'audio'
  const addReason = asset.offline ? 'Relink this source before adding it to the timeline' : 'Add this exact asset at the current playhead'
  const removeReason = asset.used > 0
    ? `Remove its ${asset.used} timeline clip${asset.used === 1 ? '' : 's'} first`
    : 'Remove this asset from the project after confirmation; its source file stays on disk'
  return <ContextMenuFrame x={menu.x} y={menu.y} menuId="data-cut-asset-menu" backdropId="data-cut-asset-ctx-backdrop" onClose={onClose}>
    <span className="tl-ctx__label" aria-hidden="true">Asset · {asset.name}</span>
    {supportsSource && <Item action="asset-open-source" disabled={asset.offline || busy} title={asset.offline ? 'Relink this source before opening it in Source Monitor' : 'Open this exact asset in Source Monitor'} onClick={() => {
      // `onOpenSource` centrally delays the modal mount until this native click
      // has settled. Schedule that guarded open before retiring this menu; a
      // requestAnimationFrame owned by the unmounting menu is not reliable in
      // every desktop WebView.
      onOpenSource(asset.id)
      onClose()
    }}><Icon name="screenPlay" size={14} /> Open in Source Monitor</Item>}
    <Item action="asset-add-playhead" disabled={asset.offline || busy} title={addReason} onClick={() => { onAddAtPlayhead(asset.id); onClose() }}><Icon name="plus" size={14} /> Add at playhead</Item>
    {asset.offline && <Item action="asset-relink" disabled={busy} title="Relink this exact missing source without changing its asset identity" onClick={() => { onRelink(asset.id); onClose() }}><Icon name="link" size={14} /> Relink source…</Item>}
    <span className="tl-ctx__sep" aria-hidden="true" />
    <Item action="asset-remove" danger disabled={asset.used > 0 || busy} title={removeReason} onClick={() => { onRemove(asset.id); onClose() }}><Icon name="trash" size={14} /> Remove from project…</Item>
  </ContextMenuFrame>
}
