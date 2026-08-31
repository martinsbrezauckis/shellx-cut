import type { Project } from '../../lib/client'
import type { ActiveVideo } from './model'
import PreviewComparison from './PreviewComparison'
import PreviewMonitorBadges from './PreviewMonitorBadges'

interface PreviewMonitorOverlayProps {
  project: Project | null
  playheadMs: number
  revision?: string
  onPause: () => void
  showVideo: boolean
  video: ActiveVideo | null
  proxyBuilding: boolean
  overlaysDropped: number
  posterActive: boolean
  showSpinner: boolean
  composed: boolean
  liveComposedPlayback: boolean
}

/** Keeps monitor-only status and review affordances out of Preview/index.tsx. */
export default function PreviewMonitorOverlay({
  project,
  playheadMs,
  revision,
  onPause,
  showVideo,
  video,
  proxyBuilding,
  overlaysDropped,
  posterActive,
  showSpinner,
  composed,
  liveComposedPlayback,
}: PreviewMonitorOverlayProps) {
  return (
    <>
      <PreviewMonitorBadges
        showVideo={showVideo}
        video={video}
        proxyBuilding={proxyBuilding}
        overlaysDropped={overlaysDropped}
        posterActive={posterActive}
        showSpinner={showSpinner}
        composed={composed}
        liveComposedPlayback={liveComposedPlayback}
      />
      <PreviewComparison project={project} playheadMs={playheadMs} revision={revision} onPause={onPause} />
    </>
  )
}
