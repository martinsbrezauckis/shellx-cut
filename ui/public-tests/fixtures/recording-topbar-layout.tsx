import { createRoot } from 'react-dom/client'
import RecordingTopBar from '../../src/topbar/RecordingTopBar'
import { recordingWorkspaceAdmission, type RecordingWorkspaceAdmissionPhase } from '../../src/panels/Record/recordingWorkspaceAdmission'
import type { Project } from '../../src/lib/client'
import '../../src/theme.css'
import '../../src/topbar/topbar.css'

const project = {
  name: 'Recording meeting with a deliberately long project name',
  settings: { width: 1920, height: 1080, fps: 30 },
  assets: {}, tracks: [], markers: [], caption_styles: {}, checkpoints: [],
} as unknown as Project
// `?phase=` picks which refusal reason the bar shows; the default is the long-running one.
const phase = (new URLSearchParams(location.search).get('phase') ?? 'finalizing') as RecordingWorkspaceAdmissionPhase
const admission = recordingWorkspaceAdmission(phase)
createRoot(document.getElementById('root')!).render(
  <div className="app" data-cut-app-root>
    <RecordingTopBar project={project} doctor={null} manualOpen={false}
      onMode={() => {}} backDisabled={admission.blocked} backReason={admission.reason}
      onOpenSetup={() => { document.body.dataset.settingsOpened = 'true' }}
      onOpenManual={() => { document.body.dataset.manualOpened = 'true' }} />
  </div>,
)
