import { useState } from 'react'
import { createRoot } from 'react-dom/client'
import { useRawRecordingCopy } from '../../src/panels/Record/useRawRecordingCopy'

Object.assign(window, { __TAURI__: { core: { invoke: async () => null } } })
const PROJECT_A = `sha256:${'a'.repeat(64)}`
const PROJECT_B = `sha256:${'b'.repeat(64)}`

function Fixture() {
  const [project, setProject] = useState(PROJECT_A)
  const [raw, setRaw] = useState('/fixture/raw-a.mp4')
  const [mounted, setMounted] = useState(true)
  Object.assign(window, {
    changeCopyProject: () => { setProject(PROJECT_B); setRaw('/fixture/raw-b.mp4') },
    returnCopyProject: () => { setProject(PROJECT_A); setRaw('/fixture/raw-a.mp4') },
    changeCopyCapture: () => setRaw('/fixture/another-raw-a.mp4'),
    unmountCopyHook: () => setMounted(false),
  })
  return mounted ? <CopyFixture project={project} raw={raw} /> : <div data-test-unmounted />
}

function CopyFixture({ project, raw }: { project: string; raw: string }) {
  const copy = useRawRecordingCopy(raw, project)
  return <div data-test-owner={`${project}:${raw}`}>
    <button data-test-copy disabled={copy.running} onClick={() => void copy.saveCopy()}>Save copy</button>
    {copy.cancelable && <button data-test-cancel onClick={() => void copy.cancelCopy()}>Cancel</button>}
    <span data-test-job={copy.jobId ?? ''} />
    <p data-test-note>{copy.note}</p>
  </div>
}

createRoot(document.getElementById('root')!).render(<Fixture />)
