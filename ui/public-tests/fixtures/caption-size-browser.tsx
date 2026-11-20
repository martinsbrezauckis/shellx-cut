import { createRoot } from 'react-dom/client'
import ProjectCaptionsSection from '../../src/panels/Inspector/ProjectCaptionsSection'
import type { Project } from '../../src/lib/client'
import '../../src/panels/Inspector/inspector.css'

const project = { name: 'caption-size-test', tracks: [], assets: {} } as Project
createRoot(document.getElementById('root')!).render(
  <ProjectCaptionsSection project={project} playheadMs={0} onSeek={() => {}} />,
)
