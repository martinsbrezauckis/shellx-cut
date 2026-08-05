import {
  useCallback,
  useLayoutEffect,
  useRef,
  type KeyboardEvent as ReactKeyboardEvent,
  type MouseEvent as ReactMouseEvent,
  type RefObject,
} from 'react'

const FOCUSABLE = [
  'a[href]',
  'button:not([disabled])',
  'input:not([disabled])',
  'select:not([disabled])',
  'textarea:not([disabled])',
  'summary',
  '[tabindex]:not([tabindex="-1"])',
].join(',')

const overlayStack: symbol[] = []

function isTopOverlay(token: symbol): boolean {
  return overlayStack[overlayStack.length - 1] === token
}

function syncOverlayState() {
  if (overlayStack.length > 0) {
    document.documentElement.dataset.cutBlockingOverlay = String(overlayStack.length)
  } else {
    delete document.documentElement.dataset.cutBlockingOverlay
  }
}

function focusableElements(root: HTMLElement): HTMLElement[] {
  return [...root.querySelectorAll<HTMLElement>(FOCUSABLE)].filter((element) => {
    const rect = element.getBoundingClientRect()
    if (rect.width <= 0 || rect.height <= 0 || element.getAttribute('aria-hidden') === 'true') {
      return false
    }

    // Chromium can report a stale/non-zero rect for descendants of a closed
    // <details>. Those controls are not keyboard reachable. Check the DOM
    // disclosure state explicitly so every embedded desktop engine agrees on
    // the focus-trap boundary.
    for (let ancestor = element.parentElement; ancestor && ancestor !== root; ancestor = ancestor.parentElement) {
      if (ancestor instanceof HTMLDetailsElement && !ancestor.open) {
        const summary = [...ancestor.children].find((child) => child instanceof HTMLElement && child.tagName === 'SUMMARY')
        if (!(summary instanceof HTMLElement) || !summary.contains(element)) return false
      }
    }
    return true
  })
}

function backgroundRegions(dialog: HTMLElement): HTMLElement[] {
  const appRoot = dialog.closest<HTMLElement>('[data-cut-app-root]')
    ?? document.querySelector<HTMLElement>('[data-cut-app-root]')
  if (!appRoot) return []
  if (!appRoot.contains(dialog)) return [appRoot]

  const regions = new Set<HTMLElement>()
  let branch: HTMLElement = dialog
  while (branch !== appRoot) {
    const parent = branch.parentElement
    if (!parent) break
    for (const sibling of parent.children) {
      if (
        sibling !== branch
        && sibling instanceof HTMLElement
        && !sibling.hasAttribute('data-cut-overlay-part')
      ) regions.add(sibling)
    }
    branch = parent
  }
  return [...regions]
}

interface BlockingOverlayContract<T extends HTMLElement> {
  dialogRef: RefObject<T | null>
  onDialogKeyDown: (event: ReactKeyboardEvent<T>) => void
  onScrimMouseDown: (event: ReactMouseEvent<HTMLElement>) => void
}

/**
 * Shared contract for app-blocking drawers and dialogs.
 *
 * The hook owns focus entry/return, Tab containment, top-overlay Escape and
 * click-away handling, and makes every background app region inert. It also
 * exposes a document-level marker used by global editor shortcut guards.
 */
export function useBlockingOverlay<T extends HTMLElement>(
  onClose: () => void,
  enabled = true,
): BlockingOverlayContract<T> {
  const dialogRef = useRef<T>(null)
  const tokenRef = useRef(Symbol('cut-blocking-overlay'))
  const openerRef = useRef<HTMLElement | null>(null)
  const closeRef = useRef(onClose)
  closeRef.current = onClose

  useLayoutEffect(() => {
    if (!enabled) return
    const dialog = dialogRef.current
    if (!dialog) return

    const token = tokenRef.current
    const active = document.activeElement
    if (active instanceof HTMLElement && active !== document.body && !dialog.contains(active)) {
      openerRef.current = active
    }
    overlayStack.push(token)
    syncOverlayState()

    const background = backgroundRegions(dialog).map((element) => ({
      element,
      inert: element.inert,
      ariaHidden: element.getAttribute('aria-hidden'),
    }))

    for (const entry of background) {
      entry.element.inert = true
      entry.element.setAttribute('aria-hidden', 'true')
    }

    const initialFocus = focusableElements(dialog)[0] ?? dialog
    initialFocus.focus({ preventScroll: true })

    return () => {
      const at = overlayStack.lastIndexOf(token)
      if (at >= 0) overlayStack.splice(at, 1)
      syncOverlayState()

      for (const entry of background) {
        entry.element.inert = entry.inert
        if (entry.ariaHidden === null) entry.element.removeAttribute('aria-hidden')
        else entry.element.setAttribute('aria-hidden', entry.ariaHidden)
      }

      const opener = openerRef.current
      if (opener?.isConnected) {
        requestAnimationFrame(() => {
          if (overlayStack.length === 0 && !dialog.isConnected && opener.isConnected) {
            opener.focus({ preventScroll: true })
          }
        })
      }
    }
  }, [enabled])

  const onDialogKeyDown = useCallback((event: ReactKeyboardEvent<T>) => {
    if (!isTopOverlay(tokenRef.current)) return

    // A blocking overlay owns every key that originates inside it. Controls
    // still receive the event; propagation stops before editor-wide handlers.
    event.stopPropagation()
    if (event.key === 'Escape') {
      event.preventDefault()
      closeRef.current()
      return
    }
    if (event.key !== 'Tab') return

    const dialog = dialogRef.current
    if (!dialog) return
    const focusable = focusableElements(dialog)
    if (focusable.length === 0) {
      event.preventDefault()
      dialog.focus()
      return
    }

    const first = focusable[0]
    const last = focusable[focusable.length - 1]
    const focused = document.activeElement
    if (event.shiftKey && (focused === first || !dialog.contains(focused))) {
      event.preventDefault()
      last.focus()
    } else if (!event.shiftKey && (focused === last || !dialog.contains(focused))) {
      event.preventDefault()
      first.focus()
    }
  }, [])

  const onScrimMouseDown = useCallback((event: ReactMouseEvent<HTMLElement>) => {
    if (event.target !== event.currentTarget || !isTopOverlay(tokenRef.current)) return
    event.preventDefault()
    closeRef.current()
  }, [])

  return { dialogRef, onDialogKeyDown, onScrimMouseDown }
}
