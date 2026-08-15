import { useEffect, useRef, type KeyboardEvent as ReactKeyboardEvent, type MouseEvent as ReactMouseEvent, type ReactNode } from 'react'

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

function revealExpandedMenuGroup(menu: HTMLDivElement, x: number, y: number): void {
  // A disclosure can make a previously short, bottom-clamped menu grow to its
  // viewport maximum. Re-clamp the fixed shell before scrolling its content;
  // otherwise the new lower half can remain physically below the WebView even
  // though the menu itself owns an overflow scrollport.
  clampMenu(menu, x, y)
  const triggers = [...menu.querySelectorAll<HTMLButtonElement>('button[aria-expanded="true"]')]
  const group = triggers.at(-1)?.nextElementSibling
  if (!(group instanceof HTMLElement)) return
  const menuRect = menu.getBoundingClientRect()
  const groupRect = group.getBoundingClientRect()
  const bottomOverflow = groupRect.bottom - (menuRect.bottom - 4)
  if (bottomOverflow > 0) menu.scrollTop += bottomOverflow
  const topOverflow = (menuRect.top + 4) - group.getBoundingClientRect().top
  if (topOverflow > 0) menu.scrollTop -= topOverflow
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

  useEffect(() => {
    const menu = menuRef.current
    if (!menu) return
    // Descendant disclosure state (Audio, Speed, Replace, Library Move) does
    // not re-render ContextMenuFrame itself. Observe that owned menu subtree so
    // WebView2 and WKWebView both re-clamp before automation or a person can
    // target the newly inserted children.
    const observer = new MutationObserver(() => revealExpandedMenuGroup(menu, x, y))
    observer.observe(menu, {
      childList: true,
      subtree: true,
      attributes: true,
      attributeFilter: ['aria-expanded'],
    })
    return () => observer.disconnect()
  }, [x, y])

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

  const onMenuClick = (event: ReactMouseEvent<HTMLDivElement>) => {
    const trigger = (event.target as HTMLElement).closest<HTMLButtonElement>('button[aria-expanded]')
    if (!trigger || !menuRef.current?.contains(trigger)) return
    // Inline groups grow below their disclosure row. Once React commits the
    // expanded children, reveal that group inside the menu's own scrollport so
    // the newly opened actions are immediately clickable at laptop heights.
    requestAnimationFrame(() => {
      if (trigger.getAttribute('aria-expanded') !== 'true') return
      const menu = menuRef.current
      if (menu) revealExpandedMenuGroup(menu, x, y)
    })
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
        onClick={onMenuClick}
        onKeyDown={onMenuKeyDown}
      >
        {children}
      </div>
    </>
  )
}
