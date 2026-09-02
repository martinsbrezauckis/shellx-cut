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
  onStart(): void
  onPause(): void
  onResume(): void
  onHide(): void
  onStop(): void
}

/**
 * A compact lifecycle control. Source selection remains exclusively in Screen
 * & sound; the native BMP itself is rendered only in the Studio plane.
 */
export function RecordingSourcePreview({
  preview, status, target, statusError, busy = false, onStart, onPause, onResume, onHide, onStop,
}: RecordingSourcePreviewProps) {
  const canStart = preview.available && target !== null && !busy
  const active = status.state === 'starting' || status.state === 'ready'
  const paused = status.state === 'paused'
  const ownsPreview = active || paused
  const startLabel = target?.source.kind === 'portal' ? 'Choose preview source…' : 'Preview selected source'

  return (
    <section className="rec-source-preview" data-cut-record-source-preview data-cut-record-source-preview-state={preview.state} aria-label="Source preview">
      <header className="rec-source-preview__head">
        <strong>Source preview</strong>
        <small role="status" data-cut-record-source-preview-detail>{preview.detail}</small>
      </header>
      {target && <p className="rec-source-preview__target" data-cut-record-source-preview-target>{target.label}</p>}
      {statusError && <p className="rec-source-preview__error" data-cut-record-source-preview-error role="status">{statusError}</p>}
      <div className="rec-source-preview__actions" aria-label="Source preview controls">
        <button type="button" data-cut-action="record-source-preview" onClick={onStart} disabled={!canStart}>{startLabel}</button>
        <button type="button" data-cut-action="record-source-preview-pause" onClick={onPause} disabled={!active || busy}>Pause</button>
        <button type="button" data-cut-action="record-source-preview-resume" onClick={onResume} disabled={!paused || busy}>Resume</button>
        <button type="button" data-cut-action="record-source-preview-hide" onClick={onHide} disabled={!ownsPreview || busy}>Hide</button>
        <button type="button" data-cut-action="record-source-preview-stop" onClick={onStop} disabled={!ownsPreview || busy}>Stop</button>
      </div>
    </section>
  )
}
