import { useState } from 'react'
import { createRoot } from 'react-dom/client'
import { useRecordingExport } from '../../src/panels/Record/useRecordingExport'

function Fixture() {
  const [project, setProject] = useState('project-a')
  const [capture, setCapture] = useState('capture-1')
  const [note, setNote] = useState('')
  const [mounted, setMounted] = useState(true)
  Object.assign(window, {
    changeExportOwner: () => { setProject('project-b'); setCapture('capture-2') },
    returnExportOwner: () => { setProject('project-a'); setCapture('capture-1') },
    unmountExportHook: () => setMounted(false),
  })
  return mounted ? <ExportFixture project={project} owner={`${project}/${capture}`} note={note} setNote={setNote} /> : <div data-test-unmounted />
}

function ExportFixture({ project, owner, note, setNote }: { project: string; owner: string; note: string; setNote: (note: string) => void }) {
  const { exportJob, exportRunning, exportCancelable, exportClip, cancelExport } = useRecordingExport({
    capture: { source: '/fixture/capture/source.mp4', plan: '/fixture/capture/plan.json' },
    projectKey: project,
    ownerKey: owner,
    format: 'mp4',
    outputPath: null,
    setNote,
  })
  return <div data-test-owner={owner}>
    <button data-test-export disabled={exportRunning} onClick={() => void exportClip()}>Export</button>
    {exportCancelable && <button data-test-cancel onClick={() => void cancelExport()}>Cancel</button>}
    <span data-test-job={exportJob?.id ?? ''} />
    <p data-test-note>{note}</p>
  </div>
}

createRoot(document.getElementById('root')!).render(<Fixture />)
