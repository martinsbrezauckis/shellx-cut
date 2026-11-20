import { createRoot } from 'react-dom/client'
import { VoiceoverTrackControl } from '../../src/panels/Timeline/VoiceoverTrackControl'
import '../../src/theme.css'
import '../../src/panels/Timeline/timeline.css'

// The control as the timeline mounts it: inside a sticky track-header rail at the
// left edge of a scroller that clips whatever hangs outside it.
const bottom = new URLSearchParams(location.search).has('bottom')
createRoot(document.getElementById('root')!).render(
  <div style={{ display: 'flex', flexDirection: 'column', width: '100vw', height: '100vh', overflow: 'hidden' }}>
    {bottom && <div style={{ flex: 1, padding: 16 }}>Preview</div>}
    <div className="tl-root" style={{ flex: 'none', height: bottom ? 180 : '100vh' }}>
      <div className="tl-scroll" data-test-timeline-scroll>
        <div className="tl-inner" style={{ width: '150vw' }}>
          {bottom && <div style={{ height: 116 }}>Video track</div>}
          <div className="tl-track-head" data-cut-track-kind="audio" style={{ width: 216, height: 40 }}>
            <span>Audio 1</span>
            <div className="tl-track-actions">
              <VoiceoverTrackControl trackId="audio-1" locked={false} projectRevision="revision-1" playheadMs={0} exportRange={null} />
            </div>
          </div>
          {bottom && <div style={{ height: 200 }} />}
        </div>
      </div>
    </div>
    {bottom && <div style={{ height: 64, flex: 'none', position: 'relative', zIndex: 20, background: 'var(--surface-2)', padding: 12 }}>Timeline footer</div>}
  </div>,
)
