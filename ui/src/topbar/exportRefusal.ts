import type { CutError } from '../lib/client'
import { humanFailureMessage, publishUserActionMessage } from '../lib/userActionFeedback'

type ExportError = Pick<CutError, 'code' | 'message'>
  & Partial<Pick<CutError, 'cause' | 'suggested_action' | 'requirement'>>

let deliverySession = 0
export function setDeliveryFeedbackSession(session: number): void { deliverySession = session }
export function deliveryFeedbackSession(): number { return deliverySession }

export const RECORDING_EXPORT_REFUSED = 'cut:recording-export-refused'

/** All human delivery entry points share this feedback, including queue admission. */
export function reportExportRefusal(error: ExportError | null | undefined, source: string): void {
  publishUserActionMessage(exportRefusalMessage(error), { source, code: error?.code })
  if (isRecordingExportRefusal(error) && typeof document !== 'undefined') {
    document.dispatchEvent(new CustomEvent(RECORDING_EXPORT_REFUSED))
  }
}

export function isHumanDeliveryVerb(name: string): boolean {
  return name.startsWith('export.') || name === 'screen_record.export' || name === 'jobs.retry'
    || ['render.final', 'render.queue', 'render.reframe', 'render.direct', 'render.qc', 'render.bundle'].includes(name)
}

// guard_export currently exposes invalid_args and this phrase, but no dedicated code.
export function isRecordingExportRefusal(error?: ExportError | null): boolean {
  return error?.code === 'invalid_args' && error.message.includes('Update video first')
}

export function exportRefusalMessage(error?: ExportError | null): string {
  if (isRecordingExportRefusal(error) || error?.message?.includes('This recording has unapplied zoom changes')) {
    return 'Export did not start: this recording has zoom changes that are not in the video yet. Press Update video, then export again.'
  }
  if (error?.code === 'conflict' && /^output(?:\s+folder)?\s+is\s+read-only\.?$/i.test(error.message.trim())) {
    return 'Export failed: the export folder is read-only. Choose a writable export folder, then try again.'
  }
  return `Export failed: ${humanFailureMessage(error, 'Export did not start')}`
}
