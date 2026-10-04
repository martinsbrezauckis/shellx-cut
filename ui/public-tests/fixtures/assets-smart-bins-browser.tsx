import { useState } from 'react'
import { createRoot } from 'react-dom/client'
import Assets from '../../src/panels/Assets'
import type { Project } from '../../src/lib/client'
import '../../src/theme.css'

function makeProject(name: string, digest: string, revision: string): Project {
  return {
    schema: 'shellx-cut/project/1', name, project_revision: revision,
    project_identity: { schema: 'shellx-cut/project-identity/1', origin_path_sha256: digest, project_name: name },
    settings: { width: 1920, height: 1080, fps: 30 }, assets: {}, tracks: [], markers: [], caption_styles: {}, checkpoints: [],
  } as Project
}

function Fixture() {
  const [scope, setScope] = useState(1)
  const [revision, setRevision] = useState(1)
  const [name, setName] = useState('A')
  const project = makeProject(name, name === 'A' ? 'origin-a' : 'origin-b', `op_${revision}`)
  Object.assign(window, {
    smartBinFixtureProject: (next: 'A' | 'B') => { setName(next); setScope(value => value + 1) },
    smartBinFixtureRevision: () => setRevision(value => value + 1),
  })
  return <div className="app" data-cut-app-root>
    <output data-test-scope>{scope}</output>
    <button data-test-a onClick={() => { setName('A'); setScope(value => value + 1) }}>A</button>
    <button data-test-b onClick={() => { setName('B'); setScope(value => value + 1) }}>B</button>
    <button data-test-revision onClick={() => setRevision(value => value + 1)}>Revision</button>
    <Assets project={project} projectScope={scope} doctor={null} playheadMs={0} />
  </div>
}

createRoot(document.getElementById('root')!).render(<Fixture />)
