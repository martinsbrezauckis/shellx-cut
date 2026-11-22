import { useEffect, type RefObject } from 'react'

/**
 * Calls `onDismiss` when `isOpen()` and a click or the keyboard focus lands
 * outside `ref`. Clicks on the scrollbar of a scrolling child count as inside,
 * because their target is that child. Same document-level `mousedown` check as
 * the other drop-downs (topbar menus, attachment picker); `focusin` adds the
 * keyboard route (Tab or Shift+Tab out of the list). `isOpen` reads the live
 * state (here the native `open` of a <details>), so there is no render in
 * between that could leave a just-opened list without its listeners.
 */
export function useDismissOnOutside(ref: RefObject<HTMLElement | null>, isOpen: () => boolean, onDismiss: () => void) {
  useEffect(() => {
    const outside = (event: Event) => isOpen() && !(event.target instanceof Node && ref.current?.contains(event.target))
    const onDown = (event: MouseEvent) => { if (outside(event)) onDismiss() }
    const onFocusIn = (event: FocusEvent) => { if (outside(event)) onDismiss() }
    // Capture phase: a handler that stops the event further down must not keep the list open.
    document.addEventListener('mousedown', onDown, true)
    document.addEventListener('focusin', onFocusIn, true)
    return () => {
      document.removeEventListener('mousedown', onDown, true)
      document.removeEventListener('focusin', onFocusIn, true)
    }
  }, [ref, isOpen, onDismiss])
}
