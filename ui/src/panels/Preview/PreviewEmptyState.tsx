interface PreviewEmptyStateProps {
  hasProject: boolean
  onImport: () => void
}

export default function PreviewEmptyState({ hasProject, onImport }: PreviewEmptyStateProps) {
  if (!hasProject) {
    return <div className="pv-empty">Create a project in Projects to begin</div>
  }
  return (
    <button
      type="button"
      className="pv-empty pv-empty--cta"
      data-cut-import-cta
      onClick={onImport}
    >
      ⬑ Import media to begin
    </button>
  )
}
