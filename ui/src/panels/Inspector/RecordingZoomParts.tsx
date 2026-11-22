import type { RecordingZoomListResult } from '../../lib/client'
import { zoomLabel } from './recordingZoomModel'

export function RecordingZoomList({ segments, selected, select }: {
  segments: RecordingZoomListResult['segments']
  selected: string
  select: (id: string) => void
}) {
  return (
    <div className="insp__group">
      <span className="insp__label" id="recording-zoom-list-label">Zoom segments</span>
      <div className="insp__zoom-list" role="listbox" aria-labelledby="recording-zoom-list-label">
        {segments.map((segment, index) => (
          <button
            key={segment.id}
            data-cut-recording-zoom-select={segment.id}
            type="button"
            role="option"
            aria-selected={selected === segment.id}
            tabIndex={selected === segment.id ? 0 : -1}
            className={`insp__btn insp__zoom-option${selected === segment.id ? ' insp__btn--on' : ''}`}
            onClick={() => select(segment.id)}
            onKeyDown={(event) => {
              const delta = event.key === 'ArrowDown' || event.key === 'ArrowRight' ? 1
                : event.key === 'ArrowUp' || event.key === 'ArrowLeft' ? -1 : 0
              if (!delta) return
              event.preventDefault()
              const next = Math.max(0, Math.min(segments.length - 1, index + delta))
              select(segments[next].id)
              const options = event.currentTarget.parentElement?.querySelectorAll<HTMLButtonElement>('[role="option"]')
              options?.[next]?.focus()
            }}
          >
            {zoomLabel(segment)}
          </button>
        ))}
      </div>
      {segments.length === 0 && <p className="insp__hint">No zooms yet. Add one at the playhead.</p>}
    </div>
  )
}

export function RecordingZoomRefresh({ stale, missingVideo = false, busy, job, progress, refresh }: {
  stale: boolean
  missingVideo?: boolean
  busy: boolean
  job: string | null
  progress: number
  refresh: () => void
}) {
  return (
    <>
      {(stale || missingVideo) && (
        <div role="status" className="insp__group">
          <p className="insp__hint">{missingVideo
            ? 'Video file missing. Update video rebuilds it.' : 'Changes not in the video yet'}</p>
          <button data-cut-action="recording-zoom-refresh" className="insp__btn" disabled={busy} onClick={refresh}>Update video</button>
        </div>
      )}
      {job && <label className="insp__label">Updating video <progress max={1} value={progress} /></label>}
    </>
  )
}

export function RecordingZoomRedo({ confirm, setConfirm, redo }: {
  confirm: boolean
  setConfirm: (confirm: boolean) => void
  redo: () => void
}) {
  return (
    <>
      <button data-cut-action="recording-zoom-redo" className="insp__btn" onClick={() => setConfirm(true)}>Redo automatic zooms</button>
      {confirm && (
        <div className="insp__group">
          <p className="insp__hint">Replace automatic zooms? Your own zooms stay.</p>
          <div className="insp__row">
            <button data-cut-action="recording-zoom-redo-confirm" className="insp__btn" onClick={() => { setConfirm(false); redo() }}>
              Redo automatic zooms
            </button>
            <button data-cut-action="recording-zoom-redo-cancel" className="insp__btn" onClick={() => setConfirm(false)}>Cancel</button>
          </div>
        </div>
      )}
    </>
  )
}
