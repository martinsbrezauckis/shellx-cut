import type { UiObservableState } from './uiControlState'
import { uiSurface, type UiSurfaceAction } from './uiSurfaceRegistry'

export interface BlockingOverlayConflict {
  message: string
  suggested_action: string
  selector?: string
}

function isNonBlockingDrawer(doc: Document, drawerId: string): boolean {
  if (drawerId === 'mask') return true
  if (drawerId === 'title') {
    const titleEl = doc.querySelector('[data-cut-title-open="true"]')
    return Boolean(titleEl && !titleEl.hasAttribute('data-cut-blocking-overlay'))
  }
  return false
}

function blockerLabel(el: Element): string {
  const aria = el.getAttribute('aria-label')?.trim()
  if (aria) return aria
  if (el.hasAttribute('data-cut-command-palette')) return 'Command palette'
  if (el.hasAttribute('data-cut-keymap-overlay')) return 'Keyboard shortcuts'
  const heading = el.querySelector('h1, h2, h3')?.textContent?.trim()
  return heading || 'blocking modal dialog'
}

function matchesSelectorRoot(el: Element, selector: string | undefined): boolean {
  if (!selector) return false
  return el.matches(selector) || Boolean(el.closest(selector))
}

/**
 * Diagnose when a blocking drawer or modal overlay currently makes background
 * editor surfaces inert/unreachable, naming the exact blocker and next step.
 * Drawer->drawer switches and Environment/Wizard auto-dismissal remain allowed
 * unless an independent modal overlay sits on top.
 */
export function blockingOverlayConflict(
  targetPanel: string,
  targetAction: UiSurfaceAction,
  before: UiObservableState,
): BlockingOverlayConflict | null {
  if (before.overlays.quit_question) {
    return {
      message: `cannot open '${targetPanel}' while the Quit confirmation dialog is open; confirm or cancel it and retry`,
      suggested_action: `confirm or cancel the open Quit confirmation dialog, then retry ui.open for '${targetPanel}'`,
      selector: '[data-cut-quit-question]',
    }
  }

  const doc = typeof document !== 'undefined' ? document : null
  const drawerId = before.overlays.drawer
  const drawerBlocks = Boolean(drawerId && (!doc || !isNonBlockingDrawer(doc, drawerId)))
  const drawerSelector = drawerId ? uiSurface(drawerId)?.selector : undefined

  if (doc?.body) {
    for (const modal of Array.from(doc.body.querySelectorAll('[data-cut-blocking-overlay]'))) {
      if (modal.matches('[data-cut-environment]') || modal.closest('[data-cut-environment]')) continue
      if (modal.matches('[data-cut-wizard-open="true"]') || modal.closest('[data-cut-wizard-open="true"]')) continue
      if (drawerId && matchesSelectorRoot(modal, drawerSelector)) continue
      const label = blockerLabel(modal)
      return {
        message: `cannot open '${targetPanel}' while blocking overlay '${label}' is open; close it (press Escape or click Close) and retry`,
        suggested_action: `close the blocking '${label}' overlay with Escape or its Close control, then retry ui.open for '${targetPanel}'`,
      }
    }
  }

  if (drawerId && drawerBlocks && targetAction.kind !== 'drawer') {
    const label = uiSurface(drawerId)?.label ?? drawerId
    return {
      message: `cannot open '${targetPanel}' while blocking drawer '${drawerId}' (${label}) is open; close '${drawerId}' (press Escape or click Close) and retry`,
      suggested_action: `close the blocking drawer '${drawerId}' (${label}) with Escape or its Close button, then retry ui.open for '${targetPanel}'`,
      selector: drawerSelector,
    }
  }

  return null
}
