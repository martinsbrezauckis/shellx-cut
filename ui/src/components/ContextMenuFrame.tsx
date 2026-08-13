import { useEffect, useRef, type KeyboardEvent as ReactKeyboardEvent, type ReactNode } from 'react'

interface ContextMenuFrameProps {
  x: number
  y: number
  menuId: string
  backdropId: string
  onClose: () => void
  children: ReactNode
  className?: string
  backdropClassName?: string
  menuAttributes?: Record<string, string | undefined>
  ariaLabel?: string
}

function clampMenu(el: HTMLDivElement, x: number, y: number): void {
  const margin = 8
  const rect = el.getBoundingClientRect()
  el.style.left = `${Math.max(margin, Math.min(x, window.innerWidth - rect.width - margin))}px`
  el.style.top = `${Math.max(margin, Math.min(y, window.innerHeight - rect.height - margin))}px`
}

/** Shared fixed-position shell for small operational context menus.
 * The timeline menu CSS supplies the compact menu grammar application-wide. */
function menuItems(menu: HTMLDivElement): HTMLButtonElement[] {
  return [...menu.querySelectorAll<HTMLButtonElement>('[role="menuitem"]:not(:disabled)')]
}

/**
 * Shared fixed-position shell for operational context menus. It keeps every
 * owner inside the viewport and supplies the small, keyboard-complete menu
 * interaction that native right-click menus normally provide.
 */
export default function ContextMenuFrame({
  x,
  y,
  menuId,
  backdropId,
  onClose,
  children,
  className = 'tl-ctx',
  backdropClassName = 'tl-ctx-backdrop',
  menuAttributes,
  ariaLabel = 'Context menu',
}: ContextMenuFrameProps) {
  const menuRef = useRef<HTMLDivElement | null>(null)
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

  const onMenuKeyDown = (event: ReactKeyboardEvent<HTMLDivElement>) => {
    const target = event.target as HTMLElement
    if (target instanceof HTMLInputElement || target instanceof HTMLTextAreaElement || target instanceof HTMLSelectElement) return
    const menu = menuRef.current
    if (!menu) return
    const items = menuItems(menu)
    if (!items.length) return
    const activeIndex = items.indexOf(document.activeElement as HTMLButtonElement)
    const focus = (index: number) => items[(index + items.length) % items.length]?.focus()
    if (event.key === 'ArrowDown') {
      event.preventDefault()
      focus(activeIndex < 0 ? 0 : activeIndex + 1)
    } else if (event.key === 'ArrowUp') {
      event.preventDefault()
      focus(activeIndex < 0 ? items.length - 1 : activeIndex - 1)
    } else if (event.key === 'Home') {
      event.preventDefault()
      focus(0)
    } else if (event.key === 'End') {
      event.preventDefault()
      focus(items.length - 1)
    } else if ((event.key === 'Enter' || event.key === ' ') && target === menu) {
      event.preventDefault()
      items[0]?.click()
    }
  }

  return (
    <>
      <div
        className={backdropClassName}
        {...{ [backdropId]: '' }}
        onMouseDown={onClose}
        onContextMenu={(event) => { event.preventDefault(); onClose() }}
      />
      <div
        className={className}
        role="menu"
        tabIndex={-1}
        aria-label={ariaLabel}
        {...{ [menuId]: '' }}
        {...menuAttributes}
        style={{ left: x, top: y }}
        ref={(el) => {
          menuRef.current = el
          if (el) {
            clampMenu(el, x, y)
            el.focus({ preventScroll: true })
          }
        }}
        onKeyDown={onMenuKeyDown}
      >
        {children}
      </div>
    </>
  )
}
