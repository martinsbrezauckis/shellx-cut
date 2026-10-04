import { useState } from 'react'
import { createRoot } from 'react-dom/client'
import TopBar from '../../src/topbar'
import { useRenderQueueOwner } from '../../src/topbar/useRenderQueueOwner'
import type { Project } from '../../src/lib/client'
import '../../src/theme.css'

function project(name: string, digest: string): Project {
  return {
    schema: 'shellx-cut/project/1', name, project_revision: 'op_1',
    project_identity: { schema: 'shellx-cut/project-identity/1', origin_path_sha256: digest, project_name: name },
    settings: { width: 1920, height: 1080, fps: 30 }, assets: {}, tracks: [], markers: [], caption_styles: {}, checkpoints: [],
  } as Project
}
const projectA = project('Queue A', `sha256:${'a'.repeat(64)}`)
const renamedA = project('Renamed A', `sha256:${'a'.repeat(64)}`)
const projectB = project('Queue B', `sha256:${'b'.repeat(64)}`)

function Fixture() {
  const [active, setActive] = useState(projectA)
  const [session, setSession] = useState(0)
  const [mode, setMode] = useState<'edit' | 'record'>('edit')
  const owner = useRenderQueueOwner(active, session)
  Object.assign(window, {
    queueFixtureProject: (target: 'a' | 'b' | 'rename') => {
      if (target === 'rename') setActive(renamedA)
      else { setActive(target === 'a' ? projectA : projectB); setSession(n => n + 1) }
    },
  })
  return <div className="app" data-cut-app-root>
    <button data-test-edit onClick={() => setMode('edit')}>Edit</button>
    <button data-test-record onClick={() => setMode('record')}>Record</button>
    <button data-test-a onClick={() => { setActive(projectA); setSession(n => n + 1) }}>A</button>
    <button data-test-a-rename onClick={() => setActive(renamedA)}>Rename A</button>
    <button data-test-b onClick={() => { setActive(projectB); setSession(n => n + 1) }}>B</button>
    <output data-test-qid>{owner.state.admitted?.id ?? ''}</output>
    <output data-test-phase>{owner.state.phase}</output>
    <output data-test-progress>{owner.state.progress}</output>
    <output data-test-result-rows>{owner.state.result?.jobs?.length ?? 0}</output>
    <output data-test-error>{owner.state.error ?? ''}</output>
    <output data-test-row-output>{owner.state.rows[0]?.output ?? ''}</output>
    {mode === 'edit' ? <TopBar key={`${active.project_identity?.origin_path_sha256}:${session}`} project={active} renderQueueOwner={owner} />
      : <div data-test-record-workspace>Record workspace</div>}
  </div>
}

createRoot(document.getElementById('root')!).render(<Fixture />)
