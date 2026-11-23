import { recordingInputHookWarning, type RecordingInputHook } from './recordingInputHook'
import './recordingInputHookWarning.css'
import { recordingNotice } from './recordingUiModel'

export function RecordingInputHookWarning({ input, saved }: { input: RecordingInputHook; saved: boolean }) {
  const warning = recordingInputHookWarning(input, saved)
  return warning ? <span className="rec-input-warning" data-cut-rec-input-hook-warning="startup_failed" role="status">{recordingNotice(warning, 'Pointer and keyboard tracking could not start. The screen and sound can still be recorded. Check your system permissions before your next take.')}</span> : null
}
