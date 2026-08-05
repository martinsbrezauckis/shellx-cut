import type { ActiveVideo } from './model'

interface PreviewMonitorBadgesProps {
  showVideo: boolean
  video: ActiveVideo | null
  proxyBuilding: boolean
  overlaysDropped: number
  posterActive: boolean
  showSpinner: boolean
  composed: boolean
  liveComposedPlayback: boolean
}

export default function PreviewMonitorBadges({
  showVideo,
  video,
  proxyBuilding,
  overlaysDropped,
  posterActive,
  showSpinner,
  composed,
  liveComposedPlayback,
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
      {liveComposedPlayback && (
        <div className="pv-chip" data-cut-live-composite-chip>LIVE COMPOSITE</div>
      )}
      {!showVideo && posterActive && showSpinner && <div className="pv-spinner" data-cut-spinner />}
      {!showVideo && posterActive && (
        <div className="pv-chip" data-cut-proxy-chip>
          <span className={proxyBuilding ? 'pv-chip-dot pv-chip-dot--live' : 'pv-chip-dot'} />{proxyBuilding ? 'PROXY RENDERING…' : composed ? 'COMPOSED FRAME' : 'no preview'}
        </div>
      )}
    </>
  )
}
