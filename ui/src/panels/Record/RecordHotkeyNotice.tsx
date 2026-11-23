import { recordingNotice } from './recordingUiModel'
import type { RecordHotkeyCapability } from '../../lib/tauri'

/** A global F9 that the desktop shell tried and failed to register (another
 * program holds F9). The shell's own reason is shown as is; it already says
 * that F9 still works while Cut is focused. */
export function RecordHotkeyNotice({ capability }: { capability: RecordHotkeyCapability | null }) {
  if (capability?.state !== 'unavailable' || !capability.reason) return null
  return <p className="rec__hint" role="status" data-cut-rec-hotkey-notice>{recordingNotice(capability.reason, 'Recording shortcuts are unavailable. Use the buttons on the Record page.')}</p>
}

export function recordHotkeyScopeLabel(capability: RecordHotkeyCapability | null): string {
  if (capability?.scope === 'global') return 'Global F9 registered'
  if (capability?.state === 'unavailable') return 'Global F9 could not be registered; F9 works while Cut is focused'
  return 'F9 works while Cut is focused'
}
