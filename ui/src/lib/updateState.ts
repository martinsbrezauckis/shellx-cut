// lib/updateState.ts — pure model for the desktop shell's update snapshot.
//
// Role: the types, validation, and display logic for the update surface (the
// topbar "Update to vX" button + Settings > About controls). Kept pure (no
// DOM, no bridge calls) so ui/public-tests/lib.test.ts can red-proof the
// state machine: available → button shows; none/idle → hidden; error → honest
// message; unsupported (Linux deb/rpm) → explanation instead of dead buttons.
//
// The snapshot originates in the shell (app/desktop/src-tauri/src/
// update_state.rs — one serializer for command replies AND the
// `cut:update-state` event) and crosses the narrow bridge in lib/tauri.ts.
// Both sides carry the same schema tag so drift fails validation, not the UI.
//
// Callers: topbar/UpdateButton, panels/Environment/About, lib/tauri.

/** Schema tag stamped on every snapshot by the shell serializer. */
export const SHELL_UPDATE_STATE_SCHEMA = 'shellx-cut/update-state/1'

/** Lifecycle status — mirrors update_state.rs `Status::as_str`. */
export type ShellUpdateStatus = 'idle' | 'none' | 'available' | 'error' | 'unsupported'

/** The shell's honest update snapshot (see update_state.rs for field docs). */
export interface ShellUpdateState {
  schema: typeof SHELL_UPDATE_STATE_SCHEMA
  status: ShellUpdateStatus
  /** Newer release version — present exactly while status === 'available'. */
  version?: string | null
  /** Installed app version. */
  current: string
  /** Unix ms of the last completed check (success or failure). */
  checked_at?: number | null
  /** Honest failure text of the most recent failed check/install attempt. */
  error?: string | null
  /** A release-feed check is in flight right now. */
  checking: boolean
  /** An install is in flight right now. */
  installing: boolean
  /** False where packages update outside the app (Linux deb/rpm). */
  supported: boolean
}

const STATUSES: readonly ShellUpdateStatus[] = ['idle', 'none', 'available', 'error', 'unsupported']

/** Strict payload validation — a malformed bridge/event payload becomes null
 *  (callers keep their previous state) instead of corrupt UI state. */
export function validShellUpdateState(value: unknown): value is ShellUpdateState {
  if (!value || typeof value !== 'object') return false
  const c = value as Partial<ShellUpdateState>
  return c.schema === SHELL_UPDATE_STATE_SCHEMA
    && typeof c.status === 'string'
    && (STATUSES as readonly string[]).includes(c.status)
    && typeof c.current === 'string'
    && typeof c.checking === 'boolean'
    && typeof c.installing === 'boolean'
    && typeof c.supported === 'boolean'
    && (c.version == null || typeof c.version === 'string')
    && (c.checked_at == null || typeof c.checked_at === 'number')
    && (c.error == null || typeof c.error === 'string')
}

/** Topbar rule: exactly one quiet button, only while an update is offered. */
export function shouldShowUpdateButton(state: ShellUpdateState | null): boolean {
  return !!state && state.status === 'available' && typeof state.version === 'string' && state.version.length > 0
}

/** Topbar button label — version-forward, quiet. */
export function updateButtonLabel(state: ShellUpdateState): string {
  if (state.installing) return 'Installing update…'
  return `Update to v${state.version ?? ''}`
}

/** Presentation tone for the About status line (styling hook, not copy). */
export type UpdateStatusTone = 'muted' | 'ok' | 'action' | 'error'

/**
 * One honest status sentence per state for Settings > About. Failure states
 * name what failed (the shell forwards the underlying error text) per the
 * repo's honest-degradation contract; Linux explains its packaging instead of
 * showing dead controls.
 */
export function describeUpdateStatus(state: ShellUpdateState | null): { tone: UpdateStatusTone; text: string } {
  if (!state) return { tone: 'muted', text: 'Reading update status…' }
  if (state.installing) return { tone: 'action', text: `Installing ShellX Cut ${state.version ?? ''}…`.replace('  ', ' ') }
  if (state.checking) return { tone: 'action', text: 'Checking for updates…' }
  switch (state.status) {
    case 'available':
      return { tone: 'action', text: `ShellX Cut ${state.version} is available.` }
    case 'none':
      return { tone: 'ok', text: "You're on the latest version." }
    case 'error':
      return { tone: 'error', text: `Update check failed: ${state.error ?? 'unknown error'}` }
    case 'unsupported':
      return {
        tone: 'muted',
        text: 'Linux builds update through deb/rpm package downloads — the in-app updater is not used.',
      }
    case 'idle':
    default:
      return { tone: 'muted', text: 'No update check has run yet in this session.' }
  }
}

/**
 * "Checked N ago" for the About panel. Returns null when no check has
 * completed (idle sessions must not show a fake timestamp). Deterministic
 * thresholds so the lib test can pin every band.
 */
export function formatCheckedAgo(checkedAt: number | null | undefined, nowMs: number): string | null {
  if (typeof checkedAt !== 'number' || checkedAt <= 0) return null
  const elapsed = Math.max(0, nowMs - checkedAt)
  const minute = 60_000
  const hour = 60 * minute
  const day = 24 * hour
  if (elapsed < 45_000) return 'Checked just now'
  if (elapsed < 90_000) return 'Checked a minute ago'
  if (elapsed < hour) return `Checked ${Math.round(elapsed / minute)} minutes ago`
  if (elapsed < 2 * hour) return 'Checked an hour ago'
  if (elapsed < day) return `Checked ${Math.round(elapsed / hour)} hours ago`
  const days = Math.round(elapsed / day)
  return `Checked ${days} ${days === 1 ? 'day' : 'days'} ago`
}

/**
 * Release-notes destination: the exact offered release when one is known,
 * otherwise the latest-release page. Plain https link — opens in the OS
 * browser from the desktop shell and a new tab in a browser build.
 */
export function releaseNotesUrl(state: ShellUpdateState | null): string {
  const base = 'https://github.com/martinsbrezauckis/shellx-cut/releases'
  if (state && state.status === 'available' && state.version) {
    return `${base}/tag/v${encodeURIComponent(state.version)}`
  }
  return `${base}/latest`
}
