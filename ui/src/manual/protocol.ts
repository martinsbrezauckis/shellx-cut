export const CUT_MANUAL_PROTOCOL = 'shellx-cut/manual-frontend@1' as const

export type ManualFrontendMode = 'embedded' | 'local'

export type ManualFrontendMessage =
  | { schema: typeof CUT_MANUAL_PROTOCOL; type: 'ready'; mode: ManualFrontendMode }
  | { schema: typeof CUT_MANUAL_PROTOCOL; type: 'reveal'; featureId: string; requestId: string }
  | { schema: typeof CUT_MANUAL_PROTOCOL; type: 'selected'; featureId: string }
  | {
      schema: typeof CUT_MANUAL_PROTOCOL
      type: 'reveal-result'
      featureId: string
      requestId: string
      status: 'shown' | 'surface-only' | 'unavailable' | 'missing'
      message?: string
    }

export function isManualFrontendMessage(value: unknown): value is ManualFrontendMessage {
  if (!value || typeof value !== 'object') return false
  const message = value as Record<string, unknown>
  if (message.schema !== CUT_MANUAL_PROTOCOL || typeof message.type !== 'string') return false
  if (message.type === 'ready') return message.mode === 'embedded' || message.mode === 'local'
  if (message.type === 'selected') return typeof message.featureId === 'string'
  if (message.type === 'reveal') {
    return typeof message.featureId === 'string' && typeof message.requestId === 'string'
  }
  if (message.type === 'reveal-result') {
    return typeof message.featureId === 'string'
      && typeof message.requestId === 'string'
      && ['shown', 'surface-only', 'unavailable', 'missing'].includes(String(message.status))
  }
  return false
}

export function isManualShell(): boolean {
  const requestedMode = new URLSearchParams(window.location.search).get('manual')
  if (requestedMode === 'embed') return false
  if (requestedMode === 'shell') return true
  return document.documentElement.dataset.cutManualShell === 'true'
}

export function isEmbeddedManualFrontend(): boolean {
  return new URLSearchParams(window.location.search).get('manual') === 'embed'
}

/**
 * Opaque desktop protocols (including Tauri's macOS custom protocol) expose
 * `window.location.origin` as the literal string `null`. `postMessage` does
 * not accept that value as a target origin, so use its required opaque-origin
 * target while retaining the source + received-origin checks below.
 */
export function manualPostMessageTargetOrigin(origin = window.location.origin): string {
  return origin === 'null' ? '*' : origin
}

export function isSameManualWindow(event: MessageEvent): boolean {
  if (event.source !== window.parent) return false
  if (event.origin === window.location.origin) return true
  return event.origin === 'null' && window.location.origin === 'null'
}
