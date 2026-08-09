import { useEffect, type ReactNode } from 'react'

interface ContextMenuFrameProps {
  x: number
  y: number
  menuId: string
  backdropId: string
  onClose: () => void
  children: ReactNode
}

function clampMenu(el: HTMLDivElement, x: number, y: number): void {
  const margin = 8
  const rect = el.getBoundingClientRect()
  el.style.left = `${Math.max(margin, Math.min(x, window.innerWidth - rect.width - margin))}px`
  el.style.top = `${Math.max(margin, Math.min(y, window.innerHeight - rect.height - margin))}px`
}

/** Shared fixed-position shell for small operational context menus.
 * The timeline menu CSS supplies the compact menu grammar application-wide. */
export default function ContextMenuFrame({ x, y, menuId, backdropId, onClose, children }: ContextMenuFrameProps) {
  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (event.key !== 'Escape') return
      // Capture before the editor's global Escape handlers clear selection or
      // collapse rails; this menu owns its own dismissal without leaking a key.
      event.preventDefault()
      event.stopPropagation()
      onClose()
    }
    document.addEventListener('keydown', onKey, true)
    return () => document.removeEventListener('keydown', onKey, true)
  }, [onClose])

  return (
    <>
      <div
        className="tl-ctx-backdrop"
        {...{ [backdropId]: '' }}
        onMouseDown={onClose}
        onContextMenu={(event) => { event.preventDefault(); onClose() }}
      />
      <div
        className="tl-ctx"
        role="menu"
        tabIndex={-1}
        {...{ [menuId]: '' }}
        style={{ left: x, top: y }}
        ref={(el) => { if (el) { clampMenu(el, x, y); el.focus({ preventScroll: true }) } }}
      >
        {children}
      </div>
    </>
  )
}
