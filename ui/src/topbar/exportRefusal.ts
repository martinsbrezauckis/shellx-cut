import type { CutError } from '../lib/client'
import { publishUserActionMessage } from '../lib/userActionFeedback'

let deliverySession = 0
export function setDeliveryFeedbackSession(session: number): void { deliverySession = session }
export function deliveryFeedbackSession(): number { return deliverySession }

export const RECORDING_EXPORT_REFUSED = 'cut:recording-export-refused'

/** All human delivery entry points share this feedback, including queue admission. */
export function reportExportRefusal(error: Pick<CutError, 'code' | 'message'> | null | undefined, source: string): void {
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
export function isRecordingExportRefusal(error?: Pick<CutError, 'code' | 'message'> | null): boolean {
  return error?.code === 'invalid_args' && error.message.includes('Update video first')
}

export function exportRefusalMessage(error?: Pick<CutError, 'code' | 'message'> | null): string {
  if (isRecordingExportRefusal(error)) {
    return 'Export did not start: this recording has zoom changes that are not in the video yet. Press Update video, then export again.'
  }
  return `export failed: ${error?.message ?? error?.code ?? 'unknown'}`
}
