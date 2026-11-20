import { useState } from 'react'
import { createRoot } from 'react-dom/client'
import Record from '../../src/panels/Record'
import { RecordingSessionProvider } from '../../src/app/RecordingSessionContext'
import { RecordingDeliveryProvider } from '../../src/app/RecordingDeliveryContext'
import { VolumeAutomationProvider } from '../../src/app/VolumeAutomationContext'
import { useRecordingSession } from '../../src/app/useRecordingSession'
import type { Project } from '../../src/lib/client'

const project = (origin: string) => ({
  name: 'Same project name',
  project_identity: { project_name: 'Same project name', origin_path_sha256: origin },
  tracks: [], assets: {},
}) as unknown as Project
const projectA = project('origin-A')
const projectB = project('origin-B')

function Fixture() {
  const [current, setCurrent] = useState(projectA)
  const [showRecord, setShowRecord] = useState(true)
  const session = useRecordingSession({ project: current, onEnsureProject: async () => current, onResult: () => {} })
  return <RecordingSessionProvider session={session}>
    <RecordingDeliveryProvider project={current}>
      <VolumeAutomationProvider>
        <button data-test-project-b onClick={() => setCurrent(projectB)}>Open B</button>
        <button data-test-edit onClick={() => setShowRecord(false)}>Edit</button>
        <output data-test-origin>{current.project_identity?.origin_path_sha256}</output>
        {showRecord ? <Record project={current} /> : <div data-test-away-surface>Edit</div>}
      </VolumeAutomationProvider>
    </RecordingDeliveryProvider>
  </RecordingSessionProvider>
}

createRoot(document.getElementById('root')!).render(<Fixture />)
