import type { Project } from '../../lib/client'
import { Icon } from '../../icons'
import LibraryPanel from './index'

export interface LibraryWorkspaceProps {
  project: Project | null
  playheadMs: number
  onAddedToProject: () => void
  onClose: () => void
}

export default function LibraryWorkspace({
  project,
  playheadMs,
  onAddedToProject,
  onClose,
}: LibraryWorkspaceProps) {
  return (
    <section className="library-workspace" data-cut-library-workspace>
      <header className="library-workspace__header">
        <div>
          <p className="library-workspace__eyebrow">Across every project</p>
          <h1>Media Library</h1>
          <p>Organize reusable media here. Media already attached to this edit remains in Project Assets.</p>
        </div>
        <button type="button" className="library-workspace__close" data-cut-library-close onClick={onClose}>
          <Icon name="collapseLeft" size={14} />
          Back to Edit
        </button>
      </header>
      <div className="library-workspace__body">
        <LibraryPanel
          project={project}
          playheadMs={playheadMs}
          onAddedToProject={onAddedToProject}
          active
        />
      </div>
    </section>
  )
}
