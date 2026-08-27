import type { CutError } from '../../lib/clientModel'
import type { JobRecord } from '../../lib/jobModel'

/** Mirrors the server's cross-platform portable package-name rule. */
export const PORTABLE_NAME_HINT = '1–80 letters, numbers, spaces, hyphens, or underscores'

export type PortableCopyPhase = 'form' | 'previewing' | 'ready' | 'confirming' | 'creating' | 'complete'

export function portablePackageNameError(name: string): string | null {
  if (!name) return 'Enter a copy name.'
  if (name.length > 80) return 'Keep the copy name to 80 characters or fewer.'
  if (name.endsWith('.')) return 'A copy name cannot end with a dot.'
  if (![...name].every((char) => /[A-Za-z0-9 _-]/.test(char))) {
    return 'Use letters, numbers, spaces, hyphens, or underscores only.'
  }
  return null
}

export function portableNameForProject(projectName: string): string {
  const normalized = projectName
    .replace(/[^A-Za-z0-9 _-]/g, ' ')
    .replace(/\s+/g, ' ')
    .trim()
    .slice(0, 80)
    .replace(/[. ]+$/, '')
  return normalized || 'Project copy'
}

export function formatPortableBytes(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes < 0) return 'Size unavailable'
  if (bytes < 1_024) return `${bytes.toLocaleString()} B`
  const units = ['KB', 'MB', 'GB', 'TB']
  let value = bytes / 1_024
  let unit = 0
  while (value >= 1_024 && unit < units.length - 1) {
    value /= 1_024
    unit += 1
  }
  return `${value >= 10 ? value.toFixed(0) : value.toFixed(1)} ${units[unit]}`
}

export function portableErrorMessage(error: CutError | undefined, fallback: string): string {
  const message = error?.message?.trim() || fallback
  const action = error?.suggested_action?.trim()
  return action ? `${message} · ${action}` : message
}

/** A preview is destination/name-bound, so only its editable review states can retain it. */
export function portablePreviewNeedsInvalidation(phase: PortableCopyPhase): boolean {
  return phase === 'ready' || phase === 'confirming'
}

/** Cancellation is a distinct job outcome, never a generic failed copy. */
export function portablePackageCancellationMessage(job: Pick<JobRecord, 'outcome' | 'outcome_reason'>): string | null {
  if (job.outcome !== 'cancelled'
    && job.outcome_reason !== 'user_cancelled'
    && job.outcome_reason !== 'project_switch_cancelled') return null
  return 'Copy cancelled before publication. No portable copy was published.'
}
