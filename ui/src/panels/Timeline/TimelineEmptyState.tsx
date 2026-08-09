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
    <button
      type="button"
      className="tl-empty tl-empty--cta"
      data-cut-import-cta
      onClick={onImport}
    >
      ⬑ Import media to begin
    </button>
  )
}
