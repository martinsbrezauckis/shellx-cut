import { useState } from 'react'
import { createRoot } from 'react-dom/client'
import Review from '../../src/panels/Review'
import type { Project } from '../../src/lib/client'

const makeProject = (origin: string) => ({
  schema: 'shellx-cut/project/1', name: 'Same project name',
  project_identity: { schema: 'shellx-cut/project-identity/1', origin_path_sha256: origin, project_name: 'Same project name' },
  settings: { width: 1920, height: 1080, fps: 30 }, assets: {}, tracks: [], markers: [], caption_styles: {}, checkpoints: [],
}) as Project

function Fixture() {
  const [project, setProject] = useState(makeProject('origin-A'))
  const [open, setOpen] = useState(true)
  return <>
    <button data-test-switch onClick={() => setProject(makeProject('origin-B'))}>Switch origin</button>
    <button data-test-unmount onClick={() => setOpen(false)}>Unmount review</button>
    <output data-test-origin>{project.project_identity?.origin_path_sha256}</output>
    {open && <Review project={project} playheadMs={0} ops={[]} receipts={[]} onUndo={() => {}} onRedo={() => {}} />}
  </>
}

createRoot(document.getElementById('root')!).render(<Fixture />)
