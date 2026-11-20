import type { ActiveVideo } from './model'
import type { PosterChip } from './posterChip'

interface PreviewMonitorBadgesProps {
  showVideo: boolean
  video: ActiveVideo | null
  proxyBuilding: boolean
  overlaysDropped: number
  posterActive: boolean
  showSpinner: boolean
  /** What the still frame is; null when it needs no explanation. */
  posterChip: PosterChip | null
  /** Plain notice while the live picture leaves out titles/effects at the playhead. */
  livePreviewNotice: string | null
}

export default function PreviewMonitorBadges({
  showVideo,
  video,
  proxyBuilding,
  overlaysDropped,
  posterActive,
  showSpinner,
  posterChip,
  livePreviewNotice,
}: PreviewMonitorBadgesProps) {
  return (
    <>
      {showVideo && video?.kind === 'source' && (
        <div className="pv-chip pv-chip--source" data-cut-source-chip>
          <span className={proxyBuilding ? 'pv-chip-dot pv-chip-dot--live' : 'pv-chip-dot'} />{proxyBuilding ? 'SOURCE · building proxy…' : 'SOURCE'}
        </div>
      )}
      {showVideo && overlaysDropped > 0 && (
        <div className="pv-chip pv-chip--warn" data-cut-overlay-cap>
          +{overlaysDropped} layer{overlaysDropped > 1 ? 's' : ''} hidden
        </div>
      )}
      {livePreviewNotice && (
        <div className="pv-chip pv-chip--note pv-live-notice" role="status" data-cut-live-preview-notice>{livePreviewNotice}</div>
      )}
      {!showVideo && posterActive && showSpinner && <div className="pv-spinner" data-cut-spinner />}
      {!showVideo && posterActive && posterChip && (
        <div
          className={posterChip.note ? 'pv-chip pv-chip--note' : 'pv-chip'}
          title={posterChip.title}
          data-cut-proxy-chip
          data-cut-poster-chip={posterChip.kind}
        >
          <span className={proxyBuilding ? 'pv-chip-dot pv-chip-dot--live' : 'pv-chip-dot'} />{posterChip.label}
        </div>
      )}
    </>
  )
}
