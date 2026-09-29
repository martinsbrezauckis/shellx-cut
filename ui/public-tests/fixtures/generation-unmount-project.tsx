import { useState } from 'react'
import { createRoot } from 'react-dom/client'
import type { Project } from '../../src/lib/client'
import GenerateDrawer from '../../src/panels/Generate'

const project = (name: string, origin: string) => ({
  name,
  project_identity: { schema: 'shellx-cut/project-identity/1', origin_path_sha256: origin, project_name: name },
  assets: {},
  tracks: [{ id: 'v1', kind: 'video', clips: [], locked: false }],
}) as Project
const a = project('Project A', 'origin-A')
const b = project('Project B', 'origin-B')

function Fixture() {
  const [open, setOpen] = useState(true)
  const [current, setCurrent] = useState(a)
  const [scope, setScope] = useState(1)
  const [generated, setGenerated] = useState(0)
  return <>
    <button data-test-unmount onClick={() => setOpen(false)}>Templates</button>
    <button data-test-open-b onClick={() => { setCurrent(b); setScope(2); setOpen(true) }}>Open B Media</button>
    <button data-test-return-a onClick={() => { setCurrent(a); setScope(3); setOpen(true) }}>Return A Media</button>
    <output data-test-project>{current.name}</output>
    <output data-test-generated-count>{generated}</output>
    {open && <GenerateDrawer project={current} projectScope={scope} onGenerated={() => setGenerated((count) => count + 1)} />}
  </>
}

createRoot(document.getElementById('root')!).render(<Fixture />)
