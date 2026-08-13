import { useCallback, useEffect, useState, type RefObject, type ReactNode } from 'react'
import ContextMenuFrame from '../../components/ContextMenuFrame'
import { Icon } from '../../icons'
import { runUserVerb } from '../../lib/userActionFeedback'
import type { ActiveVideo } from './model'

interface PreviewContextMenuProps {
  monitorRef: RefObject<HTMLDivElement | null>
  video: ActiveVideo | null
  hasProject: boolean
  playheadMs: number
  onSeek: (atMs: number) => void
}

interface MenuState {
  x: number
  y: number
  video: ActiveVideo | null
  atMs: number
}

function Item({ action, disabled = false, title, children, onClick }: {
  action: string
  disabled?: boolean
  title: string
  children: ReactNode
  onClick: () => void
}) {
  return <button className="tl-ctx__item" data-cut-preview-ctx={action} role="menuitem" disabled={disabled} title={title} onClick={onClick}>{children}</button>
}

/** Monitor context routes preserve the base clip identity captured at the
 * click. A black/gap monitor never guesses an asset from a different layer. */
export default function PreviewContextMenu({ monitorRef, video, hasProject, playheadMs, onSeek }: PreviewContextMenuProps) {
  const [menu, setMenu] = useState<MenuState | null>(null)
  const openMenu = useCallback((x: number, y: number) => {
    setMenu({ x, y, video, atMs: Math.max(0, Math.round(playheadMs)) })
  }, [playheadMs, video])
  useEffect(() => {
    const monitor = monitorRef.current
    if (!monitor) return
    const onContextMenu = (event: MouseEvent) => {
      event.preventDefault()
      openMenu(event.clientX, event.clientY)
    }
    monitor.addEventListener('contextmenu', onContextMenu)
    return () => monitor.removeEventListener('contextmenu', onContextMenu)
  }, [monitorRef, openMenu])
  const trigger = <button
    type="button"
    className="pv-context-trigger"
    data-cut-preview-menu-button
    title="More Preview actions"
    aria-label="More Preview actions"
    aria-haspopup="menu"
    aria-expanded={!!menu}
    onClick={(event) => {
      const rect = event.currentTarget.getBoundingClientRect()
      openMenu(rect.right, rect.bottom + 4)
    }}
  ><Icon name="moreH" size={16} /></button>
  if (!menu) return trigger
  const sourceTitle = menu.video
    ? `Open the exact base asset at ${Math.round(menu.video.srcMs)}ms in Source Monitor`
    : 'No unambiguous base video is under the playhead'
  return <>
    {trigger}
    <ContextMenuFrame x={menu.x} y={menu.y} menuId="data-cut-preview-menu" backdropId="data-cut-preview-ctx-backdrop" onClose={() => setMenu(null)}>
      <span className="tl-ctx__label" aria-hidden="true">Preview · {menu.atMs}ms</span>
      <Item action="preview-open-source" disabled={!menu.video} title={sourceTitle} onClick={() => {
        if (!menu.video) return
        const source = menu.video
        setMenu(null)
        // Let the native menu click finish after its backdrop unmounts. Mounting
        // the blocking Source Monitor in the same pointer dispatch can turn the
        // release into a click-through dismissal on desktop WebViews.
        requestAnimationFrame(() => {
          document.dispatchEvent(new CustomEvent('cut:open-source-monitor', { detail: { asset: source.assetId, at_ms: source.srcMs } }))
        })
      }}><Icon name="screenPlay" size={14} /> Open base source</Item>
      <Item action="preview-seek-clip-start" disabled={!menu.video} title={menu.video ? 'Seek to the exact base clip start' : 'No base clip is under the playhead'} onClick={() => {
        if (!menu.video) return
        onSeek(menu.video.startMs)
        setMenu(null)
      }}><Icon name="marker" size={14} /> Seek to base clip start</Item>
      <Item action="preview-add-marker" disabled={!hasProject} title={hasProject ? 'Add a marker at the exact preview time' : 'Open a project before adding a marker'} onClick={() => {
        if (!hasProject) return
        void runUserVerb('edit.add_marker', { at_ms: menu.atMs, label: `m @ ${menu.atMs}ms`, rationale: 'add marker from Preview menu' }, 'Could not add a marker.')
        setMenu(null)
      }}><Icon name="marker" size={14} /> Add marker here</Item>
    </ContextMenuFrame>
  </>
}
