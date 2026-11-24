import type { MutableRefObject } from 'react'
import type { UiCommandResult } from '../lib/events'
import { blockingOverlayConflict } from './uiBlockingOverlay'
import { UI_COMMAND_CONFIRM_TIMEOUT_MS, waitForCommittedState } from './uiCommandCommit'
import type { UiObservableState } from './uiControlState'
import { uiSurface } from './uiSurfaceRegistry'

type UiCommandError = NonNullable<UiCommandResult['error']>

function queryTarget(selector: string): HTMLElement | null {
  try {
    return document.querySelector<HTMLElement>(selector)
  } catch {
    return null
  }
}

export async function handleUiOpenCommand({
  args,
  before,
  stateRef,
  openSurface,
  answer,
  reject,
}: {
  args: Record<string, unknown>
  before: UiObservableState
  stateRef: MutableRefObject<UiObservableState>
  openSurface: (id: string) => boolean
  answer: (
    requested: Record<string, unknown>,
    state: UiObservableState,
    extra?: Pick<UiCommandResult, 'surface' | 'selector'>,
  ) => void
  reject: (
    requested: Record<string, unknown>,
    error: UiCommandError,
    extra?: Pick<UiCommandResult, 'surface' | 'selector'>,
  ) => void
}): Promise<void> {
  const panel = typeof args.panel === 'string' ? args.panel : ''
  const requested = { panel }
  const definition = uiSurface(panel)
  if (!definition?.action) {
    reject(requested, { code: 'not_found', message: `surface '${panel || '(empty)'}' is not agent-openable` }, { surface: panel || undefined })
    return
  }
  const blocking = blockingOverlayConflict(panel, definition.action, before)
  if (blocking) {
    reject(
      requested,
      { code: 'conflict', message: blocking.message, suggested_action: blocking.suggested_action },
      { surface: panel, selector: blocking.selector ?? definition.selector },
    )
    return
  }
  if (definition.action.kind !== 'focus' && before.open_surface_ids.includes(panel)) {
    reject(requested, { code: 'conflict', message: `surface '${panel}' is already open` }, { surface: panel, selector: definition.selector })
    return
  }
  if (!openSurface(panel)) {
    reject(requested, { code: 'not_found', message: `surface '${panel}' has no available opener` }, { surface: panel, selector: definition.selector })
    return
  }
  const rightTab = definition.action.kind === 'right' ? definition.action.tab : null
  const state = await waitForCommittedState(
    stateRef,
    before.state_revision,
    (current) => {
      if (!current.open_surface_ids.includes(panel)) return false
      const target = queryTarget(definition.selector)
      if (!target) return false
      if (rightTab) {
        return current.right.body_status === 'loaded'
          && !!queryTarget(`[data-cut-right-body-loaded="${rightTab}"]`)
      }
      if (definition.action?.kind !== 'focus') return true
      return document.activeElement === target || target.contains(document.activeElement)
    },
    UI_COMMAND_CONFIRM_TIMEOUT_MS,
    rightTab
      ? (current) => current.right.active_tab === rightTab
        && (current.right.body_status === 'blocked' || current.right.body_status === 'failed')
      : undefined,
  )
  if (state) {
    answer(requested, state, { surface: panel, selector: definition.selector })
    return
  }
  const current = stateRef.current
  const blocked = rightTab !== null
    && current.right.active_tab === rightTab
    && (current.right.body_status === 'blocked' || current.right.body_status === 'failed')
  reject(requested, {
    code: 'conflict',
    message: blocked
      ? `surface '${panel}' body is ${current.right.body_status}; use the recovery control to load it`
      : `surface '${panel}' was not confirmed in time`,
    suggested_action: blocked
      ? undefined
      : `inspect ui.state and verify '${panel}' is not obstructed, then retry ui.open`,
  }, { surface: panel, selector: blocked ? current.right.recovery_selector ?? definition.selector : definition.selector })
}
