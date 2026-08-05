import { Icon } from '../icons'
import { useBlockingOverlay } from '../components/overlay/useBlockingOverlay'
import './otio-import.css'

export interface OtioTrackPreview {
  name: string
  kind: 'video' | 'audio' | string
  clips: number
  gaps: number
  duration_ms: number
}

export interface OtioImportPreview {
  status: 'preview'
  path: string
  source_hash: string
  name: string
  tracks: OtioTrackPreview[]
  track_count: number
  clips: number
  gaps: number
  media_references: number
  media_available: number
  media_missing: number
  missing_clips: number
  missing_sources: string[]
  source_format?: { width: number; height: number; fps: number } | null
  format_policy: 'preserve_project'
}

interface OtioImportModalProps {
  preview: OtioImportPreview
  busy: boolean
  error: string | null
  onCancel: () => void
  onConfirm: () => void
}

function fileTail(path: string): string {
  return path.split(/[\\/]/).filter(Boolean).pop() ?? path
}

function duration(ms: number): string {
  const seconds = Math.max(0, Math.round(ms / 1000))
  const minutes = Math.floor(seconds / 60)
  return `${minutes}:${String(seconds % 60).padStart(2, '0')}`
}

export default function OtioImportModal({ preview, busy, error, onCancel, onConfirm }: OtioImportModalProps) {
  const close = () => { if (!busy) onCancel() }
  const overlay = useBlockingOverlay<HTMLElement>(close)
  const format = preview.source_format
    ? `${preview.source_format.width}x${preview.source_format.height} @ ${preview.source_format.fps} fps`
    : 'Not declared'
  return (
    <div className="otio-overlay" data-cut-otio-import onMouseDown={overlay.onScrimMouseDown}>
      <section
        ref={overlay.dialogRef}
        className="otio-modal"
        role="dialog"
        aria-modal="true"
        aria-label="Import timeline preview"
        data-cut-blocking-overlay
        tabIndex={-1}
        onKeyDown={overlay.onDialogKeyDown}
        onMouseDown={(event) => event.stopPropagation()}
      >
        <header className="otio-head">
          <span className="otio-title"><Icon name="import" size={16} tone="brand" /> Import timeline</span>
          <button className="otio-close" data-cut-otio-close onClick={onCancel} disabled={busy} aria-label="Close"><Icon name="close" size={16} /></button>
        </header>
        <div className="otio-body">
          <div className="otio-source">
            <strong title={preview.path}>{fileTail(preview.path)}</strong>
            {preview.name && <span>{preview.name}</span>}
          </div>
          <dl className="otio-summary">
            <div><dt>Tracks</dt><dd>{preview.track_count}</dd></div>
            <div><dt>Clips</dt><dd>{preview.clips}</dd></div>
            <div><dt>Gaps</dt><dd>{preview.gaps}</dd></div>
            <div data-state={preview.media_missing > 0 ? 'warning' : 'ready'}>
              <dt>Media</dt><dd>{preview.media_available}/{preview.media_references}</dd>
            </div>
          </dl>
          <div className="otio-track-list" data-cut-otio-track-list>
            {preview.tracks.map((track, index) => (
              <div className="otio-track" key={`${track.kind}-${track.name}-${index}`}>
                <Icon name={track.kind === 'audio' ? 'audioClip' : 'videoClip'} size={14} />
                <span className="otio-track-name">{track.name || `${track.kind} ${index + 1}`}</span>
                <span>{track.clips} clip{track.clips === 1 ? '' : 's'}</span>
                <span>{duration(track.duration_ms)}</span>
              </div>
            ))}
          </div>
          <div className="otio-format">
            <span>Source format</span><strong>{format}</strong>
            <small>Current project format stays unchanged</small>
          </div>
          {preview.media_missing > 0 && (
            <div className="otio-warning" data-cut-otio-missing>
              <Icon name="warning" size={14} />
              <span>{preview.missing_clips} offline clip{preview.missing_clips === 1 ? '' : 's'} will remain as timed gaps.</span>
              {preview.missing_sources.length > 0 && <small>{preview.missing_sources.join(', ')}</small>}
            </div>
          )}
          {error && <div className="otio-error" data-cut-otio-error><Icon name="error" size={14} /> {error}</div>}
        </div>
        <footer className="otio-actions">
          <span>One undoable timeline operation</span>
          <button className="otio-btn" data-cut-otio-cancel onClick={onCancel} disabled={busy}>Cancel</button>
          <button className="otio-btn otio-btn--primary" data-cut-otio-confirm onClick={onConfirm} disabled={busy}>
            {busy ? <><Icon name="spinner" size={14} className="cut-spin" /> Importing</> : 'Replace timeline'}
          </button>
        </footer>
      </section>
    </div>
  )
}
