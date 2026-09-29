import { useEffect, useState } from 'react'
import { createRoot } from 'react-dom/client'
import Record from '../../src/panels/Record'
import { RecordingSessionProvider } from '../../src/app/RecordingSessionContext'
import { useRecordingSession } from '../../src/app/useRecordingSession'
import type { Project } from '../../src/lib/client'

const project = {
  name: 'Record handoff fixture',
  project_identity: { project_name: 'Record handoff fixture', origin_path_sha256: 'fixture-project' },
  tracks: [], assets: {},
} as unknown as Project

function Fixture() {
  const [mode, setMode] = useState<'record' | 'edit'>('record')
  const session = useRecordingSession({ project, onEnsureProject: async () => project, onResult: () => {} })
  useEffect(() => session.setCountdownSeconds(0), [session.setCountdownSeconds])
  return <RecordingSessionProvider session={session}>
    <button data-test-edit onClick={() => setMode('edit')}>Edit</button>
    <button data-test-record onClick={() => setMode('record')}>Record</button>
    {mode === 'record' ? <Record project={project} /> : <div data-test-edit-surface>Edit workspace</div>}
    <output data-test-phase>{session.state.phase}</output>
  </RecordingSessionProvider>
}

createRoot(document.getElementById('root')!).render(<Fixture />)
