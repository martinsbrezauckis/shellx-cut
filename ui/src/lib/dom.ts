export function isEditableTarget(target: EventTarget | null): boolean {
  if (!(target instanceof HTMLElement)) return false
  return (
    target.tagName === 'INPUT' ||
    target.tagName === 'SELECT' ||
    target.tagName === 'TEXTAREA' ||
    target.isContentEditable
  )
}

/** True while a modal/drawer using the shared blocking-overlay contract is open. */
export function isBlockingOverlayActive(): boolean {
  return Number(document.documentElement.dataset.cutBlockingOverlay ?? 0) > 0
}

/** Common gate for editor-wide shortcuts. Overlay-local handlers remain active. */
export function shouldIgnoreGlobalShortcut(event: KeyboardEvent): boolean {
  return (
    event.defaultPrevented ||
    event.isComposing ||
    isBlockingOverlayActive() ||
    isEditableTarget(event.target)
  )
}
