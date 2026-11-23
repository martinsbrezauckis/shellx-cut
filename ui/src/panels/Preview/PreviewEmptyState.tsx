import { Availability } from '../../lib/availability'

interface PreviewEmptyStateProps {
  hasProject: boolean
  onImport: () => void
}

export default function PreviewEmptyState({ hasProject, onImport }: PreviewEmptyStateProps) {
  if (!hasProject) {
    return <div className="pv-empty">Create a project in Projects to begin</div>
  }
  return (
    <Availability rule="shell.desktop" replacement={<div className="pv-empty" data-cut-import-desktop-note>Importing files needs the desktop app.</div>}>
    <button
      type="button"
      className="pv-empty pv-empty--cta"
      data-cut-import-cta
      data-cut-action="import-cta"
      onClick={onImport}
    >
      ⬑ Import media to begin
    </button>
    </Availability>
  )
}
