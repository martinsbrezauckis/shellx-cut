import { timecode } from '../Timeline/layout'

export interface PreviewExactReviewState {
  url: string
  path: string
  rangeMs: [number, number]
}

interface PreviewExactReviewProps {
  exact: PreviewExactReviewState
  exactNote: string | null
  saveBusy: boolean
  onSave: () => void
  onExit: () => void
}

export default function PreviewExactReview({
  exact,
  exactNote,
  saveBusy,
  onSave,
  onExit,
}: PreviewExactReviewProps) {
  return (
    <div className="pv-exact" data-cut-exact>
      <video className="pv-exact-video" src={exact.url} controls autoPlay data-cut-exact-video />
      <div className="pv-exact-bar">
        <span className="pv-chip pv-exact-chip" data-cut-exact-chip>
          EXACT · {timecode(exact.rangeMs[0])}–{timecode(exact.rangeMs[1])}
        </span>
        {exactNote && <span className="pv-snap-note" data-cut-exact-note>{exactNote}</span>}
        <button
          className="pv-toggle pv-exact-save"
          data-cut-action="save-section"
          disabled={saveBusy}
          title="Save this rendered section to Assets as a reusable clip (shorts / highlights)"
          onClick={onSave}
        >
          {saveBusy ? 'Saving…' : 'Save to Assets'}
        </button>
        <button
          className="pv-toggle"
          data-cut-action="exit-exact"
          title="Return to the live composite preview"
          onClick={onExit}
        >
          ← Back to live
        </button>
      </div>
    </div>
  )
}
