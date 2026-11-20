import { useState } from 'react'
import { createRoot } from 'react-dom/client'
import Transcript from '../../src/panels/Transcript'
import type { Project } from '../../src/lib/client'

const makeProject = (origin: string) => ({
  schema: 'shellx-cut/project/1', name: 'Same project name',
  project_identity: { schema: 'shellx-cut/project-identity/1', origin_path_sha256: origin, project_name: 'Same project name' },
  settings: { width: 1920, height: 1080, fps: 30 },
  assets: { shared: { path: '/fixture/shared.mp4', hash: 'shared', transcript: '/fixture/words.json' } },
  tracks: [], markers: [], caption_styles: {}, checkpoints: [],
}) as Project
const projectA = makeProject('origin-A')
const projectB = makeProject('origin-B')
const makeMultiProject = (locator: string) => ({
  ...makeProject('origin-M'),
  assets: {
    shared: { path: '/fixture/shared.mp4', hash: 'shared', transcript: '/fixture/words.json' },
    second: { path: '/fixture/second.mp4', hash: 'second', transcript: locator },
  },
}) as Project
const emptyOps = []
const emptyTranscripts = {}

function Fixture() {
  const [project, setProject] = useState(projectA)
  const [open, setOpen] = useState(true)
  return <>
    <button data-test-a onClick={() => setProject(projectA)}>Project A</button>
    <button data-test-b onClick={() => setProject(projectB)}>Project B</button>
    <button data-test-multi onClick={() => setProject(makeMultiProject('/fixture/second-v1.json'))}>Two assets</button>
    <button data-test-locator-v2 onClick={() => setProject(makeMultiProject('/fixture/second-v2.json'))}>Second locator v2</button>
    <button data-test-locator-v3 onClick={() => setProject(makeMultiProject('/fixture/second-v3.json'))}>Second locator v3</button>
    <button data-test-unmount onClick={() => setOpen(false)}>Unmount</button>
    <output data-test-origin>{project.project_identity?.origin_path_sha256}</output>
    {open && <Transcript project={project} ops={emptyOps} playheadMs={0} selectedClipId="clip-1"
      transcripts={emptyTranscripts} onCutWords={() => {}} onRestore={() => {}} onSeek={() => {}} />}
  </>
}

createRoot(document.getElementById('root')!).render(<Fixture />)
