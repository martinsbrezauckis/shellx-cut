import type { ReactNode } from 'react'
import ContextMenuFrame from '../../components/ContextMenuFrame'
import { Icon } from '../../icons'

export interface ProjectContextMenuState {
  x: number
  y: number
  projectId: string
}

interface ProjectTarget {
  id: string
  name: string
  missing?: boolean
  current: boolean
}

interface ProjectContextMenuProps {
  menu: ProjectContextMenuState
  project: ProjectTarget | null
  busy: boolean
  onReopen: (projectId: string) => void
  onForget: (projectId: string) => void
  onDelete: (projectId: string) => void
  onClose: () => void
}

function Item({ action, disabled = false, title, danger = false, children, onClick }: {
  action: string
  disabled?: boolean
  title: string
  danger?: boolean
  children: ReactNode
  onClick: () => void
}) {
  return <button className={`tl-ctx__item${danger ? ' tl-ctx__item--danger' : ''}`} data-cut-project-ctx={action} role="menuitem" disabled={disabled} title={title} onClick={onClick}>{children}</button>
}

/** The row id is retained through every callback: a context menu never acts on
 * the current selection or a similarly named recent project. */
export default function ProjectContextMenu({ menu, project, busy, onReopen, onForget, onDelete, onClose }: ProjectContextMenuProps) {
  if (!project) return null
  const reopenDisabled = busy || !!project.missing || project.current
  const deleteDisabled = busy || project.current
  const reopenReason = project.current ? 'This project is already open' : project.missing ? 'This project folder is missing; forget it or restore the folder first' : `Open ${project.name}`
  const deleteReason = project.current ? 'Open a different project before deleting this one' : `Permanently delete ${project.name} after confirmation; original media files stay untouched`
  return <ContextMenuFrame x={menu.x} y={menu.y} menuId="data-cut-project-menu" backdropId="data-cut-project-ctx-backdrop" onClose={onClose}>
    <span className="tl-ctx__label" aria-hidden="true">Project · {project.name}</span>
    <Item action="project-reopen" disabled={reopenDisabled} title={reopenReason} onClick={() => { onReopen(project.id); onClose() }}><Icon name="projectOpen" size={14} /> Reopen</Item>
    <Item action="project-forget" disabled={busy} title="Remove this exact entry from Recent Projects; files stay on disk" onClick={() => { onForget(project.id); onClose() }}><Icon name="close" size={14} /> Forget from list</Item>
    <span className="tl-ctx__sep" aria-hidden="true" />
    <Item action="project-delete" danger disabled={deleteDisabled} title={deleteReason} onClick={() => { onDelete(project.id); onClose() }}><Icon name="trash" size={14} /> Delete project…</Item>
  </ContextMenuFrame>
}
