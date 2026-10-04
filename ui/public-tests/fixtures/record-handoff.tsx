import { useEffect, useState } from 'react'
import { createRoot } from 'react-dom/client'
import Record from '../../src/panels/Record'
import { RecordingSessionProvider } from '../../src/app/RecordingSessionContext'
import { RecordingDeliveryProvider, useAppRecordingDelivery } from '../../src/app/RecordingDeliveryContext'
import { VolumeAutomationProvider } from '../../src/app/VolumeAutomationContext'
import { useRecordingSession } from '../../src/app/useRecordingSession'
import type { Project } from '../../src/lib/client'
const PROJECT_A = `sha256:${'a'.repeat(64)}`
const PROJECT_B = `sha256:${'b'.repeat(64)}`

const projectA = {
  name: 'Record handoff fixture',
  project_identity: { project_name: 'Record handoff fixture', origin_path_sha256: PROJECT_A },
  tracks: [], assets: {},
} as unknown as Project
const projectB = {
  name: 'Other project',
  project_identity: { project_name: 'Other project', origin_path_sha256: PROJECT_B },
  tracks: [], assets: {},
} as unknown as Project
const projectARenamed = {
  name: 'Renamed record project',
  project_identity: { project_name: 'Renamed record project', origin_path_sha256: PROJECT_A },
  tracks: [], assets: {},
} as unknown as Project

function DeliveryProbe() {
  const delivery = useAppRecordingDelivery()
  return <div data-test-delivery>
    <output data-test-export-job>{delivery.exportJob?.id ?? ''}</output>
    <output data-test-copy-job>{delivery.rawCopy.jobId ?? ''}</output>
    <output data-test-export-note>{delivery.exportNote}</output>
    <output data-test-copy-note>{delivery.rawCopy.note}</output>
    <output data-test-owns-result>{String(delivery.ownsResult)}</output>
  </div>
}

function Fixture() {
  const [mode, setMode] = useState<'record' | 'edit' | 'library'>('record')
  const [project, setProject] = useState(projectA)
  const [deliveryMounted, setDeliveryMounted] = useState(true)
  const session = useRecordingSession({ project, onEnsureProject: async () => project, onResult: () => {} })
  useEffect(() => session.setCountdownSeconds(0), [session.setCountdownSeconds])
  return <RecordingSessionProvider session={session}>
    <button data-test-unmount-delivery onClick={() => setDeliveryMounted(false)}>Unmount delivery owner</button>
    {deliveryMounted && <RecordingDeliveryProvider project={project}>
    <VolumeAutomationProvider key={project.project_identity.origin_path_sha256}>
    <button data-test-edit onClick={() => setMode('edit')}>Edit</button>
    <button data-test-library onClick={() => setMode('library')}>Library</button>
    <button data-test-record onClick={() => setMode('record')}>Record</button>
    <button data-test-project-a onClick={() => setProject(projectA)}>Project A</button>
    <button data-test-project-a-renamed onClick={() => setProject(projectARenamed)}>Rename A</button>
    <button data-test-project-b onClick={() => setProject(projectB)}>Project B</button>
    <button data-test-stop onClick={() => void session.stop()}>Stop</button>
    <button data-test-reset onClick={() => session.reset()}>New take</button>
    {mode === 'record' ? <Record project={project} /> : <div data-test-away-surface>{mode} workspace</div>}
    <output data-test-phase>{session.state.phase}</output>
    <output data-test-result-project>{session.state.resultProjectIdentity?.origin_path_sha256 ?? ''}</output>
    <DeliveryProbe />
    </VolumeAutomationProvider>
    </RecordingDeliveryProvider>}
  </RecordingSessionProvider>
}

createRoot(document.getElementById('root')!).render(<Fixture />)
