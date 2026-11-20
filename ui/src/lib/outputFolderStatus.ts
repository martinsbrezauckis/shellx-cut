// The export destination as the engine reports it (`project.get_output_dir`
// and the `output_folder_changed` event). The engine holds the ONE truth; the
// UI only mirrors it. Pure module: no imports, safe for tests and for events.ts.

export interface OutputFolderStatus {
  /** The chosen export folder, or null when exports go to each project's exports folder. */
  dir: string | null
  destination: 'chosen' | 'project_exports'
  /** False when the chosen folder is gone or cannot be written. */
  usable: boolean
  /** Plain reason when `usable` is false. */
  problem: string | null
  /** The way forward when `usable` is false. */
  suggested_action: string | null
}

function text(value: unknown): string | null {
  return typeof value === 'string' && value.trim() ? value : null
}

export function outputFolderStatusFrom(value: unknown): OutputFolderStatus | null {
  if (!value || typeof value !== 'object') return null
  const raw = value as Record<string, unknown>
  if (raw.destination !== 'chosen' && raw.destination !== 'project_exports') return null
  if (typeof raw.usable !== 'boolean') return null
  const dir = text(raw.dir)
  if (raw.destination === 'chosen' && !dir) return null
  return {
    dir: raw.destination === 'chosen' ? dir : null,
    destination: raw.destination,
    usable: raw.usable,
    problem: text(raw.problem),
    suggested_action: text(raw.suggested_action),
  }
}

/** What the status bar and the Export menu say about the destination. */
export function outputFolderLabel(status: OutputFolderStatus | null, tail: (dir: string) => string): string {
  if (!status || !status.dir) return 'project exports'
  return status.usable ? tail(status.dir) : `${tail(status.dir)} (unavailable)`
}

/** Hover text: where exports go, and for an unusable folder why and what to do. */
export function outputFolderTitle(status: OutputFolderStatus | null): string {
  if (!status || !status.dir) return 'Default export folder: each project /exports folder'
  if (status.usable) return `Default export folder: ${status.dir}`
  return `Default export folder: ${status.dir}. ${status.problem ?? 'It cannot be used.'}${status.suggested_action ? ` ${status.suggested_action}` : ''}`
}

/** What `ui.state` says about the destination: the name of the folder, never its path. */
export interface ExportFolderState {
  destination: OutputFolderStatus['destination']
  usable: boolean
  folder: string | null
}

export function exportFolderState(status: OutputFolderStatus | null | undefined): ExportFolderState | null {
  if (!status) return null
  const name = status.dir ? status.dir.replace(/[\\/]+$/, '').split(/[\\/]/).pop() || null : null
  return { destination: status.destination, usable: status.usable, folder: name }
}
