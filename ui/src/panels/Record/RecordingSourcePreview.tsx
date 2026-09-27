import type {
  ScreenRecordSourcePreviewStatus,
} from '../../lib/clientResults'
import {
  type RecordingSourcePreviewPresentation,
  type RecordingSourcePreviewTarget,
} from './recordingNativeSourcePreview'

interface RecordingSourcePreviewProps {
  preview: RecordingSourcePreviewPresentation
  status: ScreenRecordSourcePreviewStatus
  target: RecordingSourcePreviewTarget | null
  statusError: string | null
  busy?: boolean
  onPause(): void
  onResume(): void
  onHide(): void
  onStop(): void
}

/**
 * Native preview status and lease controls. The start action lives in the
 * Studio plane; source selection remains exclusively in Screen & sound.
 */
export function RecordingSourcePreview({
  preview, status, target, statusError, busy = false, onPause, onResume, onHide, onStop,
}: RecordingSourcePreviewProps) {
  const active = status.state === 'starting' || status.state === 'ready'
  const paused = status.state === 'paused'
  const ownsPreview = active || paused

  return (
    <section className="rec-source-preview" data-cut-record-source-preview data-cut-record-source-preview-state={preview.state} aria-label="Source preview">
      <header className="rec-source-preview__head">
        <strong>Source preview</strong>
        <small role="status" data-cut-record-source-preview-detail>{preview.detail}</small>
      </header>
      {target && <p className="rec-source-preview__target" data-cut-record-source-preview-target>{target.label}</p>}
      {statusError && <p className="rec-source-preview__error" data-cut-record-source-preview-error role="status">{statusError}</p>}
      <div className="rec-source-preview__actions" aria-label="Source preview controls">
        {active && <button type="button" data-cut-action="record-source-preview-pause" onClick={onPause} disabled={busy}>Pause</button>}
        {paused && <button type="button" data-cut-action="record-source-preview-resume" onClick={onResume} disabled={busy}>Resume</button>}
        {ownsPreview && <>
          <button type="button" data-cut-action="record-source-preview-hide" onClick={onHide} disabled={busy}>Hide</button>
          <button type="button" data-cut-action="record-source-preview-stop" onClick={onStop} disabled={busy}>Stop</button>
        </>}
      </div>
    </section>
  )
}
