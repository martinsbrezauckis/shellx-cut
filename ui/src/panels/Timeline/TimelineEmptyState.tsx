import { Availability } from '../../lib/availability'

interface TimelineEmptyStateProps {
  hasProject: boolean
  hasTracks: boolean
  onImport: () => void
}

export default function TimelineEmptyState({ hasProject, hasTracks, onImport }: TimelineEmptyStateProps) {
  if (!hasProject) {
    return <div className="tl-empty">Create a project in Projects to begin</div>
  }
  if (hasTracks) return null

  return (
    <Availability rule="shell.desktop" replacement={<div className="tl-empty" data-cut-import-desktop-note>Importing files needs the desktop app.</div>}>
    <div className="tl-empty">
      <button
        type="button"
        className="tl-empty--cta"
        data-cut-import-cta
        data-cut-action="import-cta"
        onClick={onImport}
      >
        ⬑ Import media to begin
      </button>
    </div>
    </Availability>
  )
}
