import { createRoot } from 'react-dom/client'
import TopBar from '../../src/topbar'
import type { Project } from '../../src/lib/client'
import '../../src/theme.css'

const project = {
  schema: 'shellx-cut/project/1',
  name: 'queue-preflight-fixture',
  project_revision: 'op_1',
  project_identity: { schema: 'shellx-cut/project-identity/1', origin_path_sha256: 'fixture', project_name: 'queue-preflight-fixture' },
  settings: { width: 1920, height: 1080, fps: 30 },
  assets: {}, tracks: [], markers: [], caption_styles: {}, checkpoints: [],
} as Project

createRoot(document.getElementById('root')!).render(
  <div className="app" data-cut-app-root><TopBar project={project} /></div>,
)
