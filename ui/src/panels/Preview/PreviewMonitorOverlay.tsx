import type { Project } from '../../lib/client'
import type { ActiveVideo } from './model'
import PreviewComparison from './PreviewComparison'
import PreviewMonitorBadges from './PreviewMonitorBadges'
import type { PosterChip } from './posterChip'

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
  posterChip: PosterChip | null
  /** Plain notice while the live picture leaves out titles/effects at the playhead. */
  livePreviewNotice: string | null
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
  posterChip,
  livePreviewNotice,
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
        posterChip={posterChip}
        livePreviewNotice={livePreviewNotice}
      />
      <PreviewComparison project={project} playheadMs={playheadMs} revision={revision} onPause={onPause} />
    </>
  )
}
