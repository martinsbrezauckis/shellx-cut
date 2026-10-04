import { useState } from 'react'
import { createRoot } from 'react-dom/client'
import MatteDrawer from '../../src/panels/Matte'
import type { Project } from '../../src/lib/client'

const project = (name: string, origin: string) => ({
  name,
  project_identity: { project_name: name, origin_path_sha256: origin },
  tracks: [{ kind: 'video', clips: [{ id: 'clip-1', asset: 'asset-1' }] }],
  assets: { 'asset-1': {} },
}) as unknown as Project

function Fixture() {
  const [current, setCurrent] = useState(project('Project A', 'origin-a'))
  const [open, setOpen] = useState(true)
  return <>
    <button data-test-project-a onClick={() => setCurrent(project('Project A', 'origin-a'))}>A</button>
    <button data-test-project-b onClick={() => setCurrent(project('Project B', 'origin-b'))}>B</button>
    <button data-test-rename onClick={() => setCurrent((value) => project(`Renamed ${value.name}`, value.project_identity.origin_path_sha256))}>Rename</button>
    <button data-test-unmount onClick={() => setOpen(false)}>Unmount</button>
    <button data-test-remount onClick={() => setOpen(true)}>Remount</button>
    <output data-test-origin>{current.project_identity?.origin_path_sha256}</output>
    {open && <MatteDrawer project={current} clipId="clip-1" playheadMs={0} onClose={() => setOpen(false)} />}
  </>
}

createRoot(document.getElementById('root')!).render(<Fixture />)
