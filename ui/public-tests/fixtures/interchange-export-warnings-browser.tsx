import { useState } from 'react'
import { createRoot } from 'react-dom/client'
import TopBar from '../../src/topbar'
import { useRenderQueueOwner } from '../../src/topbar/useRenderQueueOwner'
import type { Project } from '../../src/lib/client'
import '../../src/theme.css'

const project = {
  schema: 'shellx-cut/project/1',
  name: 'interchange-warning-fixture',
  project_revision: 'op_1',
  project_identity: { schema: 'shellx-cut/project-identity/1', origin_path_sha256: 'fixture', project_name: 'interchange-warning-fixture' },
  settings: { width: 1920, height: 1080, fps: 30 },
  assets: {}, tracks: [], markers: [], caption_styles: {}, checkpoints: [],
} as Project

function Fixture() {
  const [activeProject, setActiveProject] = useState(project)
  const renderQueueOwner = useRenderQueueOwner(activeProject, 0)
  Object.assign(window, {
    fixtureProjectName: activeProject.name,
    reviseFixtureProject: () => setActiveProject((current) => ({ ...current, project_revision: 'op_2' })),
    switchFixtureProject: () => setActiveProject({
      ...project,
      name: 'other-project',
      project_identity: { ...project.project_identity!, origin_path_sha256: 'other-origin', project_name: 'other-project' },
    }),
  })
  return <div className="app" data-cut-app-root><TopBar project={activeProject} renderQueueOwner={renderQueueOwner} /></div>
}

const root = createRoot(document.getElementById('root')!)
Object.assign(window, { unmountFixture: () => root.unmount() })
root.render(<Fixture />)
