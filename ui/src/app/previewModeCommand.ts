import type { MutableRefObject } from 'react'
import type { UiCommandResult } from '../lib/events'
import type { UiObservableState } from './uiControlState'
import { waitForCommittedState } from './uiCommandCommit'

type UiCommandError = NonNullable<UiCommandResult['error']>

/** ui.preview {composed}: the COMPOSED switch of the preview monitor for
 * agents. The Preview panel applies it (cut:set-preview-composed, remembered
 * like the person's own switch); the answer waits for a later ui-state
 * revision whose preview.mode reports the requested mode. */
export async function handlePreviewModeCommand({
  command, before, stateRef, answer, reject,
}: {
  command: { args: Record<string, unknown> }
  before: UiObservableState
  stateRef: MutableRefObject<UiObservableState>
  answer: (requested: Record<string, unknown>, state: UiObservableState) => void
  reject: (requested: Record<string, unknown>, error: UiCommandError) => void
}): Promise<void> {
  const raw = command.args.composed
  if (typeof raw !== 'boolean') {
    reject({}, { code: 'invalid_args', message: 'composed must be true or false' })
    return
  }
  const requested = { composed: raw }
  const mode = raw ? 'composed' : 'live'
  if (!before.preview?.mode) {
    reject(requested, { code: 'not_found', message: 'the preview monitor is not shown' })
    return
  }
  if (before.preview.mode === mode) {
    reject(requested, { code: 'conflict', message: `the preview is already in ${mode} mode` })
    return
  }
  document.dispatchEvent(new CustomEvent('cut:set-preview-composed', { detail: { composed: raw } }))
  const state = await waitForCommittedState(stateRef, before.state_revision, (current) => current.preview?.mode === mode)
  if (state) answer(requested, state)
  else reject(requested, { code: 'conflict', message: 'the preview mode did not commit before the confirmation deadline' })
}
