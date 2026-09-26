import { cursorCorrelationLabel, type CursorCorrelation, type StudioRawStreams } from './studioTypes'
import './recordingResultPanel.css'

interface RecordingResultPanelProps {
  outcome: 'saved' | 'partial' | 'recovery'
  raw: boolean
  message: string
  duration: string
  rawSaved: boolean
  polishedClipSaved: boolean
  hasMic: boolean
  hasSystem: boolean
  streams: StudioRawStreams | null
  cursorCorrelation: CursorCorrelation | null
  cadence: string
  hotkeyScope: string
  exportFormat: 'mp4' | 'gif'
  exportRunning: boolean
  rawCopyRunning: boolean
  exportNote: string
  rawCopyNote: string
  recoveryAction: 'retry_stop' | 'check_status' | null
  onFormat: (format: 'mp4' | 'gif') => void
  onExport: () => void
  onCancelExport: () => void
  onSaveRawCopy: () => void
  onCancelRawCopy: () => void
  onAddRawToTimeline: () => void
  onOpenEdit?: () => void
  onNewRecording: () => void
  onOpenOutputSettings?: () => void
  onRecovery: () => void
}

/** Post-Stop facts and actions. Paths stay in the native owner, not ordinary DOM. */
export function RecordingResultPanel({
  outcome, raw, message, duration, rawSaved, polishedClipSaved, hasMic, hasSystem,
  streams, cursorCorrelation, cadence, hotkeyScope, exportFormat, exportRunning,
  rawCopyRunning, exportNote, rawCopyNote, recoveryAction, onFormat, onExport,
  onCancelExport, onSaveRawCopy, onCancelRawCopy, onAddRawToTimeline,
  onOpenEdit, onNewRecording, onOpenOutputSettings, onRecovery,
}: RecordingResultPanelProps) {
  const saved = outcome === 'saved'
  const title = saved
    ? raw ? 'Unedited raw MP4 saved' : 'Editable clip added to current project'
    : outcome === 'recovery' ? 'Stop needs attention' : 'Recording needs attention'
  const streamNames = streams
    ? [streams.screen && 'Screen', streams.camera && 'Camera', streams.mic && 'Microphone',
      streams.system && 'Computer sound', streams.studio_events && 'Studio changes'].filter(Boolean).join(' · ')
    : 'Stream details unavailable'

  return (
    <section className="rec-result" data-cut-rec-result={outcome} role="status">
      <div className={`rec-result__hero rec-result__hero--${outcome}`}>
        <span className="rec-result__icon" aria-hidden="true">{saved ? '✓' : '!'}</span>
        <h2>{title}</h2>
        <p>{message}</p>
      </div>
      <div className="rec-result__facts" aria-label="Recording result">
        <span><strong>Duration</strong> · about {duration}</span>
        <span><strong>Raw MP4</strong> · {rawSaved ? 'saved in the default export folder' : 'not confirmed saved'}</span>
        <span><strong>Current project</strong> · {polishedClipSaved ? 'polished editable clip added' : raw ? 'unchanged until Add to timeline' : 'clip not confirmed'}</span>
      </div>

      {outcome === 'recovery' && (
        <button type="button" className="rec__export-btn" data-cut-action={recoveryAction === 'retry_stop' ? 'record-retry-stop' : 'record-check-capture-status'} onClick={onRecovery}>
          {recoveryAction === 'retry_stop' ? 'Retry Stop for this capture' : 'Check capture status'}
        </button>
      )}

      {rawSaved && (
        <div className="rec-result__export" data-cut-rec-export>
          <h2>Export</h2>
          <p>{raw
            ? 'The unchanged MP4 is already saved. Add it to the timeline or save a copy elsewhere.'
            : polishedClipSaved
              ? 'The original MP4 is saved. Export the polished clip when it is ready, or save a copy of the original.'
              : 'The original MP4 is saved. The polished clip is not confirmed; you can save a copy of the original.'}</p>
          <div className="rec-result__actions">
            {raw && <button type="button" className="rec__export-btn" data-cut-action="record-add-raw" onClick={onAddRawToTimeline}>Add to timeline</button>}
            {!raw && polishedClipSaved && (
              <>
                <div className="rec__seg" role="group" aria-label="Export format">
                  {(['mp4', 'gif'] as const).map((format) => (
                    <button key={format} type="button" className={`rec__seg-btn${exportFormat === format ? ' rec__seg-btn--on' : ''}`}
                      data-cut-rec-export-fmt={format} aria-pressed={exportFormat === format}
                      disabled={exportRunning} onClick={() => onFormat(format)}>{format.toUpperCase()}</button>
                  ))}
                </div>
                <button type="button" className="rec__export-btn" data-cut-action="record-export" disabled={exportRunning} onClick={onExport}>Export {exportFormat.toUpperCase()}…</button>
                {exportRunning && <button type="button" className="rec__export-btn" data-cut-action="record-export-cancel" onClick={onCancelExport}>Cancel export</button>}
              </>
            )}
            <button type="button" className="rec__export-btn" data-cut-action={raw ? 'record-save-copy' : 'record-save-raw-copy'} disabled={rawCopyRunning} onClick={onSaveRawCopy}>
              {raw ? 'Save a copy…' : 'Save raw MP4 copy…'}
            </button>
            {rawCopyRunning && <button type="button" className="rec__export-btn" data-cut-action="record-copy-cancel" onClick={onCancelRawCopy}>Cancel copy</button>}
            {onOpenOutputSettings && <button type="button" className="rec__export-btn" data-cut-action="record-output-settings" onClick={onOpenOutputSettings}>Export folder settings</button>}
          </div>
          {rawCopyNote && <p className="rec__export-note" data-cut-rec-copy-note role="status">{rawCopyNote}</p>}
          {exportNote && <p className="rec__export-note" data-cut-rec-export-note role="status">{exportNote}</p>}
        </div>
      )}

      <details className="rec-result__details" data-cut-rec-recording-details>
        <summary>Recording details</summary>
        <div>
          <p><strong>Streams:</strong> {streamNames}. {hasMic && hasSystem ? 'Microphone and computer sound were captured.' : hasMic ? 'Microphone sound was captured.' : hasSystem ? 'Computer sound was captured.' : 'No audio source was confirmed.'}</p>
          <p><strong>Pointer:</strong> {cursorCorrelationLabel(cursorCorrelation)}.</p>
          <p><strong>Frame timing:</strong> {cadence}.</p>
          <p><strong>Shortcut:</strong> {hotkeyScope}.</p>
        </div>
      </details>
      {outcome !== 'recovery' && (
        <div className="rec-result__next">
          {polishedClipSaved && onOpenEdit && <button type="button" className="rec__export-btn" data-cut-action="record-open-edit" onClick={onOpenEdit}>Open clip in Edit</button>}
          <button type="button" className="rec__export-btn" data-cut-action="record-new-take" onClick={onNewRecording}>New recording</button>
        </div>
      )}
    </section>
  )
}
