import { useCallback, useEffect, useRef } from 'react'
import {
  CUT_MANUAL_PROTOCOL,
  isEmbeddedManualFrontend,
  isManualFrontendMessage,
  isSameManualWindow,
  manualPostMessageTargetOrigin,
  type ManualFrontendMessage,
} from './protocol'
import { manualFeatureForElement, manualFeatureTarget, type ManualFeatureTarget } from './targets'

type ManualRevealRequest = Extract<ManualFrontendMessage, { type: 'reveal' }>
type ManualRevealResult = Extract<ManualFrontendMessage, { type: 'reveal-result' }>

interface ManualFrontendBridgeOptions {
  /** Opens the same registered surface used by ui.open and the command palette. */
  openSurface: (id: string) => boolean
}

/**
 * Opens only controls whose sole effect is to expose a menu. Entries with an
 * action-shaped opener must be reviewed here before a manual reveal may click
 * them: a documentation request must never render, export, mutate, or start a
 * job merely to show where the button lives.
 */
const PASSIVE_REVEAL_OPENERS = new Set([
  '[data-cut-render-opts]',
  '[data-cut-export-btn]',
])

function nextFrame(): Promise<void> {
  return new Promise((resolve) => window.requestAnimationFrame(() => resolve()))
}

function waitForElement(selector: string, timeoutMs = 1_500): Promise<HTMLElement | null> {
  const current = document.querySelector<HTMLElement>(selector)
  if (current) return Promise.resolve(current)
  return new Promise((resolve) => {
    const observer = new MutationObserver(() => {
      const element = document.querySelector<HTMLElement>(selector)
      if (!element) return
      window.clearTimeout(timeout)
      observer.disconnect()
      resolve(element)
    })
    const timeout = window.setTimeout(() => {
      observer.disconnect()
      resolve(document.querySelector<HTMLElement>(selector))
    }, timeoutMs)
    observer.observe(document.documentElement, { childList: true, subtree: true, attributes: true })
  })
}

function localRevealRequest(value: unknown): ManualRevealRequest | null {
  if (isManualFrontendMessage(value) && value.type === 'reveal') return value
  if (!value || typeof value !== 'object') return null
  const detail = value as Record<string, unknown>
  if (typeof detail.featureId !== 'string' || typeof detail.requestId !== 'string') return null
  return {
    schema: CUT_MANUAL_PROTOCOL,
    type: 'reveal',
    featureId: detail.featureId,
    requestId: detail.requestId,
  }
}

function targetLabel(target: ManualFeatureTarget): string {
  return target.featureId.replace(/^cut\./, '').replaceAll('.', ' · ')
}

/**
 * Connects an embedded manual to this real editor instance. Incoming requests
 * only open registered surfaces or passive menus, then reuse the app's native
 * local-highlight event. The bridge deliberately owns no project mutation.
 */
export function useManualFrontendBridge({ openSurface }: ManualFrontendBridgeOptions): void {
  const suppressSelectionRef = useRef(false)

  const announce = useCallback((message: ManualFrontendMessage, localEvent: string) => {
    document.dispatchEvent(new CustomEvent(localEvent, { detail: message }))
    if (window.parent !== window) window.parent.postMessage(message, manualPostMessageTargetOrigin())
  }, [])

  const reveal = useCallback(async (request: ManualRevealRequest): Promise<void> => {
    const target = manualFeatureTarget(request.featureId)
    const reply = (result: Omit<ManualRevealResult, 'schema' | 'type' | 'featureId' | 'requestId'>) => {
      announce({
        schema: CUT_MANUAL_PROTOCOL,
        type: 'reveal-result',
        featureId: request.featureId,
        requestId: request.requestId,
        ...result,
      }, 'cut:manual-reveal-result')
    }

    if (target.unavailable) {
      reply({ status: 'unavailable', message: target.unavailable })
      return
    }

    let surfaceOpened = false
    if (target.surface) surfaceOpened = openSurface(target.surface)

    let highlightSelector = target.selector
    let openerWasInvoked = false
    let actionOpenerWasSkipped = false
    if (target.openSelector) {
      const opener = document.querySelector<HTMLElement>(target.openSelector)
      if (!opener) {
        if (!target.surface) {
          reply({ status: 'missing', message: 'The control is not available in this editor view.' })
          return
        }
      } else if (PASSIVE_REVEAL_OPENERS.has(target.openSelector) && !opener.matches(':disabled')) {
        // `.click()` is intentionally limited to the passive allowlist above.
        suppressSelectionRef.current = true
        try {
          opener.click()
          openerWasInvoked = true
        } finally {
          suppressSelectionRef.current = false
        }
      } else if (!PASSIVE_REVEAL_OPENERS.has(target.openSelector)) {
        // Storyboard and any future action-shaped opener remains a visible
        // pointer only. Do not make an undocumented action safe by accident.
        highlightSelector = target.openSelector
        actionOpenerWasSkipped = true
      }
    }

    // Surface opening and menu toggles schedule React state. Give that work two
    // frames before judging selector availability or asking HighlightOverlay to
    // scroll to it.
    if (surfaceOpened || openerWasInvoked) {
      await nextFrame()
      await nextFrame()
    }

    const element = highlightSelector
      ? (surfaceOpened || openerWasInvoked
        ? await waitForElement(highlightSelector)
        : document.querySelector<HTMLElement>(highlightSelector))
      : null
    if (element && highlightSelector) {
      document.dispatchEvent(new CustomEvent('cut:local-highlight', {
        detail: {
          selector: highlightSelector,
          label: targetLabel(target),
          description: actionOpenerWasSkipped
            ? 'This action is highlighted but was not run from the manual.'
            : undefined,
          duration_ms: 0,
          scroll: true,
        },
      }))
      reply({
        status: actionOpenerWasSkipped ? 'surface-only' : 'shown',
        ...(actionOpenerWasSkipped
          ? { message: 'The action was highlighted without running it.' }
          : {}),
      })
      return
    }

    if (surfaceOpened) {
      reply({
        status: 'surface-only',
        message: 'The related editor surface opened, but this exact control is not available in the current state.',
      })
      return
    }

    reply({ status: 'missing', message: 'The documented control is not available in this editor view.' })
  }, [announce, openSurface])

  useEffect(() => {
    const mode = isEmbeddedManualFrontend() ? 'embedded' : 'local'
    announce({ schema: CUT_MANUAL_PROTOCOL, type: 'ready', mode }, 'cut:manual-ready')

    const onMessage = (event: MessageEvent<unknown>) => {
      if (!isSameManualWindow(event) || !isManualFrontendMessage(event.data) || event.data.type !== 'reveal') return
      void reveal(event.data)
    }
    const onLocalReveal = (event: Event) => {
      const request = localRevealRequest((event as CustomEvent<unknown>).detail)
      if (request) void reveal(request)
    }
    const onClick = (event: MouseEvent) => {
      if (suppressSelectionRef.current || !(event.target instanceof Element)) return
      const featureId = manualFeatureForElement(event.target)
      if (!featureId) return
      announce({ schema: CUT_MANUAL_PROTOCOL, type: 'selected', featureId }, 'cut:manual-selected')
    }

    window.addEventListener('message', onMessage)
    document.addEventListener('cut:manual-reveal', onLocalReveal)
    document.addEventListener('click', onClick, true)
    return () => {
      window.removeEventListener('message', onMessage)
      document.removeEventListener('cut:manual-reveal', onLocalReveal)
      document.removeEventListener('click', onClick, true)
    }
  }, [announce, reveal])
}
