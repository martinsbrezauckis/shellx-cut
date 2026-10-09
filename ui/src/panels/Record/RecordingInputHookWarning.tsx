import { recordingInputHookWarning, type RecordingInputHook } from './recordingInputHook'
import './recordingInputHookWarning.css'

export function RecordingInputHookWarning({ input, saved }: { input: RecordingInputHook; saved: boolean }) {
  const warning = recordingInputHookWarning(input, saved)
  return warning ? <span className="rec-input-warning" data-cut-rec-input-hook-warning="startup_failed" role="status">{warning}</span> : null
}
